use super::*;
use crate::cf::{ColumnFamily, KeyRange};

#[test]
fn read_barrier_blocks_point_batch_and_scan_reads_until_removed() {
    let clock = FixedClock::new(100);
    let store = VersionedCfStore::default();
    let blocked = base_key(cx(0x10));
    let outside = base_key(cx(0x20));
    let seq = store
        .commit_batch([
            (ColumnFamily::Base, blocked.clone(), b"blocked".to_vec()),
            (ColumnFamily::Base, outside.clone(), b"outside".to_vec()),
        ])
        .unwrap();
    let snapshot = Snapshot::new(
        seq,
        Freshness::FreshDerived,
        ReaderLease::new(1, seq, 100, 1_000),
    );
    let range = KeyRange {
        start: blocked.clone(),
        end: Some(base_key(cx(0x11))),
    };

    store.install_read_barrier(ReadBarrier::base_corrupt("shard_10", range));

    let error = store
        .read_at(snapshot, ColumnFamily::Base, &blocked, &clock)
        .expect_err("blocked point read fails closed");
    assert_eq!(error.code, CALYX_ASTER_BASE_CORRUPT);
    assert_eq!(
        store
            .read_at(snapshot, ColumnFamily::Base, &outside, &clock)
            .unwrap(),
        Some(b"outside".to_vec())
    );
    assert_eq!(
        store
            .read_batch(
                snapshot,
                &[CfRead::new(ColumnFamily::Base, blocked.clone())],
                &clock,
            )
            .expect_err("blocked batch fails")
            .code,
        CALYX_ASTER_BASE_CORRUPT
    );
    assert_eq!(
        store
            .scan_cf_at(snapshot, ColumnFamily::Base, &clock)
            .expect_err("blocked scan fails")
            .code,
        CALYX_ASTER_BASE_CORRUPT
    );

    assert!(store.remove_read_barrier("shard_10"));
    assert_eq!(
        store
            .read_at(snapshot, ColumnFamily::Base, &blocked, &clock)
            .unwrap(),
        Some(b"blocked".to_vec())
    );
}
