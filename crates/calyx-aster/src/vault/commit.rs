use super::{AsterVault, encode, ledger_hook};
use calyx_core::{CalyxError, Clock, Result, Seq};

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
            ledger_hook::refresh_hook(
                hook,
                durable.root(),
                &recovered,
                durable.ledger_checkpoint(),
            )?;
        }
        self.replace_retention_horizon(recovered.retention_horizon.clone())?;
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

    pub(crate) fn commit_rows_locked(&self, rows: &[encode::WriteRow]) -> Result<Seq> {
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
            Err(error) => {
                self.restore_committed_rows(durable_seq, rows)?;
                eprintln!(
                    "calyx durable commit restored WAL seq {durable_seq} after MVCC/router error: {error}"
                );
                if let Err(checkpoint_error) = durable.checkpoint_batch(durable_seq, rows) {
                    eprintln!(
                        "calyx durable checkpoint failed after WAL seq {durable_seq}: {checkpoint_error}"
                    );
                }
                return Ok(durable_seq);
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
        self.rows.commit_batch(
            rows.iter()
                .map(|row| (row.cf, row.key.clone(), row.value.clone())),
        )
    }

    fn restore_committed_rows(&self, seq: Seq, rows: &[encode::WriteRow]) -> Result<()> {
        self.rows.restore_batch(
            seq,
            rows.iter()
                .map(|row| (row.cf, row.key.clone(), row.value.clone())),
        )?;
        self.rows.advance_to_at_least(seq);
        Ok(())
    }
}
