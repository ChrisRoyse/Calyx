use super::*;

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Opens a durable vault with an injected clock.
    pub fn open_with_clock(
        vault_dir: impl AsRef<Path>,
        vault_id: VaultId,
        vault_salt: impl Into<Vec<u8>>,
        options: VaultOptions,
        clock: C,
    ) -> Result<Self> {
        DurableVault::validate_options(&options)?;
        let vault_root = vault_dir.as_ref().to_path_buf();
        // Deterministic open-time migration + fail-closed validation (#1935):
        // every open hardens each pre-existing descendant to owner-only modes
        // (0700 dirs / 0600 files) and rejects any symlinked descendant
        // (O_NOFOLLOW) before any recovery read, so a tree written by an older,
        // umask-honouring binary — or a swapped-in symlink — can neither serve
        // nor extend a world-accessible vault. Applied on read-only opens too so
        // a `readback`/rebuild cannot silently read a world-accessible tree; if
        // the modes cannot be corrected the open fails closed.
        crate::private_fs::harden_tree(&vault_root)?;
        let mut recovery = DurableVault::recover_batches(vault_dir.as_ref(), &options)?;
        let ledger_hook = if options.restore_ledger_hook && options.value_crypto.is_some() {
            Some(ledger_hook::recover_hook(
                &recovery,
                options.ledger_checkpoint.clone(),
            )?)
        } else if options.restore_ledger_hook {
            Some(ledger_hook::recover_hook_from_vault_dir(
                vault_dir.as_ref(),
                &recovery,
                options.ledger_checkpoint.clone(),
                options.tiering_policy.as_ref(),
            )?)
        } else {
            None
        };
        let recovery_report = VaultRecoveryReport {
            last_recovered_seq: recovery.last_recovered_seq,
            torn_tail: recovery.torn_tail.clone(),
        };
        let mut selected_cfs = options.selected_cfs.clone();
        if recovery.migrate_derived_content_model {
            let required = durable::recovery_readback::persistent_search_cfs(
                vault_dir.as_ref(),
                options.tiering_policy.as_ref(),
            )?;
            if let Some(selected) = selected_cfs.as_mut() {
                selected.extend(required);
                selected.sort();
                selected.dedup();
            }
        }
        let router = match &selected_cfs {
            Some(cfs) => CfRouter::open_selected_cfs_with_tiering_and_crypto(
                vault_dir.as_ref(),
                options.memtable_byte_cap,
                cfs.iter().copied(),
                options.tiering_policy.clone(),
                options.value_crypto.clone(),
            )?,
            None => CfRouter::open_with_tiering_and_crypto(
                vault_dir.as_ref(),
                options.memtable_byte_cap,
                options.tiering_policy.clone(),
                options.value_crypto.clone(),
            )?,
        };
        if recovery.migrate_derived_content_model {
            recovery.derived_content_floor_seq =
                router.prove_persistent_search_content_watermark(recovery.wal_replay_floor_seq)?;
        }
        let rows = if recovery.router_latest_readback {
            VersionedCfStore::new_with_router_latest_readback(recovery.last_recovered_seq, router)
        } else {
            VersionedCfStore::new_with_router(recovery.last_recovered_seq, router)
        };
        // Derived-content watermark (issue #1100): the manifest floor vouches
        // for checkpointed seqs; replayed batches below re-derive the rest
        // from their CFs.
        rows.advance_derived_content_seq_to_at_least(recovery.derived_content_floor_seq);
        // WAL-tail batches have no durable-batch SSTs yet; write-capable
        // handles must re-stage them so no later manifest advance can strand
        // them behind the WAL replay floor (issue #1132).
        let wal_tail_batches: Vec<(u64, Vec<encode::WriteRow>)> = if options.read_only {
            Vec::new()
        } else {
            recovery
                .batches
                .iter()
                .filter(|batch| batch.seq > recovery.wal_replay_floor_seq)
                .map(|batch| (batch.seq, batch.rows.clone()))
                .collect()
        };
        for batch in recovery.batches {
            let rows_at_seq = batch
                .rows
                .into_iter()
                .map(|row| (row.cf, row.key, row.value));
            rows.restore_batch(batch.seq, rows_at_seq)?;
        }
        rows.set_start_seq(recovery.last_recovered_seq)?;
        if !recovery.router_latest_readback {
            // Full-restore contract (issue #1132): every row physically held
            // in Router-class SSTs must be visible to the restored MVCC state,
            // otherwise snapshot reads on this handle silently miss it.
            let violations = durable::router_coverage::router_only_rows(
                vault_dir.as_ref(),
                options.tiering_policy.as_ref(),
                |cf, key| rows.has_any_version(cf, key),
            )?;
            if !violations.is_empty() {
                return Err(durable::router_coverage::router_only_rows_error(
                    &violations,
                ));
            }
        }
        let mut durable_options = options.clone();
        durable_options.temporal_policy = recovery.temporal_policy;
        durable_options.dedup_policy = recovery.dedup_policy;
        durable_options.retention_horizon = recovery.retention_horizon.clone();
        let dedup_policy = durable_options.dedup_policy.clone().unwrap_or_default();
        let retention_horizon = durable_options.retention_horizon.clone();
        let durable = if options.read_only {
            None
        } else {
            let durable = DurableVault::open_after(
                vault_dir.as_ref(),
                &durable_options,
                recovery.wal_replay_floor_seq,
                recovery.derived_content_floor_seq,
            )?;
            if let Some(floor) = recovery.wal_tail_stream_floor {
                // Streaming recovery (issue #1992): the post-floor WAL tail was
                // not materialized into `recovery.batches`. Restore + stage it
                // one commit at a time with a reused decode buffer, and flush
                // the staged checkpoint every `RECOVERY_FLUSH_CHUNK_BYTES` so
                // `pending_checkpoint` stays O(chunk) instead of holding the
                // whole tail — otherwise a multi-GB orphaned tail (three
                // SIGKILLed weave runs) OOM-kills the open. Value crypto is
                // `None` on this path (recover_batches gate), so a decoded batch
                // is already the plaintext rows; the torn newest tail was healed
                // during metadata recovery, so the scan sees no torn tail here.
                const RECOVERY_FLUSH_CHUNK_BYTES: u64 = 512 * 1024 * 1024;
                let mut staged_bytes: u64 = 0;
                crate::wal::for_each_record_payload_after(
                    vault_root.join("wal"),
                    floor,
                    |seq, payload| {
                        let batch_rows = encode::decode_write_batch(payload)?;
                        rows.restore_batch(
                            seq,
                            batch_rows
                                .iter()
                                .map(|row| (row.cf, row.key.clone(), row.value.clone())),
                        )?;
                        staged_bytes = staged_bytes.saturating_add(payload.len() as u64);
                        durable.stage_recovered_wal_batch(seq, batch_rows)?;
                        if staged_bytes >= RECOVERY_FLUSH_CHUNK_BYTES {
                            // Drains pending_checkpoint into durable-batch SSTs
                            // and advances the manifest (crash-safe idempotent
                            // redo); bounds staging heap to one chunk.
                            durable.flush()?;
                            staged_bytes = 0;
                        }
                        Ok(true)
                    },
                )?;
            } else {
                durable.stage_recovered_wal_batches(wal_tail_batches)?;
            }
            Some(durable)
        };
        // Data residency (PRD 30 §4): a caller-supplied pin is enforced against
        // tiering and persisted (conflict-checked, immutable); on reopen the
        // on-disk pin is authoritative and re-enforced against tiering.
        if let Some(pin) = &options.residency {
            if let Some(tiering) = &options.tiering_policy {
                pin.enforce_tier_roots(&tiering.tier_roots())?;
            }
            pin.persist(&vault_root)?;
        }
        let residency = crate::residency::Residency::load(&vault_root)?;
        if options.residency.is_none()
            && let (Some(pin), Some(tiering)) = (&residency, &options.tiering_policy)
        {
            pin.enforce_tier_roots(&tiering.tier_roots())?;
        }
        let vault = Self {
            vault_id,
            vault_salt: vault_salt.into(),
            clock,
            rows,
            durable,
            dedup_policy,
            retention_horizon: Mutex::new(retention_horizon),
            ledger_hook,
            read_only: options.read_only,
            recurrence_write_lock: Mutex::new(()),
            recovery_report,
            residency,
        };
        if options.read_only {
            if crate::retained_input::pending_entries_exist(&vault_root)? {
                return Err(CalyxError {
                    code: crate::retained_input::CALYX_INPUT_RETENTION_RECONCILIATION_REQUIRED,
                    message: format!(
                        "read-only open found pending retained-input state in {}",
                        vault_root.join("inputs").join(".pending").display()
                    ),
                    remediation: "open the vault write-capable once to reconcile retained inputs before serving reads",
                });
            }
        } else {
            vault.reconcile_retained_input_state()?;
        }
        Ok(vault)
    }
}
