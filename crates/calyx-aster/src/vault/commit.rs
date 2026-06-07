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
        let mvcc_seq = self.commit_rows_to_mvcc(rows)?;
        if mvcc_seq != durable_seq {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "durable WAL seq {durable_seq} diverged from MVCC seq {mvcc_seq}"
            )));
        }
        durable.checkpoint_batch(durable_seq, rows)?;
        Ok(mvcc_seq)
    }

    fn commit_rows_to_mvcc(&self, rows: &[encode::WriteRow]) -> Result<Seq> {
        self.rows.commit_batch(
            rows.iter()
                .map(|row| (row.cf, row.key.clone(), row.value.clone())),
        )
    }
}
