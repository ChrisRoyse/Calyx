use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    AsterAnnealLedgerStore, AsterBanditStorage, CALYX_FORGE_CACHE_WRITE_FAIL, ConfigBanditStore,
    DType, ForgeConfig, ForgeScopeTuner, ShapeKey, decode_config_bandit,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::FixedClock;
use calyx_forge::AutotuneCache;
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use serde_json::{Value, json};

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use fsv_support::write_json;

const FSV_TS: u64 = 1_785_500_413;

#[test]
#[ignore = "requires CALYX_ISSUE413_FSV_ROOT in a manual verification run"]
fn issue413_forge_scope_tuner_fsv() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE413_FSV_ROOT").expect("set CALYX_ISSUE413_FSV_ROOT"));
    reset_dir(&root);
    fs::create_dir_all(&root).expect("create FSV root");
    let vault_dir = root.join("vault");
    let cache_path = root.join("forge-autotune-cache.json");
    let vault = open_vault(&vault_dir);
    let key = forge_key();
    let configs = two_configs(&key);

    let before = sot_readback(&vault, &cache_path);
    assert_eq!(before["cache_exists"], false);
    assert_eq!(before["bandit_rows"].as_array().unwrap().len(), 0);
    assert_eq!(before["ledger_rows"].as_array().unwrap().len(), 0);

    let mut ledger = open_anneal_ledger(&vault);
    let bandit_store = ConfigBanditStore::new(AsterBanditStorage::new(&vault));
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = ForgeScopeTuner::with_parts(cache, &mut ledger, bandit_store);
        tuner
            .install_candidates(key.clone(), configs.clone())
            .unwrap();
        tuner.on_op_for_arm(key.clone(), 0, 1_000, 0.99).unwrap();
        tuner.on_op_for_arm(key.clone(), 1, 800, 0.99).unwrap();
        tuner.on_op_for_arm(key.clone(), 1, 790, 0.99).unwrap();
        let decision = tuner.on_op_for_arm(key.clone(), 1, 780, 0.99).unwrap();
        assert_eq!(decision.incumbent, configs[1]);
        assert!(decision.promoted.is_some());
    }
    vault.flush().expect("flush happy path");
    let after = sot_readback(&vault, &cache_path);
    assert_eq!(after["cache_entries"].as_array().unwrap().len(), 1);
    assert_eq!(after["bandit_rows"].as_array().unwrap().len(), 1);
    assert_eq!(after["ledger_rows"].as_array().unwrap().len(), 1);
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["action"],
        "autotune_promote"
    );
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["metrics"]["metrics"][0]["incumbent_value"],
        1_000.0
    );
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["metrics"]["metrics"][0]["candidate_value"],
        780.0
    );
    assert!(
        after["ledger_rows"][0]["payload_json"]["description"]
            .as_str()
            .unwrap()
            .contains("latency 1000 -> 780")
    );

    let lower_recall_before = sot_readback(&vault, &cache_path);
    let low_key = ShapeKey::new("gemm", &[1025, 768], DType::Fp16, "cuda");
    let low_store = ConfigBanditStore::new(AsterBanditStorage::new(&vault));
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = ForgeScopeTuner::with_parts(cache, &mut ledger, low_store);
        tuner
            .install_candidates(low_key.clone(), two_configs(&low_key))
            .unwrap();
        tuner
            .on_op_for_arm(low_key.clone(), 0, 1_000, 0.99)
            .unwrap();
        for _ in 0..3 {
            let decision = tuner.on_op_for_arm(low_key.clone(), 1, 700, 0.98).unwrap();
            assert!(decision.promoted.is_none());
        }
    }
    vault.flush().expect("flush lower recall edge");
    let lower_recall_after = sot_readback(&vault, &cache_path);
    assert_eq!(
        lower_recall_after["cache_entries"],
        lower_recall_before["cache_entries"]
    );
    assert_eq!(
        lower_recall_after["ledger_rows"],
        lower_recall_before["ledger_rows"]
    );

    let first_key = ShapeKey::new("gemm", &[1, 0], DType::Fp32, "cpu");
    let first_before = sot_readback(&vault, &cache_path);
    let first_store = ConfigBanditStore::new(AsterBanditStorage::new(&vault));
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = ForgeScopeTuner::with_parts(cache, &mut ledger, first_store);
        let decision = tuner.on_op(first_key.clone(), 500, 0.99).unwrap();
        assert_eq!(decision.incumbent, ForgeConfig::default_for(&first_key));
        assert!(decision.promoted.is_none());
    }
    vault.flush().expect("flush first op edge");
    let first_after = sot_readback(&vault, &cache_path);
    assert_eq!(first_after["cache_entries"], first_before["cache_entries"]);
    assert_eq!(first_after["ledger_rows"], first_before["ledger_rows"]);

    let missing_cache = root.join("missing-parent").join("cache.json");
    let cache = AutotuneCache::load(&missing_cache).unwrap();
    let mut fail_tuner = ForgeScopeTuner::new(cache);
    fail_tuner
        .install_candidates(key.clone(), configs.clone())
        .unwrap();
    fail_tuner
        .on_op_for_arm(key.clone(), 0, 1_000, 0.99)
        .unwrap();
    fail_tuner.on_op_for_arm(key.clone(), 1, 800, 0.99).unwrap();
    fail_tuner.on_op_for_arm(key.clone(), 1, 790, 0.99).unwrap();
    let cache_error = fail_tuner
        .on_op_for_arm(key.clone(), 1, 780, 0.99)
        .unwrap_err();
    assert_eq!(cache_error.code, CALYX_FORGE_CACHE_WRITE_FAIL);
    assert_eq!(fail_tuner.get_incumbent(&key).unwrap(), configs[1]);

    let final_readback = sot_readback(&vault, &cache_path);
    write_json(
        &root.join("forge-scope-readback.json"),
        &json!({
            "surface": "anneal.forge_scope_tuner",
            "source_of_truth": {
                "cache_json": cache_path.display().to_string(),
                "bandit_cf": "vault/cf/anneal_bandit",
                "ledger_cf": "vault/cf/ledger"
            },
            "trigger": "50 synthetic observations equivalent: incumbent arm0 baseline plus arm1 wins; FSV uses 3 wins to prove hysteresis promotion deterministically",
            "expected": {
                "happy_cache_entries": 1,
                "happy_bandit_rows": 1,
                "happy_ledger_action": "autotune_promote",
                "happy_latency_before_ns": 1000,
                "happy_latency_after_ns": 780,
                "incumbent_tile_m": 128,
                "incumbent_batch_size": 2
            },
            "before": before,
            "after_happy_path": after,
            "final_readback": final_readback,
            "edges": [
                {
                    "case": "lower_recall_candidate_rejected",
                    "before": lower_recall_before,
                    "after": lower_recall_after
                },
                {
                    "case": "first_on_op_new_key_defaults",
                    "before": first_before,
                    "after": first_after,
                    "default_config": ForgeConfig::default_for(&first_key)
                },
                {
                    "case": "cache_write_fail",
                    "expected": CALYX_FORGE_CACHE_WRITE_FAIL,
                    "actual": cache_error.code,
                    "in_memory_incumbent_after": fail_tuner.get_incumbent(&key).unwrap()
                }
            ]
        }),
    );
}

