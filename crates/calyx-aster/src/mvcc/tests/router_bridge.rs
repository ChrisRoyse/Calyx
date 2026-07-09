use super::*;

#[test]
fn mvcc_store_flushes_router_rows_to_disk_and_cold_opens() {
    let dir = test_dir("router-bridge");
    let router = CfRouter::open(&dir, 1024).unwrap();
    let store = VersionedCfStore::new_with_router(0, router);
    let cx_id = cx(12);
    let seq = store
        .commit_batch([
            (ColumnFamily::Base, base_key(cx_id), b"base-disk".to_vec()),
            (
                ColumnFamily::slot(SlotId::new(0)),
                slot_key(cx_id),
                b"slot-disk".to_vec(),
            ),
        ])
        .unwrap();

    assert_eq!(seq, 1);
    assert_eq!(sst_count(dir.join("cf/base")), 0);
    let summaries = store.flush_all_cfs().unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!(sst_count(dir.join("cf/base")), 1);
    assert_eq!(sst_count(dir.join("cf/slot_00")), 1);

    let reopened = CfRouter::open(&dir, 1024).unwrap();
    assert_eq!(
        reopened.get(ColumnFamily::Base, &base_key(cx_id)).unwrap(),
        Some(b"base-disk".to_vec())
    );
    assert_eq!(
        reopened
            .get(ColumnFamily::slot(SlotId::new(0)), &slot_key(cx_id))
            .unwrap(),
        Some(b"slot-disk".to_vec())
    );
    println!("MVCC_ROUTER_FLUSH seq=1 base_ssts=1 slot_ssts=1");
    cleanup(dir);
}

#[test]
fn router_bridge_flush_edges_and_start_seq_recovery() {
    let dir = test_dir("router-edges");
    let router = CfRouter::open(&dir, 1024).unwrap();
    let store = VersionedCfStore::new_with_router(0, router);
    store.set_start_seq(41).unwrap();
    assert_eq!(
        store
            .commit_batch([(ColumnFamily::Base, b"k".to_vec(), b"v1".to_vec())])
            .unwrap(),
        42
    );
    assert_eq!(store.flush_all_cfs().unwrap().len(), 1);
    assert_eq!(
        store
            .commit_batch([(ColumnFamily::Base, b"k".to_vec(), b"v2".to_vec())])
            .unwrap(),
        43
    );
    assert_eq!(store.flush_all_cfs().unwrap().len(), 1);
    assert_eq!(sst_count(dir.join("cf/base")), 2);
    assert_eq!(
        store
            .set_start_seq(7)
            .expect_err("allocated store rejects reset")
            .code,
        "CALYX_BACKPRESSURE"
    );

    let empty = test_dir("router-empty");
    CfRouter::open(&empty, 1024).expect("cold open empty vault dir");
    cleanup(empty);
    cleanup(dir);
}

