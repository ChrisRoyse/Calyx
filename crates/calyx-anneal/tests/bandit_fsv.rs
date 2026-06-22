use std::env;
use std::fs;
use std::path::PathBuf;

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use calyx_anneal::{
    AsterBanditStorage, BanditPolicy, BanditStorage, CALYX_ANNEAL_BANDIT_EMPTY,
    CALYX_ANNEAL_BANDIT_INVALID_CONFIG, ConfigBandit, bandit_key, encode_config_bandit,
    shape_key_hash,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use fsv_support::{hex_bytes, parse_vault_id, write_json};
use serde_json::json;

const SHAPE_KEY: &str = "issue412:forge:gemm:768x768:fp16:cuda:recall0.99";

#[test]
#[ignore = "requires CALYX_ISSUE412_FSV_ROOT in a manual verification run"]
fn issue412_bandit_cf_fsv() {
    let root = PathBuf::from(env::var("CALYX_ISSUE412_FSV_ROOT").expect("set FSV root"));
    fs::create_dir_all(&root).expect("create FSV root");
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue412-bandit".to_vec(),
        VaultOptions::default(),
    )
    .expect("open vault");
    let hash = shape_key_hash(SHAPE_KEY);
    let before_rows = scan_bandit_cf(&vault);

    let mut bandit =
        ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 412).with_hysteresis(3);
    bandit.add_arm(b"arm0_latency_120us_recall_099".to_vec());
    bandit.add_arm(b"arm1_latency_080us_recall_099".to_vec());
    let schedule = run_known_schedule(&mut bandit);
    assert_eq!(bandit.incumbent_idx, 1);
    assert!(bandit.arms[1].win_rate() > bandit.arms[0].win_rate());

    {
        let storage = AsterBanditStorage::new(&vault);
        storage
            .save(bandit_key(hash), encode_config_bandit(&bandit).unwrap())
            .expect("save bandit");
    }
    vault.flush().expect("flush bandit CF");
    let after_rows = scan_bandit_cf(&vault);
    assert_eq!(after_rows.len(), before_rows.len() + 1);
    drop(vault);

    let reopened = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue412-bandit".to_vec(),
        VaultOptions::default(),
    )
    .expect("reopen vault");
    let reopened_value = reopened
        .read_cf_at(
            reopened.latest_seq(),
            ColumnFamily::AnnealBandit,
            &bandit_key(hash),
        )
        .unwrap()
        .expect("bandit row exists");
    let reopened_bandit = calyx_anneal::decode_config_bandit(&reopened_value).unwrap();
    assert_eq!(reopened_bandit.incumbent_idx, 1);
    let edges = edge_cases(&reopened);
    write_json(
        &root.join("bandit-readback.json"),
        &json!({
            "surface": "anneal.config_bandit",
            "source_of_truth": "Aster CF anneal_bandit row plus WAL/SST under vault/",
            "vault": vault_dir,
            "shape_key": SHAPE_KEY,
            "shape_key_hash": hex_bytes(&hash),
            "trigger": "50 deterministic A/B outcomes -> save ConfigBandit to anneal_bandit CF",
            "expected": "incumbent=1 and arm1 win_rate > arm0 win_rate after reload",
            "before_rows": before_rows,
            "after_rows": after_rows,
            "schedule": schedule,
            "status_after_reload": reopened_bandit.status(hash).unwrap(),
            "row_key_hex": hex_bytes(&bandit_key(hash)),
            "row_value_hex": hex_bytes(&reopened_value),
            "edges": edges,
        }),
    );
}

fn run_known_schedule(bandit: &mut ConfigBandit) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();
    for round in 0..50 {
        let arm_idx = round % 2;
        let won = if arm_idx == 0 {
            round % 10 == 0
        } else {
            round % 10 != 1
        };
        bandit.record_result(arm_idx, won).unwrap();
        rows.push(json!({
            "round": round,
            "arm_idx": arm_idx,
            "won": won,
            "incumbent_after": bandit.incumbent_idx,
        }));
    }
    rows
}

fn edge_cases<C>(vault: &AsterVault<C>) -> Vec<serde_json::Value>
where
    C: calyx_core::Clock,
{
    let before = scan_bandit_cf(vault);
    let mut empty = ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 1);
    let empty_err = empty.select_arm().unwrap_err();
    let after_empty = scan_bandit_cf(vault);
    assert_eq!(empty_err.code, CALYX_ANNEAL_BANDIT_EMPTY);
    assert_eq!(before, after_empty);

    let mut zero = ConfigBandit::new(BanditPolicy::Thompson, 2).with_hysteresis(0);
    zero.add_arm(vec![0]);
    zero.add_arm(vec![1]);
    zero.record_result(1, true).unwrap();
    assert_eq!(zero.incumbent_idx, 1);

    let before_invalid = zero.clone();
    let invalid_err = zero.record_result(9, true).unwrap_err();
    assert_eq!(invalid_err.code, CALYX_ANNEAL_BANDIT_INVALID_CONFIG);
    assert_eq!(zero, before_invalid);

    vec![
        json!({
            "case": "zero_arms_select",
            "expected": CALYX_ANNEAL_BANDIT_EMPTY,
            "actual_code": empty_err.code,
            "before_rows": before,
            "after_rows": after_empty,
        }),
        json!({
            "case": "zero_hysteresis_first_win_promotes",
            "expected_incumbent": 1,
            "after_incumbent": zero.incumbent_idx,
            "arm1_wins": zero.arms[1].wins,
            "arm1_trials": zero.arms[1].trials,
        }),
        json!({
            "case": "invalid_arm_index_no_state_change",
            "expected": CALYX_ANNEAL_BANDIT_INVALID_CONFIG,
            "actual_code": invalid_err.code,
            "before_state": before_invalid,
            "after_state": zero,
        }),
    ]
}

fn scan_bandit_cf<C>(vault: &AsterVault<C>) -> Vec<serde_json::Value>
where
    C: calyx_core::Clock,
{
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealBandit)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            json!({
                "key_hex": hex_bytes(&key),
                "value_hex": hex_bytes(&value),
                "value_len": value.len(),
            })
        })
        .collect()
}

fn vault_id() -> calyx_core::VaultId {
    parse_vault_id("01J00000000000000000000412")
}
