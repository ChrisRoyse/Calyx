//! Process-lifetime backpressure event counters for one vault store.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic backpressure event counters.
///
/// Prometheus counter semantics: values only increase for the lifetime of the
/// process and reset to zero on restart. One instance is shared between the
/// MVCC store and its CF router so every memtable backpressure event is
/// counted at the point it fires (PRD 24 §8 "backpressure events").
#[derive(Debug, Default)]
pub struct ResourceCounters {
    memtable_absorbed_total: AtomicU64,
    memtable_rejected_total: AtomicU64,
    disk_pressure_events_total: AtomicU64,
}

impl ResourceCounters {
    /// Records a memtable byte-cap hit absorbed by an emergency flush.
    pub fn record_memtable_absorbed(&self) {
        self.memtable_absorbed_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a memtable rejection that persisted after the emergency flush
    /// (single row larger than the byte cap).
    pub fn record_memtable_rejected(&self) {
        self.memtable_rejected_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a `CALYX_DISK_PRESSURE` write-admission rejection.
    pub fn record_disk_pressure(&self) {
        self.disk_pressure_events_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshots the counters into a serializable status section.
    pub fn snapshot(&self) -> BackpressureStatus {
        let absorbed = self.memtable_absorbed_total.load(Ordering::Relaxed);
        let rejected = self.memtable_rejected_total.load(Ordering::Relaxed);
        let disk_pressure = self.disk_pressure_events_total.load(Ordering::Relaxed);
        BackpressureStatus {
            memtable_absorbed_total: absorbed,
            memtable_rejected_total: rejected,
            disk_pressure_events_total: disk_pressure,
            events_total: absorbed.saturating_add(rejected),
        }
    }
}

/// Backpressure section of [`super::ResourceStatus`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackpressureStatus {
    /// `CALYX_BACKPRESSURE` events absorbed by an emergency memtable flush.
    pub memtable_absorbed_total: u64,
    /// `CALYX_BACKPRESSURE` events that persisted after the emergency flush.
    pub memtable_rejected_total: u64,
    /// `CALYX_DISK_PRESSURE` write-admission rejections.
    pub disk_pressure_events_total: u64,
    /// Sum of all backpressure events observed by this process.
    pub events_total: u64,
}