#[test]
fn latest_readback_merges_router_sst_with_wal_overlay_and_blocks_history() {
    let dir = test_dir("latest-readback");
    let writer = VersionedCfStore::new_with_router(0, CfRouter::open(&dir, 1024).unwrap());
    let flushed_seq = writer
        .commit_batch([
            (ColumnFamily::Base, b"a".to_vec(), b"sst-a".to_vec()),
            (ColumnFamily::Base, b"gone".to_vec(), b"sst-gone".to_vec()),
        ])
        .unwrap();
    assert_eq!(flushed_seq, 1);
    assert_eq!(writer.flush_all_cfs().unwrap().len(), 1);

    let physical = CfRouter::open(&dir, 1024).unwrap();
    assert_eq!(
        physical.get(ColumnFamily::Base, b"a").unwrap(),
        Some(b"sst-a".to_vec())
    );
    assert_eq!(
        physical.get(ColumnFamily::Base, b"gone").unwrap(),
        Some(b"sst-gone".to_vec())
    );

    let latest_physical =
        VersionedCfStore::new_with_router_latest_readback(1, CfRouter::open(&dir, 1024).unwrap());
    let clock = FixedClock::new(200);
    let range = KeyRange {
        start: b"a".to_vec(),
        end: None,
    };
    let physical_snapshot = latest_physical.pin_snapshot(Freshness::FreshDerived, &clock, 1_000);
    let physical_first = latest_physical
        .scan_cf_range_page_at(
            physical_snapshot,
            ColumnFamily::Base,
            &range,
            None,
            1,
            &clock,
        )
        .unwrap();
    assert_eq!(physical_first, [(b"a".to_vec(), b"sst-a".to_vec())]);
    let physical_second = latest_physical
        .scan_cf_range_page_at(
            physical_snapshot,
            ColumnFamily::Base,
            &range,
            Some(b"a"),
            1,
            &clock,
        )
        .unwrap();
    assert_eq!(physical_second, [(b"gone".to_vec(), b"sst-gone".to_vec())]);

    let latest =
        VersionedCfStore::new_with_router_latest_readback(2, CfRouter::open(&dir, 1024).unwrap());
    latest
        .restore_batch(
            2,
            [
                (ColumnFamily::Base, b"a".to_vec(), b"wal-a".to_vec()),
                (ColumnFamily::Base, b"b".to_vec(), b"wal-b".to_vec()),
                (ColumnFamily::Base, b"gone".to_vec(), tombstone_value()),
            ],
        )
        .unwrap();
    let snapshot = latest.pin_snapshot(Freshness::FreshDerived, &clock, 1_000);

    assert_eq!(
        latest
            .read_at(snapshot, ColumnFamily::Base, b"a", &clock)
            .unwrap(),
        Some(b"wal-a".to_vec())
    );
    assert_eq!(
        latest
            .read_at(snapshot, ColumnFamily::Base, b"b", &clock)
            .unwrap(),
        Some(b"wal-b".to_vec())
    );
    assert_eq!(
        latest
            .read_at(snapshot, ColumnFamily::Base, b"gone", &clock)
            .unwrap(),
        None
    );

    let range = KeyRange {
        start: b"a".to_vec(),
        end: None,
    };
    let keys = latest
        .scan_cf_range_keys_at(snapshot, ColumnFamily::Base, &range, &clock)
        .unwrap();
    assert_eq!(keys, [b"a".to_vec(), b"b".to_vec()]);
    let rows = latest
        .scan_cf_range_at(snapshot, ColumnFamily::Base, &range, &clock)
        .unwrap();
    assert_eq!(
        rows,
        [
            (b"a".to_vec(), b"wal-a".to_vec()),
            (b"b".to_vec(), b"wal-b".to_vec())
        ]
    );

    let overlay_first = latest
        .scan_cf_range_page_at(snapshot, ColumnFamily::Base, &range, None, 1, &clock)
        .unwrap();
    assert_eq!(overlay_first, [(b"a".to_vec(), b"wal-a".to_vec())]);
    let overlay_second = latest
        .scan_cf_range_page_at(snapshot, ColumnFamily::Base, &range, Some(b"a"), 1, &clock)
        .unwrap();
    assert_eq!(overlay_second, [(b"b".to_vec(), b"wal-b".to_vec())]);
    let overlay_done = latest
        .scan_cf_range_page_at(snapshot, ColumnFamily::Base, &range, Some(b"b"), 1, &clock)
        .unwrap();
    assert!(overlay_done.is_empty());
    assert_eq!(
        collect_cf_pages(&latest, snapshot, &clock, 1),
        [
            (b"a".to_vec(), b"wal-a".to_vec()),
            (b"b".to_vec(), b"wal-b".to_vec())
        ]
    );

    let historical = Snapshot::new(
        1,
        Freshness::FreshDerived,
        ReaderLease::new(0, 1, 200, 1_000),
    );
    let error = latest
        .read_at(historical, ColumnFamily::Base, b"a", &clock)
        .unwrap_err();
    assert_eq!(error.code, "CALYX_ASTER_LATEST_ONLY_HISTORY_UNAVAILABLE");
    cleanup(dir);
}

#[test]
fn scan_cf_pages_matches_materialized_scan_and_holds_snapshot() {
    let store = VersionedCfStore::default();
    let clock = FixedClock::new(100);
    store
        .commit_batch([
            (ColumnFamily::Base, b"a".to_vec(), b"v1-a".to_vec()),
            (ColumnFamily::Base, b"c".to_vec(), b"v1-c".to_vec()),
        ])
        .unwrap();
    let snapshot = store.pin_snapshot(Freshness::FreshDerived, &clock, 1_000);
    store
        .commit_batch([
            (ColumnFamily::Base, b"b".to_vec(), b"v2-b".to_vec()),
            (ColumnFamily::Base, b"c".to_vec(), b"v2-c".to_vec()),
        ])
        .unwrap();

    let materialized = store
        .scan_cf_at(snapshot, ColumnFamily::Base, &clock)
        .unwrap();
    let streamed = collect_cf_pages(&store, snapshot, &clock, 1);

    assert_eq!(
        materialized,
        [
            (b"a".to_vec(), b"v1-a".to_vec()),
            (b"c".to_vec(), b"v1-c".to_vec())
        ]
    );
    assert_eq!(streamed, materialized);
}

