use super::*;
use std::sync::{Arc, Barrier};
use std::thread;

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
    println!("MVCC_SNAPSHOT_ISOLATION 1000/1000 iterations: no partial read");
}

#[test]
fn barrier_snapshot_reader_sees_none_before_batch_and_all_after() {
    let store = Arc::new(VersionedCfStore::default());
    let barrier = Arc::new(Barrier::new(2));
    let cx_id = cx(10);
    let reads = read_pair(cx_id);
    let reader_store = Arc::clone(&store);
    let reader_barrier = Arc::clone(&barrier);

    let reader = thread::spawn(move || {
        let clock = FixedClock::new(100);
        let before = reader_store.pin_snapshot(Freshness::FreshDerived, &clock, 10);
        reader_barrier.wait();
        let rows = reader_store
            .read_batch(before, &reads, &clock)
            .expect("read before batch");
        assert_eq!(rows, [None, None]);
    });

    let writer_store = Arc::clone(&store);
    let writer = thread::spawn(move || {
        barrier.wait();
        writer_store
            .commit_batch([
                (ColumnFamily::Base, base_key(cx_id), b"base-v1".to_vec()),
                (
                    ColumnFamily::slot(SlotId::new(0)),
                    slot_key(cx_id),
                    b"slot-v1".to_vec(),
                ),
            ])
            .expect("commit batch");
    });

    reader.join().expect("reader joins");
    writer.join().expect("writer joins");
    let clock = FixedClock::new(100);
    let after = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);
    assert_eq!(
        store.read_batch(after, &read_pair(cx_id), &clock).unwrap(),
        [Some(b"base-v1".to_vec()), Some(b"slot-v1".to_vec())]
    );
}

#[test]
fn commit_batch_edges_are_atomic_at_one_sequence() {
    let store = VersionedCfStore::default();
    let clock = FixedClock::new(100);
    assert_eq!(
        store
            .commit_batch(Vec::<(ColumnFamily, Vec<u8>, Vec<u8>)>::new())
            .unwrap(),
        0
    );

    let cx_id = cx(11);
    let seq1 = store
        .commit_batch([(ColumnFamily::Base, base_key(cx_id), b"only-base".to_vec())])
        .unwrap();
    let snap1 = Snapshot::new(
        seq1,
        Freshness::FreshDerived,
        ReaderLease::new(0, seq1, 100, 10),
    );
    assert_eq!(
        store
            .read_at(snap1, ColumnFamily::Base, &base_key(cx_id), &clock)
            .unwrap(),
        Some(b"only-base".to_vec())
    );

    let rows = (0..10)
        .map(|index| {
            let cf = match index % 5 {
                0 => ColumnFamily::Base,
                1 => ColumnFamily::slot(SlotId::new(0)),
                2 => ColumnFamily::slot(SlotId::new(1)),
                3 => ColumnFamily::Ledger,
                _ => ColumnFamily::Online,
            };
            (cf, vec![index as u8], vec![index as u8, 0xaa])
        })
        .collect::<Vec<_>>();
    let reads = rows
        .iter()
        .map(|(cf, key, _)| CfRead::new(*cf, key.clone()))
        .collect::<Vec<_>>();
    let before = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);
    store.commit_batch(rows.clone()).unwrap();
    let after = store.pin_snapshot(Freshness::FreshDerived, &clock, 10);

    assert!(
        store
            .read_batch(before, &reads, &clock)
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
    assert_eq!(
        store.read_batch(after, &reads, &clock).unwrap(),
        rows.into_iter()
            .map(|(_, _, value)| Some(value))
            .collect::<Vec<_>>()
    );
}
