//! Garbage-collection and reclaimer scaffolding for Aster.

pub mod compaction_gc;
pub mod snapshot_gc;

pub use compaction_gc::{
    CompactionCadence, CompactionGcReclaimer, CompactionGcResult, CompactionGcTarget,
    CompactionIoStats, CompactionThrottle, DEFAULT_COMPACTION_DEBT_ALERT_THRESHOLD,
    DEFAULT_DISK_BW_BYTES_PER_SEC, DEFAULT_MAX_IO_FRACTION, DEFAULT_TOMBSTONE_RATIO_TRIGGER,
    TombstoneCfStats, TombstoneInventory, VaultCompactionGcTarget, scan_catalog_tombstones,
    scan_tombstone_inventory, tombstone_ratio_for_counts,
};
pub use snapshot_gc::{
    BoundedStalenessSnapshot, CALYX_GC_ERROR, DEFAULT_GC_MAX_OPS_PER_RUN,
    DEFAULT_GC_MIN_INTERVAL_MS, DEFAULT_MAX_PINNED_SEQ_GAP, DEFAULT_READER_LEASE_MS, GapAlert,
    GcMetrics, GcRateLimit, GcResult, GcScheduler, GcSchedulerTick, GcTask, ReadLease, ReaderId,
    SnapshotGcCounters, SnapshotGcReclaimer, SnapshotGcTick, SnapshotPinMetrics,
    SnapshotPinWatchdog, SnapshotVersionGc,
};
