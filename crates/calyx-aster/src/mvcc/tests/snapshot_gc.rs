use super::*;
use crate::gc::GcRateLimit;
use calyx_core::{Clock, FixedClock};
use proptest::prelude::*;
use std::time::Duration;

fn version_value(seq: u64) -> Vec<u8> {
    format!("v{seq:03}").into_bytes()
}

#[test]
fn safe_point_50_reclaims_exactly_49_and_keeps_visible_boundary() {
    let clock = FixedClock::new(1_000);
    let store = VersionedCfStore::default();
    for seq in 1..=100 {
        let committed = store
            .commit_batch([(ColumnFamily::Base, b"ph58-key".to_vec(), version_value(seq))])
            .unwrap();
        assert_eq!(committed, seq);
    }
    let snapshot = store.pin_snapshot_at(50, Freshness::FreshDerived, &clock, 60_000);
    store.set_snapshot_gc_rate_limit(GcRateLimit::new(1_000, Duration::ZERO));

    let result = store.snapshot_version_gc_tick(&clock).unwrap();

    assert_eq!(result.safe_point_seq, 50);
    assert_eq!(result.versions_reclaimed, 49);
    assert_eq!(result.bytes_freed, 49 * 4);
    assert_eq!(result.compaction_debt, 0);
    assert_eq!(
        store
            .read_at(snapshot, ColumnFamily::Base, b"ph58-key", &clock)
            .unwrap(),
        Some(version_value(50))
    );
    let latest = store.pin_snapshot_at(100, Freshness::FreshDerived, &clock, 60_000);
    assert_eq!(
        store
            .seq_for_key_at(latest, ColumnFamily::Base, b"ph58-key", &clock)
            .unwrap(),
        Some(100)
    );
    let metrics = store.snapshot_gc_metrics(clock.now());
    assert_eq!(metrics.versions_reclaimed_total, 49);
    assert_eq!(metrics.bytes_freed_total, 49 * 4);
    assert_eq!(metrics.compaction_debt, 0);
}

#[test]
fn max_ops_rate_limit_leaves_debt_for_next_tick() {
    let clock = FixedClock::new(2_000);
    let store = VersionedCfStore::default();
    for seq in 1..=100 {
        store
            .commit_batch([(ColumnFamily::Base, b"rate-key".to_vec(), version_value(seq))])
            .unwrap();
    }
    let _pin = store.pin_snapshot_at(50, Freshness::FreshDerived, &clock, 60_000);
    store.set_snapshot_gc_rate_limit(GcRateLimit::new(10, Duration::ZERO));

    let result = store.snapshot_version_gc_tick(&clock).unwrap();

    assert_eq!(result.versions_reclaimed, 10);
    assert_eq!(result.compaction_debt, 39);
    assert!(result.rate_limited);
    assert_eq!(store.snapshot_gc_metrics(clock.now()).compaction_debt, 39);
}

#[test]
fn no_readers_reclaims_history_but_preserves_latest_visible_value() {
    let clock = FixedClock::new(3_000);
    let store = VersionedCfStore::default();
    for seq in 1..=100 {
        store
            .commit_batch([(
                ColumnFamily::Base,
                b"latest-key".to_vec(),
                version_value(seq),
            )])
            .unwrap();
    }
    store.set_snapshot_gc_rate_limit(GcRateLimit::new(1_000, Duration::ZERO));

    let result = store.snapshot_version_gc_tick(&clock).unwrap();

    assert_eq!(result.safe_point_seq, 100);
    assert_eq!(result.versions_reclaimed, 99);
    assert_eq!(result.compaction_debt, 0);
    let latest = store.pin_snapshot_at(100, Freshness::FreshDerived, &clock, 60_000);
    assert_eq!(
        store
            .read_at(latest, ColumnFamily::Base, b"latest-key", &clock)
            .unwrap(),
        Some(version_value(100))
    );
}

#[test]
fn exact_newest_safe_point_with_single_version_has_no_work() {
    let clock = FixedClock::new(4_000);
    let store = VersionedCfStore::default();
    store
        .commit_batch([(ColumnFamily::Base, b"single-key".to_vec(), b"only".to_vec())])
        .unwrap();
    let _pin = store.pin_snapshot_at(1, Freshness::FreshDerived, &clock, 60_000);
    store.set_snapshot_gc_rate_limit(GcRateLimit::new(100, Duration::ZERO));

    let result = store.snapshot_version_gc_tick(&clock).unwrap();

    assert_eq!(result.safe_point_seq, store.current_seq());
    assert_eq!(result.versions_reclaimed, 0);
    assert_eq!(result.compaction_debt, 0);
}

proptest! {
    #[test]
    fn gc_preserves_value_visible_at_safe_point(
        mut seqs in proptest::collection::vec(1u64..200, 1..40),
        safe_point in 1u64..200,
    ) {
        seqs.sort_unstable();
        seqs.dedup();
        let clock = FixedClock::new(5_000);
        let store = VersionedCfStore::default();
        for seq in &seqs {
            store
                .restore_batch(*seq, [(ColumnFamily::Base, b"prop-key".to_vec(), version_value(*seq))])
                .unwrap();
        }
        store.advance_to_at_least(*seqs.iter().max().unwrap());
        let snapshot = store.pin_snapshot_at(safe_point, Freshness::FreshDerived, &clock, 60_000);
        let before = store
            .read_at(snapshot, ColumnFamily::Base, b"prop-key", &clock)
            .unwrap();
        store.set_snapshot_gc_rate_limit(GcRateLimit::new(1_000, Duration::ZERO));

        let result = store.snapshot_version_gc_tick(&clock).unwrap();
        let after = store
            .read_at(snapshot, ColumnFamily::Base, b"prop-key", &clock)
            .unwrap();

        prop_assert_eq!(after, before);
        prop_assert!(result.versions_reclaimed <= seqs.iter().filter(|seq| **seq < safe_point).count());
        prop_assert!(result.compaction_debt == 0);
    }
}
