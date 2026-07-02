//! Issue #1100: the derived-content watermark advances only for commits that
//! write CFs feeding derived search content, so content-neutral commits
//! (idempotency-ledger appends + time-index sentinels) never mark persistent
//! search indexes stale.

use super::*;
use calyx_core::SystemClock;

fn ledger_only_rows() -> Vec<(ColumnFamily, Vec<u8>, Vec<u8>)> {
    vec![
        (ColumnFamily::Ledger, b"lk".to_vec(), b"lv".to_vec()),
        (ColumnFamily::TimeIndex, b"tk".to_vec(), b"tv".to_vec()),
    ]
}

fn base_rows() -> Vec<(ColumnFamily, Vec<u8>, Vec<u8>)> {
    vec![
        (ColumnFamily::Base, b"bk".to_vec(), b"bv".to_vec()),
        (ColumnFamily::TimeIndex, b"tk".to_vec(), b"tv".to_vec()),
    ]
}

#[test]
fn content_neutral_commits_do_not_advance_derived_content_seq() {
    let store = VersionedCfStore::new(0);

    let neutral_seq = store.commit_batch(ledger_only_rows()).expect("ledger");
    assert_eq!(neutral_seq, 1);
    assert_eq!(
        store.derived_content_seq(),
        0,
        "ledger+time-index commit must not advance the watermark"
    );

    let content_seq = store.commit_batch(base_rows()).expect("base");
    assert_eq!(content_seq, 2);
    assert_eq!(store.derived_content_seq(), 2);

    let trailing_neutral = store.commit_batch(ledger_only_rows()).expect("ledger 2");
    assert_eq!(trailing_neutral, 3);
    assert_eq!(
        store.derived_content_seq(),
        2,
        "trailing neutral commit must leave the watermark at the last content seq"
    );
}

#[test]
fn every_non_exempt_cf_advances_derived_content_seq() {
    // Fail-closed doctrine: only Ledger and TimeIndex are content-neutral.
    for cf in ColumnFamily::STATIC {
        let store = VersionedCfStore::new(0);
        store
            .commit_batch(vec![(cf, b"k".to_vec(), b"v".to_vec())])
            .expect("commit");
        let expected = if cf.feeds_derived_search_content() {
            1
        } else {
            0
        };
        assert_eq!(
            store.derived_content_seq(),
            expected,
            "CF {} watermark classification drifted",
            cf.name()
        );
        assert_eq!(
            cf.feeds_derived_search_content(),
            !matches!(cf, ColumnFamily::Ledger | ColumnFamily::TimeIndex),
            "only Ledger and TimeIndex may be exempt; CF {} changed doctrine",
            cf.name()
        );
    }
    let slot_store = VersionedCfStore::new(0);
    slot_store
        .commit_batch(vec![(
            ColumnFamily::slot(SlotId::new(3)),
            b"k".to_vec(),
            b"v".to_vec(),
        )])
        .expect("slot commit");
    assert_eq!(slot_store.derived_content_seq(), 1);
}

#[test]
fn pinned_snapshots_carry_clamped_derived_content_seq() {
    let store = VersionedCfStore::new(0);
    store.commit_batch(base_rows()).expect("base");
    store.commit_batch(ledger_only_rows()).expect("ledger");

    let pin = store.pin_snapshot(Freshness::FreshDerived, &SystemClock, 60_000);
    assert_eq!(pin.seq(), 2);
    assert_eq!(
        pin.derived_content_seq(),
        1,
        "live pin must expose the content watermark, not the raw tip"
    );
    store.release_lease(pin.lease().id());

    // Historical pin below the live watermark: unknowable, clamp to the pin
    // seq itself (fail-closed pre-#1100 equality semantics).
    store.commit_batch(base_rows()).expect("base 2");
    let historical = store.pin_snapshot_at(1, Freshness::FreshDerived, &SystemClock, 60_000);
    assert_eq!(historical.derived_content_seq(), 1);
    store.release_lease(historical.lease().id());
}

#[test]
fn restore_batch_rederives_watermark_from_replayed_cfs() {
    let store = VersionedCfStore::new(0);
    store
        .restore_batch(4, ledger_only_rows())
        .expect("restore neutral");
    assert_eq!(store.derived_content_seq(), 0);
    store
        .restore_batch(7, base_rows())
        .expect("restore content");
    assert_eq!(store.derived_content_seq(), 7);

    store.advance_derived_content_seq_to_at_least(5);
    assert_eq!(
        store.derived_content_seq(),
        7,
        "floor advance must be monotonic"
    );
    store.advance_derived_content_seq_to_at_least(9);
    assert_eq!(store.derived_content_seq(), 9);
}
