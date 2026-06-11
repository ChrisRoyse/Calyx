use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, Result};

use super::{
    CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION, CALYX_ASTER_SNAPSHOT_UNAVAILABLE, MvccSnapshot,
};

pub struct AsterRebuildSource<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> Copy for AsterRebuildSource<'a, C> where C: Clock {}

impl<'a, C> Clone for AsterRebuildSource<'a, C>
where
    C: Clock,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, C> AsterRebuildSource<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }

    pub fn latest_snapshot(&self) -> MvccSnapshot {
        self.vault.latest_seq()
    }

    pub fn scan_cf(
        &self,
        snapshot: MvccSnapshot,
        cf: ColumnFamily,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        ensure_allowed_source(cf)?;
        self.vault
            .scan_cf_at(snapshot, cf)
            .map_err(|error| snapshot_unavailable(cf, error))
    }

    pub fn read_cf(
        &self,
        snapshot: MvccSnapshot,
        cf: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        ensure_allowed_source(cf)?;
        self.vault
            .read_cf_at(snapshot, cf, key)
            .map_err(|error| snapshot_unavailable(cf, error))
    }
}

fn ensure_allowed_source(cf: ColumnFamily) -> Result<()> {
    match cf {
        ColumnFamily::Base | ColumnFamily::Anchors | ColumnFamily::Slot { .. } => Ok(()),
        _ => Err(CalyxError {
            code: CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION,
            message: format!("rebuild attempted to read derived CF {}", cf.name()),
            remediation: "derive rebuild artifacts only from base, slot, and anchor source CFs",
        }),
    }
}

fn snapshot_unavailable(cf: ColumnFamily, error: CalyxError) -> CalyxError {
    CalyxError {
        code: CALYX_ASTER_SNAPSHOT_UNAVAILABLE,
        message: format!("snapshot read failed for {}: {}", cf.name(), error.message),
        remediation: "retry rebuild against a fresh Aster MVCC snapshot",
    }
}
