use super::{AsterVault, encode, ledger_hook};
use calyx_core::{CalyxError, Clock, Result, Seq};

impl<C> AsterVault<C>
where
    C: Clock,
{
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
        self.refresh_from_durable()?;
        f()
    }

    fn refresh_from_durable(&self) -> Result<()> {
        let Some(durable) = &self.durable else {
            return Ok(());
        };
        let _append_guard = crate::file_lock::FileLockGuard::acquire(&durable.append_lock_path())?;
        let current = self.latest_seq();
        let recovered = durable.recover_current_batches()?;
        if let Some(hook) = &self.ledger_hook {
            ledger_hook::refresh_hook(hook, &recovered, durable.ledger_checkpoint())?;
        }
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
        let Some(durable) = &self.durable else {
            return self.commit_rows_to_mvcc(rows);
        };

        let durable_seq = durable.append_batch(rows)?;
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
        if let Err(error) = durable.checkpoint_batch(durable_seq, rows) {
            eprintln!("calyx durable checkpoint failed after WAL seq {durable_seq}: {error}");
        }
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
        self.rows.set_start_seq(seq)
    }
}