fn open_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        "01J41300000000000000000000".parse().unwrap(),
        b"issue413-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn open_anneal_ledger(
    vault: &AsterVault,
) -> calyx_anneal::AnnealLedger<AsterAnnealLedgerStore<'_, calyx_core::SystemClock>, FixedClock> {
    let store = AsterAnnealLedgerStore::new(vault);
    let appender = LedgerAppender::open(store, FixedClock::new(FSV_TS)).unwrap();
    calyx_anneal::AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-issue413-fsv".to_string()),
    )
    .unwrap()
}

fn sot_readback(vault: &AsterVault, cache_path: &Path) -> Value {
    json!({
        "cache_exists": cache_path.exists(),
        "cache_entries": read_cache_entries(cache_path),
        "bandit_rows": read_bandit_rows(vault),
        "ledger_rows": read_ledger_rows(vault),
    })
}

fn read_cache_entries(path: &Path) -> Value {
    if !path.exists() {
        return json!([]);
    }
    let raw = fs::read(path).expect("read cache");
    let value: Value = serde_json::from_slice(&raw).expect("parse cache");
    value.get("entries").cloned().unwrap_or_else(|| json!([]))
}

fn read_bandit_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealBandit)
        .expect("scan anneal_bandit")
        .into_iter()
        .map(|(key, value)| {
            let bandit = decode_config_bandit(&value).expect("decode bandit");
            json!({
                "key_hex": hex(&key),
                "value_hex": hex(&value),
                "incumbent": bandit.incumbent_idx,
                "arm_count": bandit.arms.len(),
                "arms": bandit.arms,
            })
        })
        .collect()
}

fn read_ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan ledger")
        .into_iter()
        .map(|(key, bytes)| {
            let entry = decode_ledger(&bytes).expect("decode ledger entry");
            assert_eq!(entry.kind, EntryKind::Anneal);
            assert_eq!(key, ledger_key(entry.seq));
            json!({
                "seq": entry.seq,
                "key_hex": hex(&key),
                "payload_hex": hex(&entry.payload),
                "payload_json": serde_json::from_slice::<Value>(&entry.payload).unwrap(),
            })
        })
        .collect()
}

fn forge_key() -> ShapeKey {
    ShapeKey::new("gemm", &[768, 768], DType::Fp16, "cuda")
}

fn two_configs(key: &ShapeKey) -> Vec<ForgeConfig> {
    vec![
        ForgeConfig::default_for(key),
        ForgeConfig {
            tile_m: 128,
            tile_n: 128,
            tile_k: 64,
            dtype: DType::Fp16,
            batch_size: 2,
        },
    ]
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove old FSV dir");
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
