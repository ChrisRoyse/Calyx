use super::{AsterVault, encode};
use crate::cf::{ColumnFamily, temporal_xterm_key};
use calyx_core::{Clock, CxId, Result, Seq};

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub fn put_temporal_xterm(&self, cx_a: CxId, cx_b: CxId, value: Vec<u8>) -> Result<Seq> {
        self.commit_rows(&[encode::WriteRow {
            cf: ColumnFamily::TemporalXTerm,
            key: temporal_xterm_key(cx_a, cx_b),
            value,
        }])
    }

    pub fn read_temporal_xterm(
        &self,
        snapshot: Seq,
        cx_a: CxId,
        cx_b: CxId,
    ) -> Result<Option<Vec<u8>>> {
        self.read_cf_at(
            snapshot,
            ColumnFamily::TemporalXTerm,
            &temporal_xterm_key(cx_a, cx_b),
        )
    }
}
