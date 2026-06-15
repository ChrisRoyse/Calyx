use super::*;
use calyx_core::{CalyxError, Result};
use proptest::prelude::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn tombstone_ratio_matches_hand_computed_counts() {
    assert_eq!(tombstone_ratio_for_counts(100, 100), 0.5);
    assert!((tombstone_ratio_for_counts(200, 100) - 0.666_666_666).abs() < 0.000_001);
}

#[test]
fn maybe_trigger_compacts_when_ratio_high_and_io_available() {
    let target = MockTarget::new(inventory(60, 40, 12_000), inventory(0, 100, 8_000), None);
    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1_000, 100_000, 0);

    let result = reclaimer.maybe_trigger_at(&target, 0.5, 0);

    assert!(result.triggered);
    assert_eq!(target.compact_calls.load(Ordering::Relaxed), 1);
    assert_eq!(result.tombstones_removed, 60);
    assert_eq!(result.bytes_compacted, 12_000);
    assert_eq!(result.bytes_freed, 4_000);
    assert_eq!(result.compacted_cfs, vec!["base"]);
    assert_eq!(reclaimer.debt_metric.load(Ordering::Relaxed), 8_000);
}

#[test]
fn maybe_trigger_uses_per_cf_ratio_when_indexes_lower_total_ratio() {
    let target = MockTarget::new(
        inventory_with_index(60, 40, 200, 20_000),
        inventory_with_index(0, 100, 200, 14_000),
        None,
    );
    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1_000, 100_000, 0);

    let result = reclaimer.maybe_trigger_at(&target, 0.5, 0);

    assert!(result.tombstone_ratio_before < DEFAULT_TOMBSTONE_RATIO_TRIGGER);
    assert!(result.triggered);
    assert_eq!(result.compacted_cfs, vec!["base"]);
    assert_eq!(result.tombstones_removed, 60);
}

#[test]
fn maybe_trigger_skips_when_io_available_is_too_low() {
    let target = MockTarget::new(inventory(60, 40, 12_000), inventory(0, 100, 8_000), None);
    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1_000, 100_000, 0);

    let result = reclaimer.maybe_trigger_at(&target, 0.1, 0);

    assert!(!result.triggered);
    assert_eq!(result.skipped_reason, Some(SKIP_LOW_IO_AVAILABLE));
    assert_eq!(target.compact_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn token_bucket_enforces_one_second_io_budget() {
    let throttle = CompactionThrottle::new(0.5, 2 * 1024 * 1024, 0);

    assert!(throttle.try_consume(1024 * 1024, 0));
    assert!(!throttle.try_consume(1, 0));
    assert!(throttle.try_consume(1024 * 1024, 1_000));
}

#[test]
fn write_amp_uses_manifest_style_compaction_over_flush_bytes() {
    let stats = CompactionIoStats {
        bytes_written_by_compaction: 4 * 1024 * 1024 * 1024,
        bytes_written_by_flush: 1024 * 1024 * 1024,
    };

    assert_eq!(stats.write_amp(), 4.0);
}

#[test]
fn all_tombstones_trigger_and_remove_to_zero_ratio() {
    let target = MockTarget::new(inventory(10, 0, 4_096), inventory(0, 0, 128), None);
    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1_000, 100_000, 0);

    let result = reclaimer.maybe_trigger_at(&target, 0.5, 0);

    assert!(result.triggered);
    assert_eq!(result.tombstone_ratio_before, 1.0);
    assert_eq!(result.tombstone_ratio_after, 0.0);
    assert_eq!(result.tombstones_removed, 10);
}

#[test]
fn zero_tombstones_do_not_trigger() {
    let target = MockTarget::new(inventory(0, 10, 4_096), inventory(0, 10, 4_096), None);
    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1_000, 100_000, 0);

    let result = reclaimer.maybe_trigger_at(&target, 0.5, 0);

    assert_eq!(result.skipped_reason, Some(SKIP_LOW_TOMBSTONE_RATIO));
    assert_eq!(target.compact_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn compaction_error_preserves_code_and_debt() {
    let error = CalyxError {
        code: "CALYX_IO_ERROR",
        message: "synthetic compaction I/O failure".to_string(),
        remediation: "inspect device and retry",
    };
    let target = MockTarget::new(
        inventory(60, 40, 12_000),
        inventory(0, 100, 8_000),
        Some(error),
    );
    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1_000, 100_000, 0);

    let result = reclaimer.maybe_trigger_at(&target, 0.5, 0);

    assert_eq!(result.error_code, Some("CALYX_IO_ERROR"));
    assert_eq!(result.compaction_debt, 12_000);
    assert_eq!(reclaimer.debt_metric.load(Ordering::Relaxed), 12_000);
}

