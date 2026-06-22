use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use calyx_anneal::{
    ArtifactKey, ArtifactPtr, AsterRollbackStorage, CALYX_ANNEAL_CHANGE_COMMITTED,
    CALYX_ANNEAL_INVALID_ROLLBACK_STATE, CALYX_ANNEAL_UNKNOWN_CHANGE_ID, ChangeId, RollbackStore,
    rollback_live_key,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::FixedClock;
use fsv_support::{hex_bytes, vault_id, write_json, write_manifest};
use serde_json::json;

const FSV_TS: u64 = 1_785_500_396;

#[test]
#[ignore = "requires CALYX_ISSUE396_FSV_ROOT in a manual verification run"]
fn issue396_rollback_store_fsv() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE396_FSV_ROOT").expect("set CALYX_ISSUE396_FSV_ROOT"));
    fs::create_dir_all(&root).expect("create FSV root");
    let vault_dir = root.join("vault");
    let clock = FixedClock::new(FSV_TS);
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue396-rollback".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");

    let storage = AsterRollbackStorage::new(&vault);
    let store = RollbackStore::open(&clock, 396, storage).expect("open rollback store");
    let key = ArtifactKey::HnswGraph([0x11; 32]);
    let prior =
        ArtifactPtr::HnswGraphPath("/var/lib/calyx/data/fsv-issue396/prior-hnsw.graph".to_string());
    let candidate = ArtifactPtr::HnswGraphPath(
        "/var/lib/calyx/data/fsv-issue396/candidate-hnsw.graph".to_string(),
    );

    store
        .install_live_ptr(key.clone(), prior.clone())
        .expect("install prior live pointer");
    let live_before = read_cf_row(&vault, &rollback_live_key(&key));
    let change_id = store
        .prepare_with_description(key.clone(), candidate.clone(), "issue396 happy path")
        .expect("prepare rollback snapshot");
    let after_prepare = store.readback(change_id).expect("readback prepare");
    store.promote(change_id).expect("promote candidate");
    let after_promote = store.readback(change_id).expect("readback promote");
    assert_eq!(after_promote.live_ptr, candidate);
    store.rollback(change_id).expect("rollback to prior");
    let after_rollback = store.readback(change_id).expect("readback rollback");
    assert_eq!(after_rollback.live_ptr, prior);
    assert!(after_rollback.snapshot.reverted);

    let unknown_edge = edge_unknown_change(&store, &vault);
    let missing_live_edge = edge_missing_live(&store, &vault);
    let committed_edge = edge_committed(&store, &vault);
    vault.flush().expect("flush rollback CF");
    drop(store);
    let reopened = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue396-rollback".to_vec(),
        VaultOptions::default(),
    )
    .expect("reopen durable vault");
    let reopened_store =
        RollbackStore::open(&clock, 396, AsterRollbackStorage::new(&reopened)).expect("reopen");
    let reopened_readback = reopened_store
        .readback(change_id)
        .expect("reopened readback");
    assert_eq!(reopened_readback.live_ptr, after_rollback.live_ptr);

    write_bytes(&root.join("live-before.bin"), &live_before);
    write_bytes(
        &root.join("snapshot-after-prepare.bin"),
        &after_prepare.snapshot_bytes,
    );
    write_bytes(
        &root.join("live-after-promote.bin"),
        &after_promote.live_bytes,
    );
    write_bytes(
        &root.join("snapshot-after-rollback.bin"),
        &after_rollback.snapshot_bytes,
    );
    write_json(
        &root.join("rollback-readback.json"),
        &json!({
            "surface": "anneal.rollback_store",
            "source_of_truth": "Aster CF anneal_rollback rows plus durable WAL/SST under vault/",
            "vault": vault_dir,
            "trigger": "install_live_ptr -> prepare -> promote -> rollback",
            "expected": "live pointer is candidate after promote and prior after rollback",
            "change_id": change_id.0,
            "snapshot_key_hex": hex_bytes(&after_rollback.snapshot_key),
            "live_key_hex": hex_bytes(&after_rollback.live_key),
            "live_before_hex": hex_bytes(&live_before),
            "after_prepare": readback_json(&after_prepare),
            "after_promote": readback_json(&after_promote),
            "after_rollback": readback_json(&after_rollback),
            "reopened_readback": readback_json(&reopened_readback),
            "edges": [unknown_edge, missing_live_edge, committed_edge]
        }),
    );
    write_manifest(
        &root,
        &[
            root.join("live-before.bin"),
            root.join("snapshot-after-prepare.bin"),
            root.join("live-after-promote.bin"),
            root.join("snapshot-after-rollback.bin"),
            root.join("rollback-readback.json"),
        ],
    );
}

