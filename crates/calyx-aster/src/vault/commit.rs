use super::{AsterVault, encode};
use calyx_core::{CalyxError, Clock, Result, Seq};

impl<C> AsterVault<C>
where
    C: Clock,
{
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
