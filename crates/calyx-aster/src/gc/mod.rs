//! Garbage-collection and reclaimer scaffolding for Aster.

pub mod snapshot_gc;

pub use snapshot_gc::{
    BoundedStalenessSnapshot, CALYX_GC_ERROR, DEFAULT_GC_MAX_OPS_PER_RUN,
    DEFAULT_GC_MIN_INTERVAL_MS, DEFAULT_MAX_PINNED_SEQ_GAP, DEFAULT_READER_LEASE_MS, GapAlert,
    GcMetrics, GcRateLimit, GcResult, GcScheduler, GcSchedulerTick, GcTask, ReadLease, ReaderId,
    SnapshotGcCounters, SnapshotGcReclaimer, SnapshotGcTick, SnapshotPinMetrics,
    SnapshotPinWatchdog, SnapshotVersionGc,
};
