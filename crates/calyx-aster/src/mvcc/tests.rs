use super::*;
use crate::cf::{ColumnFamily, base_key, slot_key};
use calyx_core::{CxId, FixedClock, SlotId};
use std::sync::Arc;
use std::thread;

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn read_pair(cx_id: CxId) -> [CfRead; 2] {
    [
        CfRead::new(ColumnFamily::Base, base_key(cx_id)),
        CfRead::new(ColumnFamily::slot(SlotId::new(0)), slot_key(cx_id)),
    ]
}

#[test]
fn allocator_and_snapshot_pin_latest_committed_sequence() {
    let store = VersionedCfStore::default();
    let clock = FixedClock::new(100);

    let initial = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);
    assert_eq!(initial.seq(), 0);
    assert_eq!(initial.lease().pinned_seq(), 0);

    let seq = store
        .commit_batch([(ColumnFamily::Ledger, vec![0], b"ledger-v1".to_vec())])
        .expect("commit");
    let after = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);

    assert_eq!(seq, 1);
    assert_eq!(store.current_seq(), 1);
    assert_eq!(after.seq(), 1);
}

#[test]
fn snapshot_reads_resolve_all_cfs_at_one_sequence() {
    let store = VersionedCfStore::default();
    let clock = FixedClock::new(100);
    let cx_id = cx(3);
    let reads = read_pair(cx_id);
    let before = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);

    store
        .commit_batch([
            (ColumnFamily::Base, base_key(cx_id), b"base-v1".to_vec()),
            (
                ColumnFamily::slot(SlotId::new(0)),
                slot_key(cx_id),
                b"slot-v1".to_vec(),
            ),
        ])
        .expect("commit v1");
    let after_v1 = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);

    store
        .commit_batch([
            (ColumnFamily::Base, base_key(cx_id), b"base-v2".to_vec()),
            (
                ColumnFamily::slot(SlotId::new(0)),
                slot_key(cx_id),
                b"slot-v2".to_vec(),
            ),
        ])
        .expect("commit v2");
    let after_v2 = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);

    assert_eq!(
        store.read_batch(before, &reads, &clock).unwrap(),
        [None, None]
    );
    assert_eq!(
        store.read_batch(after_v1, &reads, &clock).unwrap(),
        [Some(b"base-v1".to_vec()), Some(b"slot-v1".to_vec())]
    );
    assert_eq!(
        store.read_batch(after_v2, &reads, &clock).unwrap(),
        [Some(b"base-v2".to_vec()), Some(b"slot-v2".to_vec())]
    );
}

#[test]
fn freshness_policy_fails_closed_when_derived_is_too_old() {
    Freshness::FreshDerived
        .ensure(10, 10)
        .expect("same seq is fresh");
    Freshness::StaleOk { max_lag: 2 }
        .ensure(10, 8)
        .expect("bounded lag accepted");

    let fresh_error = Freshness::FreshDerived
        .ensure(10, 9)
        .expect_err("fresh required");
    let stale_error = Freshness::StaleOk { max_lag: 2 }
        .ensure(10, 7)
        .expect_err("lag too large");

    assert_eq!(fresh_error.code, "CALYX_STALE_DERIVED");
    assert_eq!(stale_error.code, "CALYX_STALE_DERIVED");
}

#[test]
fn reader_lease_expiration_fails_closed() {
    let lease = ReaderLease::new(1, 7, 100, 5);
    let live = FixedClock::new(105);
    let expired = FixedClock::new(106);

    lease.ensure_live(&live).expect("lease still live");
    let error = lease.ensure_live(&expired).expect_err("lease expired");

    assert_eq!(error.code, "CALYX_READER_LEASE_EXPIRED");
}

#[test]
fn concurrent_reader_never_observes_partial_constellation() {
    let store = Arc::new(VersionedCfStore::default());
    let cx_id = cx(9);
    let reads = read_pair(cx_id);
    let writer = Arc::clone(&store);

    let writer_thread = thread::spawn(move || {
        for seq in 1..=200_u64 {
            writer
                .commit_batch([
                    (
                        ColumnFamily::Base,
                        base_key(cx_id),
                        format!("base-{seq}").into_bytes(),
                    ),
                    (
                        ColumnFamily::slot(SlotId::new(0)),
                        slot_key(cx_id),
                        format!("slot-{seq}").into_bytes(),
                    ),
                ])
                .expect("commit batch");
        }
    });

    let reader_thread = thread::spawn(move || {
        let clock = FixedClock::new(100);
        for _ in 0..1_000 {
            let snapshot = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);
            let rows = store
                .read_batch(snapshot, &reads, &clock)
                .expect("read batch");
            match (&rows[0], &rows[1]) {
                (None, None) => {}
                (Some(base), Some(slot)) => {
                    let base = std::str::from_utf8(base).expect("base utf8");
                    let slot = std::str::from_utf8(slot).expect("slot utf8");
                    assert_eq!(
                        base.strip_prefix("base-"),
                        slot.strip_prefix("slot-"),
                        "snapshot {} saw mismatched CF versions",
                        snapshot.seq()
                    );
                }
                other => panic!(
                    "partial constellation at snapshot {}: {other:?}",
                    snapshot.seq()
                ),
            }
        }
    });

    writer_thread.join().expect("writer joins");
    reader_thread.join().expect("reader joins");
}