#[test]
fn scan_cf_pages_never_emits_more_than_limit() {
    let store = VersionedCfStore::default();
    let clock = FixedClock::new(100);
    let rows = (0..37)
        .map(|index| {
            (
                ColumnFamily::Base,
                format!("k{index:03}").into_bytes(),
                format!("v{index:03}").into_bytes(),
            )
        })
        .collect::<Vec<_>>();
    store.commit_batch(rows).unwrap();
    let snapshot = store.pin_snapshot(Freshness::FreshDerived, &clock, 1_000);

    let mut page_lengths = Vec::new();
    let mut streamed = Vec::new();
    store
        .scan_cf_pages_at(snapshot, ColumnFamily::Base, 7, &clock, |page| {
            page_lengths.push(page.len());
            streamed.extend(page);
            Ok::<(), calyx_core::CalyxError>(())
        })
        .unwrap();

    assert!(page_lengths.iter().all(|len| *len <= 7));
    assert!(page_lengths.len() > 1);
    assert_eq!(streamed.len(), 37);
    assert_eq!(
        streamed,
        store
            .scan_cf_at(snapshot, ColumnFamily::Base, &clock)
            .unwrap()
    );
}

#[test]
fn scan_cf_pages_matches_router_sst_only_scan() {
    let dir = test_dir("router-page-sst-only");
    let writer = VersionedCfStore::new_with_router(0, CfRouter::open(&dir, 1024).unwrap());
    writer
        .commit_batch([
            (ColumnFamily::Base, b"a".to_vec(), b"sst-a".to_vec()),
            (ColumnFamily::Base, b"b".to_vec(), b"sst-b".to_vec()),
            (ColumnFamily::Base, b"c".to_vec(), b"sst-c".to_vec()),
        ])
        .unwrap();
    writer.flush_all_cfs().unwrap();

    let store =
        VersionedCfStore::new_with_router_latest_readback(1, CfRouter::open(&dir, 1024).unwrap());
    let clock = FixedClock::new(100);
    let snapshot = store.pin_snapshot(Freshness::FreshDerived, &clock, 1_000);
    let materialized = store
        .scan_cf_at(snapshot, ColumnFamily::Base, &clock)
        .unwrap();
    let streamed = collect_cf_pages(&store, snapshot, &clock, 2);

    assert_eq!(streamed, materialized);
    assert_eq!(streamed.len(), 3);
    cleanup(dir);
}

#[test]
fn aster_vault_put_flushes_through_router_to_cf_ssts() {
    let dir = test_dir("vault-router");
    let router = CfRouter::open(&dir, 2048).unwrap();
    let vault_id = vault_id();
    let vault = AsterVault::with_clock_and_router(
        vault_id,
        b"mvcc-router-salt".to_vec(),
        FixedClock::new(100),
        router,
    );
    let cx = sample_constellation(vault_id);
    let id = cx.cx_id;

    vault.put(cx).expect("put constellation");
    let summaries = vault.flush_all_cfs().expect("flush router CFs");
    assert!(summaries.len() >= 3);

    let reopened = CfRouter::open(&dir, 2048).unwrap();
    assert!(
        reopened
            .get(ColumnFamily::Base, &base_key(id))
            .unwrap()
            .is_some()
    );
    assert!(
        reopened
            .get(ColumnFamily::slot(SlotId::new(0)), &slot_key(id))
            .unwrap()
            .is_some()
    );
    let mut streamed = Vec::new();
    vault
        .scan_cf_pages_at(vault.latest_seq(), ColumnFamily::Base, 1, |page| {
            streamed.extend(page);
            Ok::<(), calyx_core::CalyxError>(())
        })
        .unwrap();
    assert_eq!(streamed.len(), 1);
    println!(
        "ASTER_VAULT_ROUTER_FLUSH base_ssts={} slot_ssts={}",
        sst_count(dir.join("cf/base")),
        sst_count(dir.join("cf/slot_00"))
    );
    cleanup(dir);
}

fn collect_cf_pages(
    store: &VersionedCfStore,
    snapshot: Snapshot,
    clock: &FixedClock,
    limit: usize,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut rows = Vec::new();
    store
        .scan_cf_pages_at(snapshot, ColumnFamily::Base, limit, clock, |page| {
            rows.extend(page);
            Ok::<(), calyx_core::CalyxError>(())
        })
        .unwrap();
    rows
}
