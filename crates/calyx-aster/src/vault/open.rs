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
        let recovery = DurableVault::recover_batches(vault_dir.as_ref(), &options)?;
        let ledger_hook = if options.restore_ledger_hook {
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
        let router = match &options.selected_cfs {
            Some(cfs) => CfRouter::open_selected_cfs(
                vault_dir.as_ref(),
                options.memtable_byte_cap,
                cfs.iter().copied(),
            )?,
            None => CfRouter::open_with_tiering(
                vault_dir.as_ref(),
                options.memtable_byte_cap,
                options.tiering_policy.clone(),
            )?,
        };
        let rows = if recovery.router_latest_readback {
            VersionedCfStore::new_with_router_latest_readback(recovery.last_recovered_seq, router)
        } else {
            VersionedCfStore::new_with_router(recovery.last_recovered_seq, router)
        };
        // Derived-content watermark (issue #1100): the manifest floor vouches
        // for checkpointed seqs; replayed batches below re-derive the rest
        // from their CFs.
        rows.advance_derived_content_seq_to_at_least(recovery.derived_content_floor_seq);
        for batch in recovery.batches {
            let rows_at_seq = batch
                .rows
                .into_iter()
                .map(|row| (row.cf, row.key, row.value));
            rows.restore_batch(batch.seq, rows_at_seq)?;
        }
        rows.set_start_seq(recovery.last_recovered_seq)?;
        let mut durable_options = options.clone();
        durable_options.temporal_policy = recovery.temporal_policy;
        durable_options.dedup_policy = recovery.dedup_policy;
        durable_options.retention_horizon = recovery.retention_horizon.clone();
        let dedup_policy = durable_options.dedup_policy.clone().unwrap_or_default();
        let retention_horizon = durable_options.retention_horizon.clone();
        let durable = if options.read_only {
            None
        } else {
            Some(DurableVault::open_after(
                vault_dir.as_ref(),
                &durable_options,
                recovery.wal_replay_floor_seq,
            )?)
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
        Ok(Self {
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
        })
    }
}
