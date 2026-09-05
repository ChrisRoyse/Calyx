use super::{AsterVault, encode, ledger_hook};
use calyx_core::{CalyxError, Clock, Result, Seq};

/// The WAL append is durable, but the live MVCC/router apply failed and the
/// caller must reconcile the reported sequence before retrying.
pub const CALYX_DURABLE_COMMIT_RECONCILIATION_REQUIRED: &str =
    "CALYX_DURABLE_COMMIT_RECONCILIATION_REQUIRED";

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub(crate) fn with_durable_commit_lock<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let Some(durable) = &self.durable else {
            return f();
        };
        let _commit_guard = crate::file_lock::FileLockGuard::acquire(&durable.commit_lock_path())?;
        if durable.durable_tip_seq()? > self.latest_seq() {
            self.refresh_from_durable()?;
        }
        f()
    }

    pub(crate) fn with_recurrence_write_lock<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let _guard = self
            .recurrence_write_lock
            .lock()
            .map_err(|_| CalyxError::backpressure("recurrence write lock poisoned"))?;
        let _file_guard = self
            .durable
            .as_ref()
            .map(|durable| {
                crate::file_lock::FileLockGuard::acquire(&durable.recurrence_lock_path())
            })
            .transpose()?;
        if self
            .durable
            .as_ref()
            .map(|durable| durable.durable_tip_seq())
            .transpose()?
            .is_some_and(|tip| tip > self.latest_seq())
        {
            self.refresh_from_durable()?;
        }
        f()
    }

    fn refresh_from_durable(&self) -> Result<()> {
        let Some(durable) = &self.durable else {
            return Ok(());
        };
        let current = self.latest_seq();
        let recovered = durable.recover_current_batches()?;
        if let Some(hook) = &self.ledger_hook {
            if durable.value_crypto_enabled() {
                ledger_hook::refresh_hook_from_recovery(
                    hook,
                    &recovered,
                    durable.ledger_checkpoint(),
                )?;
            } else {
                ledger_hook::refresh_hook(
                    hook,
                    durable.root(),
                    &recovered,
                    durable.ledger_checkpoint(),
                    durable.tiering_policy(),
                )?;
            }
        }
        self.replace_retention_horizon(recovered.retention_horizon.clone())?;
        self.rows
            .advance_derived_content_seq_to_at_least(recovered.derived_content_floor_seq);
        durable.advance_derived_content_watermark_to_at_least(recovered.derived_content_floor_seq);
        // WAL-tail batches from a foreign writer have no durable-batch SSTs
        // yet; stage them here so this handle's next checkpoint flush cannot
        // advance the manifest past them if that writer dies (issue #1132).
        durable.stage_recovered_wal_batches(
            recovered
                .batches
                .iter()
                .filter(|batch| batch.seq > recovered.wal_replay_floor_seq)
                .map(|batch| (batch.seq, batch.rows.clone()))
                .collect(),
        )?;
        for batch in &recovered.batches {
            if batch.seq <= current {
                continue;
            }
            let rows_at_seq = batch
                .rows
                .iter()
                .map(|row| (row.cf, row.key.clone(), row.value.clone()));
            self.rows.restore_batch(batch.seq, rows_at_seq)?;
        }
        self.rows.advance_to_at_least(recovered.last_recovered_seq);
        Ok(())
    }

    pub(super) fn commit_rows(&self, rows: &[encode::WriteRow]) -> Result<Seq> {
        self.with_durable_commit_lock(|| self.commit_rows_locked(rows))
    }

    pub(super) fn commit_reconstructible_rows(&self, rows: &[encode::WriteRow]) -> Result<Seq> {
        self.with_durable_commit_lock(|| self.commit_reconstructible_rows_locked(rows))
    }

    pub(crate) fn commit_reconstructible_rows_locked(
        &self,
        rows: &[encode::WriteRow],
    ) -> Result<Seq> {
        self.ensure_writeable("commit reconstructible derived generation")?;
        for row in rows {
            let isolated = match row.cf {
                crate::cf::ColumnFamily::Graph => {
                    crate::plain_graph::is_generation_storage_graph_key(&row.key)
                }
                crate::cf::ColumnFamily::XTerm => {
                    crate::cf::is_weave_xterm_generation_key(&row.key)
                }
                _ => false,
            };
            if !isolated {
                return Err(CalyxError {
                    code: "CALYX_RECONSTRUCTIBLE_DERIVED_NAMESPACE_REQUIRED",
                    message: format!(
                        "reconstructible write rejected non-isolated row cf={} key_len={}",
                        row.cf.name(),
                        row.key.len()
                    ),
                    remediation: "write only generation-namespaced Graph/XTerm rows, physically verify the complete generation, then publish it with a WAL-backed lifecycle commit",
                });
            }
        }
        let Some(durable) = &self.durable else {
            return Err(CalyxError {
                code: "CALYX_RECONSTRUCTIBLE_DERIVED_DURABILITY_REQUIRED",
                message: "reconstructible derived writes require a durable Aster vault".to_string(),
                remediation: "open the real durable vault before building a derived generation",
            });
        };
        durable.ensure_disk_write_allowed(self.rows.resource_counters())?;
        // Never advance the manifest over an older WAL-only commit. Drain
        // every pending checkpoint first; the generation CF router levels
        // are refreshed once after all direct batches are durable.
        durable.flush()?;
        let expected = self.rows.current_seq().checked_add(1).ok_or_else(|| {
            CalyxError::aster_corrupt_shard(
                "sequence exhausted while writing reconstructible derived generation",
            )
        })?;
        // The bulk payload itself bypasses WAL, but the global sequence
        // cannot: the WAL batcher and MVCC allocator must reserve the same
        // sequence or the next authoritative commit will diverge. Persist
        // an empty write-batch fence (no reconstructible row bytes), then
        // checkpoint the generation payload directly into authenticated
        // SSTs at that sequence. Recovery can advance through the empty
        // fence if checkpoint installation is interrupted, while the
        // unpublished namespace remains either fully read back and
        // accepted later or safely unreachable.
        let seq = durable.append_batch(&[])?;
        if seq != expected {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "reconstructible sequence fence {seq} diverged from next MVCC seq {expected}"
            )));
        }
        durable.checkpoint_batch(seq, rows)?;
        let feeds_derived_content = rows
            .iter()
            .any(|row| row.cf.feeds_persistent_search_index());
        self.rows
                .advance_reconstructible_checkpoint(seq, feeds_derived_content)
                .map_err(|error| CalyxError {
                    code: "CALYX_RECONSTRUCTIBLE_DERIVED_REOPEN_REQUIRED",
                    message: format!(
                        "checkpoint seq {seq} is durable but the live router could not install it: [{}] {}",
                        error.code, error.message
                    ),
                    remediation: "stop this process and reopen the vault; do not publish the generation until physical readback succeeds",
                })?;
        Ok(seq)
    }

    pub(crate) fn commit_rows_locked(&self, rows: &[encode::WriteRow]) -> Result<Seq> {
        if rows
            .iter()
            .any(|row| row.cf == crate::cf::ColumnFamily::TimeIndex)
        {
            return Err(CalyxError::aster_corrupt_shard(
                "time_index is a reserved derived column family; caller-supplied rows are forbidden because they can forge or corrupt the sole time-to-sequence mapping",
            ));
        }
        self.commit_rows_locked_inner(rows)
    }

    /// Trusted erasure path for tombstoning existing derived TimeIndex rows.
    /// It deliberately accepts only the MVCC tombstone value, never a forged
    /// live time-to-sequence mapping.
    pub(crate) fn commit_erasure_rows_locked(&self, rows: &[encode::WriteRow]) -> Result<Seq> {
        if let Some(row) = rows.iter().find(|row| {
            row.cf == crate::cf::ColumnFamily::TimeIndex
                && !crate::mvcc::is_tombstone_value(&row.value)
        }) {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "trusted erasure attempted a live time_index write: key_len={} value_len={}",
                row.key.len(),
                row.value.len()
            )));
        }
        self.commit_rows_locked_inner(rows)
    }

    fn commit_rows_locked_inner(&self, rows: &[encode::WriteRow]) -> Result<Seq> {
        if rows.is_empty() {
            // Empty commit: do not advance the seq or stamp a time-index entry.
            return self.commit_prepared_rows(rows);
        }
        // Time-travel (PH72 T04): stamp this group-commit with one time-index
        // entry in the SAME batch as the data, so the (millis -> seqno) mapping
        // is atomic with the write — a crash can never leave a write without its
        // time mapping (A15). We hold the durable commit lock here, so the next
        // allocated seq is exactly current_seq()+1; we assert that against the
        // committed seq below and fail loud on any divergence (never silent).
        let predicted = self.rows.current_seq().saturating_add(1);
        let (cf, key, value) = crate::timetravel::entry_row(self.clock.now(), predicted);
        let mut all_rows = rows.to_vec();
        all_rows.push(encode::WriteRow { cf, key, value });
        let committed = self.commit_prepared_rows(&all_rows)?;
        if committed != predicted {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "time-index seqno prediction {predicted} diverged from committed seq {committed}"
            )));
        }
        Ok(committed)
    }

    fn commit_prepared_rows(&self, rows: &[encode::WriteRow]) -> Result<Seq> {
        if !rows.is_empty() {
            self.ensure_writeable("commit")?;
        }
        self.rows.ensure_memtable_admission(
            rows.iter()
                .map(|row| (row.cf, row.key.as_slice(), row.value.as_slice())),
        )?;
        let Some(durable) = &self.durable else {
            return self.commit_rows_to_mvcc(rows);
        };

        durable.ensure_disk_write_allowed(self.rows.resource_counters())?;
        let durable_seq = durable.append_batch(rows)?;
        if let Some(anchor) = crate::ledger_head::newest_anchor_from_rows(rows)? {
            crate::ledger_head::write_head_anchor(durable.root(), &anchor)?;
        }
        let mvcc_seq = match self.commit_rows_to_mvcc(rows) {
            Ok(seq) => seq,
            Err(mvcc_error) => {
                let restore = self.restore_committed_rows(durable_seq, rows);
                let checkpoint = durable.checkpoint_batch(durable_seq, rows);
                return Err(post_wal_commit_error(
                    durable_seq,
                    &mvcc_error,
                    &restore,
                    &checkpoint,
                ));
            }
        };
        if mvcc_seq != durable_seq {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "durable WAL seq {durable_seq} diverged from MVCC seq {mvcc_seq}"
            )));
        }
        durable.stage_checkpoint_batch(durable_seq, rows)?;
        Ok(mvcc_seq)
    }

    fn commit_rows_to_mvcc(&self, rows: &[encode::WriteRow]) -> Result<Seq> {
        #[cfg(test)]
        if self
            .durable
            .as_ref()
            .is_some_and(|durable| durable.take_mvcc_commit_failure())
        {
            return Err(CalyxError::aster_corrupt_shard(
                "injected post-WAL MVCC/router commit failure",
            ));
        }
        self.rows.commit_batch(
            rows.iter()
                .map(|row| (row.cf, row.key.clone(), row.value.clone())),
        )
    }

    fn restore_committed_rows(&self, seq: Seq, rows: &[encode::WriteRow]) -> Result<()> {
        #[cfg(test)]
        if self
            .durable
            .as_ref()
            .is_some_and(|durable| durable.take_mvcc_restore_failure())
        {
            return Err(CalyxError::aster_corrupt_shard(
                "injected post-WAL MVCC restore failure",
            ));
        }
        self.rows.restore_batch(
            seq,
            rows.iter()
                .map(|row| (row.cf, row.key.clone(), row.value.clone())),
        )?;
        self.rows.advance_to_at_least(seq);
        Ok(())
    }
}

fn post_wal_commit_error(
    durable_seq: Seq,
    mvcc_error: &CalyxError,
    restore: &Result<()>,
    checkpoint: &Result<()>,
) -> CalyxError {
    CalyxError {
        code: CALYX_DURABLE_COMMIT_RECONCILIATION_REQUIRED,
        message: format!(
            "WAL commit is durable but live MVCC/router application failed; wal_seq={durable_seq} \
             mvcc=error[{}]: {} restore={} checkpoint={}",
            mvcc_error.code,
            mvcc_error.message,
            reconciliation_outcome(restore),
            reconciliation_outcome(checkpoint),
        ),
        remediation: "treat wal_seq as durably committed; reconcile by idempotency/readback or reopen the vault before retrying",
    }
}

fn reconciliation_outcome(result: &Result<()>) -> String {
    match result {
        Ok(()) => "ok".to_string(),
        Err(error) => format!("error[{}]: {}", error.code, error.message),
    }
}

#[cfg(test)]
#[path = "commit_failure_tests.rs"]
mod failure_tests;