#[test]
fn adaptive_cadence_increases_only_when_debt_alert_and_serving_safe() {
    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1_000, 100_000, 0);
    reclaimer.debt_metric.store(2_000, Ordering::Relaxed);

    let aggressive = reclaimer.adaptive_cadence(true);
    let conservative = reclaimer.adaptive_cadence(false);

    assert!(aggressive.debt_alert);
    assert!((aggressive.max_io_fraction - 0.3).abs() < f64::EPSILON);
    assert!((conservative.max_io_fraction - 0.1).abs() < f64::EPSILON);
}

proptest! {
    #[test]
    fn tombstone_ratio_is_bounded(tombstones in 0_u64..1_000_000, live in 0_u64..1_000_000) {
        let ratio = tombstone_ratio_for_counts(tombstones, live);
        prop_assert!((0.0..=1.0).contains(&ratio));
        if tombstones + live > 0 {
            prop_assert!((ratio - tombstones as f64 / (tombstones + live) as f64).abs() < f64::EPSILON);
        }
    }
}

#[derive(Debug)]
struct MockTarget {
    before: TombstoneInventory,
    after: TombstoneInventory,
    compact_error: Mutex<Option<CalyxError>>,
    compact_calls: AtomicU64,
}

impl MockTarget {
    fn new(
        before: TombstoneInventory,
        after: TombstoneInventory,
        compact_error: Option<CalyxError>,
    ) -> Self {
        Self {
            before,
            after,
            compact_error: Mutex::new(compact_error),
            compact_calls: AtomicU64::new(0),
        }
    }
}

impl CompactionGcTarget for MockTarget {
    fn tombstone_inventory(&self) -> Result<TombstoneInventory> {
        if self.compact_calls.load(Ordering::Relaxed) == 0 {
            Ok(self.before.clone())
        } else {
            Ok(self.after.clone())
        }
    }

    fn compact_tombstoned_cfs(&self, _cfs: &[ColumnFamily]) -> Result<()> {
        self.compact_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(error) = self
            .compact_error
            .lock()
            .expect("mock compact error poisoned")
            .take()
        {
            return Err(error);
        }
        Ok(())
    }
}

fn inventory(tombstones: u64, live: u64, sst_bytes: u64) -> TombstoneInventory {
    TombstoneInventory {
        per_cf: vec![TombstoneCfStats {
            cf: ColumnFamily::Base,
            cf_name: "base".to_string(),
            sst_files: 2,
            sst_bytes,
            live_keys: live,
            tombstone_keys: tombstones,
            live_value_bytes: live * 16,
            tombstone_value_bytes: tombstones * 16,
        }],
        io_stats: CompactionIoStats {
            bytes_written_by_compaction: 0,
            bytes_written_by_flush: sst_bytes,
        },
    }
}

fn inventory_with_index(
    base_tombstones: u64,
    base_live: u64,
    index_live: u64,
    sst_bytes: u64,
) -> TombstoneInventory {
    let base_bytes = sst_bytes / 2;
    TombstoneInventory {
        per_cf: vec![
            TombstoneCfStats {
                cf: ColumnFamily::Base,
                cf_name: "base".to_string(),
                sst_files: 2,
                sst_bytes: base_bytes,
                live_keys: base_live,
                tombstone_keys: base_tombstones,
                live_value_bytes: base_live * 16,
                tombstone_value_bytes: base_tombstones * 16,
            },
            TombstoneCfStats {
                cf: ColumnFamily::TimeIndex,
                cf_name: "time_index".to_string(),
                sst_files: 2,
                sst_bytes: sst_bytes - base_bytes,
                live_keys: index_live,
                tombstone_keys: 0,
                live_value_bytes: index_live * 16,
                tombstone_value_bytes: 0,
            },
        ],
        io_stats: CompactionIoStats {
            bytes_written_by_compaction: 0,
            bytes_written_by_flush: sst_bytes,
        },
    }
}
