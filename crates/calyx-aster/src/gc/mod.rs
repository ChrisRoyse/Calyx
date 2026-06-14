//! Garbage-collection and reclaimer scaffolding for Aster.

pub mod snapshot_gc;

pub use snapshot_gc::{
    BoundedStalenessSnapshot, DEFAULT_MAX_PINNED_SEQ_GAP, DEFAULT_READER_LEASE_MS, GapAlert,
    ReadLease, ReaderId, SnapshotGcTick, SnapshotPinMetrics, SnapshotPinWatchdog,
};
