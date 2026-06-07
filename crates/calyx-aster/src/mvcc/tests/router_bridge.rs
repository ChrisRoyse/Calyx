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
    println!(
        "ASTER_VAULT_ROUTER_FLUSH base_ssts={} slot_ssts={}",
        sst_count(dir.join("cf/base")),
        sst_count(dir.join("cf/slot_00"))
    );
    cleanup(dir);
}
