use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use calyx_anneal::{
    AsterHeadStorage, CALYX_ANNEAL_HEAD_TOO_LARGE, HeadKind, OnlineHead, OnlineHeadState,
    ReplayEntry, decode_online_head,
};
use calyx_aster::cf::{ColumnFamily, full_content_hash};
use calyx_aster::vault::AsterVault;
use calyx_core::{CxId, FixedClock};
use serde_json::{Value, json};

#[path = "fsv_support/mod.rs"]
mod fsv_support;
#[allow(dead_code)]
#[path = "support/fsv_bad_change.rs"]
mod support;
use fsv_support::write_json;

const TEST_TS: u64 = 1_785_500_408;

#[test]
#[ignore = "requires CALYX_ISSUE408_FSV_ROOT in a manual verification run"]
fn fsv_online_head_manual() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE408_FSV_ROOT").expect("set CALYX_ISSUE408_FSV_ROOT"));
    support::reset_dir(&root);
    let (vault_dir, vault) = support::open_durable_vault(&root, "vault");
    let base_hash_before = cf_hash(&vault, ColumnFamily::Base);
    let slot_hash_before = cf_hash(&vault, ColumnFamily::slot(calyx_core::SlotId::new(0)));
    let before_rows = raw_cf_rows(&vault, ColumnFamily::AnnealHeads);

    let clock_revert = FixedClock::new(TEST_TS);
    let substrate_revert = support::durable_substrate_with_budget(
        &clock_revert,
        &vault,
        &vault_dir,
        support::budget_config(0.0),
    );
    let mut state = OnlineHeadState::open(
        AsterHeadStorage::new(&vault),
        substrate_revert,
        Arc::new(FixedClock::new(TEST_TS)),
        [zero_predictor()],
    )
    .unwrap();

    let empty = state.update(&[], 0.01, 1.0).unwrap();
    let rows_after_empty = raw_cf_rows(&vault, ColumnFamily::AnnealHeads);
    let lr_zero = state.update(&[entry(1.0, 1)], 0.0, 1.0).unwrap();
    let rows_after_lr_zero = raw_cf_rows(&vault, ColumnFamily::AnnealHeads);
    let too_large_error = OnlineHead::new(HeadKind::Predictor, vec![0.0; 1025])
        .unwrap_err()
        .code
        .to_string();
    assert_eq!(too_large_error, CALYX_ANNEAL_HEAD_TOO_LARGE);
    let invalid_lr_error = state
        .update(&[entry(1.0, 2)], f32::NAN, 1.0)
        .unwrap_err()
        .code
        .to_string();
    let reverted_error = state
        .update(&[entry(1.0, 3)], 0.01, 0.0)
        .unwrap_err()
        .code
        .to_string();
    let rows_after_revert = raw_cf_rows(&vault, ColumnFamily::AnnealHeads);
    drop(state);

    let clock_promote = FixedClock::new(TEST_TS + 1);
    let substrate_promote = support::durable_substrate(&clock_promote, &vault, &vault_dir);
    let mut state = OnlineHeadState::open(
        AsterHeadStorage::new(&vault),
        substrate_promote,
        Arc::new(FixedClock::new(TEST_TS + 1)),
        [zero_predictor()],
    )
    .unwrap();
    let promoted = state.update(&[entry(1.0, 4)], 0.01, 0.0).unwrap();
    vault.flush().unwrap();

    let after_rows = raw_cf_rows(&vault, ColumnFamily::AnnealHeads);
    let decoded_after = decoded_head_rows(&vault);
    let base_hash_after = cf_hash(&vault, ColumnFamily::Base);
    let slot_hash_after = cf_hash(&vault, ColumnFamily::slot(calyx_core::SlotId::new(0)));
    let ledger_rows = support::read_ledger_rows(&vault);
    let rollback_rows = support::read_rollback_rows(&vault);
    let artifact = json!({
        "issue": 408,
        "source_of_truth": "Aster anneal_heads CF rows plus WAL/SST bytes under the durable vault",
        "vault": vault_dir.display().to_string(),
        "trigger": "high-surprise ReplayEntry surprise=1.0, lr=0.01",
        "expected_first_delta": 0.01,
        "before": {
            "head_rows": before_rows,
            "base_cf_hash": base_hash_before,
            "slot_00_cf_hash": slot_hash_before,
        },
        "edges": {
            "empty_batch": {
                "promoted": empty.promoted,
                "version_after": state_version(&empty),
                "rows_after": rows_after_empty,
            },
            "lr_zero": {
                "promoted": lr_zero.promoted,
                "version_after": state_version(&lr_zero),
                "rows_after": rows_after_lr_zero,
            },
            "too_large_error": too_large_error,
            "invalid_lr_error": invalid_lr_error,
            "budget_revert_error": reverted_error,
            "rows_after_revert": rows_after_revert,
        },
        "after": {
            "promoted": promoted.promoted,
            "change_id": promoted.change_id.map(|id| id.0),
            "head_rows": after_rows,
            "decoded_heads": decoded_after,
            "base_cf_hash": base_hash_after,
            "slot_00_cf_hash": slot_hash_after,
            "ledger_rows": ledger_rows,
            "rollback_rows": rollback_rows,
        }
    });
    assert!(before_rows.is_empty());
    assert!(rows_after_empty.is_empty());
    assert!(rows_after_lr_zero.is_empty());
    assert!(rows_after_revert.is_empty());
    assert_eq!(base_hash_before, base_hash_after);
    assert_eq!(slot_hash_before, slot_hash_after);
    assert_eq!(decoded_after[0]["head"]["version"], 1);
    let observed_param = decoded_after[0]["head"]["params"][0]
        .as_f64()
        .expect("decoded predictor param must be numeric");
    assert!((observed_param - 0.01).abs() < 0.000001);
    write_json(&root.join("issue408-fsv-artifact.json"), &artifact);
}

fn zero_predictor() -> OnlineHead {
    OnlineHead::new(HeadKind::Predictor, vec![0.0]).unwrap()
}

fn entry(surprise: f64, seq: u64) -> ReplayEntry {
    ReplayEntry::new(
        CxId::from_bytes([seq as u8; 16]),
        surprise,
        calyx_anneal::MistakeRef { seq, surprise },
        TEST_TS,
    )
    .unwrap()
}

fn state_version(outcome: &calyx_anneal::HeadUpdateOutcome) -> u64 {
    outcome
        .heads
        .iter()
        .find(|head| head.kind == HeadKind::Predictor)
        .map_or(0, |head| head.version)
}

fn decoded_head_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealHeads)
        .unwrap()
        .into_iter()
        .map(|(key, bytes)| {
            let head = decode_online_head(&bytes).unwrap();
            json!({
                "key_hex": hex(&key),
                "value_len": bytes.len(),
                "value_hex": hex(&bytes),
                "head": head,
            })
        })
        .collect()
}

fn raw_cf_rows(vault: &AsterVault, cf: ColumnFamily) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, bytes)| json!({"key_hex": hex(&key), "value_hex": hex(&bytes)}))
        .collect()
}

fn cf_hash(vault: &AsterVault, cf: ColumnFamily) -> String {
    let mut parts = Vec::new();
    for (key, value) in vault.scan_cf_at(vault.latest_seq(), cf).unwrap() {
        parts.push(key);
        parts.push(value);
    }
    hex(&full_content_hash(parts.iter().map(Vec::as_slice)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