fn edge_unknown_change<S>(store: &RollbackStore<'_, S>, vault: &AsterVault) -> serde_json::Value
where
    S: calyx_anneal::RollbackStorage,
{
    let before = scan_cf(vault);
    let err = store
        .rollback(ChangeId(9_999_999))
        .expect_err("unknown id fails");
    let after = scan_cf(vault);
    assert_eq!(err.code, CALYX_ANNEAL_UNKNOWN_CHANGE_ID);
    assert_eq!(before, after);
    json!({
        "case": "unknown_change_id",
        "expected": CALYX_ANNEAL_UNKNOWN_CHANGE_ID,
        "before_rows": before,
        "after_rows": after,
        "actual_code": err.code
    })
}

fn edge_missing_live<S>(store: &RollbackStore<'_, S>, vault: &AsterVault) -> serde_json::Value
where
    S: calyx_anneal::RollbackStorage,
{
    let before = scan_cf(vault);
    let err = store
        .prepare(
            ArtifactKey::QuantLevel([0x77; 32]),
            ArtifactPtr::QuantLevelRecordHash([0x88; 32]),
        )
        .expect_err("missing live ptr fails");
    let after = scan_cf(vault);
    assert_eq!(err.code, CALYX_ANNEAL_INVALID_ROLLBACK_STATE);
    assert_eq!(before, after);
    json!({
        "case": "missing_live_pointer",
        "expected": CALYX_ANNEAL_INVALID_ROLLBACK_STATE,
        "before_rows": before,
        "after_rows": after,
        "actual_code": err.code
    })
}

fn edge_committed<S>(store: &RollbackStore<'_, S>, vault: &AsterVault) -> serde_json::Value
where
    S: calyx_anneal::RollbackStorage,
{
    let key = ArtifactKey::ConfigCache([0x44; 32]);
    store
        .install_live_ptr(key.clone(), ArtifactPtr::ConfigCacheKeyHash([0x45; 32]))
        .expect("edge live ptr");
    let id = store
        .prepare(key, ArtifactPtr::ConfigCacheKeyHash([0x46; 32]))
        .expect("edge prepare");
    store.promote(id).expect("edge promote");
    store.commit(id).expect("edge commit");
    let before = scan_cf(vault);
    let err = store.rollback(id).expect_err("committed rollback fails");
    let after = scan_cf(vault);
    assert_eq!(err.code, CALYX_ANNEAL_CHANGE_COMMITTED);
    assert_eq!(before, after);
    json!({
        "case": "committed_change",
        "change_id": id.0,
        "expected": CALYX_ANNEAL_CHANGE_COMMITTED,
        "before_rows": before,
        "after_rows": after,
        "actual_code": err.code
    })
}

fn readback_json(readback: &calyx_anneal::RollbackReadback) -> serde_json::Value {
    json!({
        "snapshot": readback.snapshot,
        "live_ptr": readback.live_ptr,
        "snapshot_key_hex": hex_bytes(&readback.snapshot_key),
        "snapshot_bytes_hex": hex_bytes(&readback.snapshot_bytes),
        "live_key_hex": hex_bytes(&readback.live_key),
        "live_bytes_hex": hex_bytes(&readback.live_bytes)
    })
}

fn read_cf_row(vault: &AsterVault, key: &[u8]) -> Vec<u8> {
    vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::AnnealRollback, key)
        .expect("read rollback CF")
        .expect("rollback CF row exists")
}

fn scan_cf(vault: &AsterVault) -> Vec<serde_json::Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealRollback)
        .expect("scan rollback CF")
        .into_iter()
        .map(|(key, value)| json!({"key_hex": hex_bytes(&key), "value_hex": hex_bytes(&value)}))
        .collect()
}

fn write_bytes(path: &Path, value: &[u8]) {
    fs::write(path, value).expect("write binary artifact");
}
