//! Vault-facing bridge for snapshot GC scheduler ticks.

use crate::cf::ColumnFamily;
use crate::gc::SnapshotGcTick;
use crate::mvcc::Snapshot;
use crate::vault::AsterVault;
use calyx_core::{Clock, Result};

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Runs one snapshot-pin watchdog tick.
    ///
    /// The background GC scheduler should call this at its 1 s cadence once the
    /// scheduler exists. Until then, resource-status and tests use the same
    /// underlying store hook to abort expired reader pins fail-closed.
    pub fn snapshot_gc_tick(&self, max_gap_seqs: u64) -> SnapshotGcTick {
        self.rows.snapshot_gc_tick(&self.clock, max_gap_seqs)
    }

    /// Reads one CF row through an explicit tracked reader snapshot.
    pub fn read_pinned_cf(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        self.rows.read_at(snapshot, cf, key, &self.clock)
    }
}
