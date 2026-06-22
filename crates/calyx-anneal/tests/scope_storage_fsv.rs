use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    AsterAnnealLedgerStore, AsterBanditStorage, CALYX_STORAGE_SCOPE_INVALID_CONFIG,
    ConfigBanditStore, NoopStorageBanditStore, StorageBanditPersistence, StorageConfig,
    StorageMetrics, StorageScopeTuner, StorageShapeKey, decode_config_bandit,
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

const FSV_TS: u64 = 1_785_500_583;

#[test]
#[ignore = "requires CALYX_ISSUE583_FSV_ROOT in a manual verification run"]
fn issue583_storage_autotune_scope_fsv() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE583_FSV_ROOT").expect("set CALYX_ISSUE583_FSV_ROOT"));
    reset_dir(&root);
    fs::create_dir_all(&root).expect("create FSV root");
    let vault_dir = root.join("vault");
    let cache_path = root.join("storage-autotune-cache.json");
    let vault = open_vault(&vault_dir);
    let key = storage_key("happy");
    let configs = tuned_configs();

    let before = sot_readback(&vault, &vault_dir, &cache_path);
    assert_eq!(before["cache_exists"], false);
    assert_eq!(before["bandit_rows"].as_array().unwrap().len(), 0);
    assert_eq!(before["ledger_rows"].as_array().unwrap().len(), 0);

    let mut ledger = open_anneal_ledger(&vault);
    let bandit_store = ConfigBanditStore::new(AsterBanditStorage::new(&vault));
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = StorageScopeTuner::with_parts(cache, &mut ledger, bandit_store);
        tuner
            .install_candidates(key.clone(), configs.clone())
            .unwrap();
        tuner
            .on_observation_for_arm(key.clone(), 0, baseline_metrics())
            .unwrap();
        let mut saw_promotion = false;
        for _ in 0..50 {
            let decision = tuner
                .on_observation_for_arm(key.clone(), 1, winning_metrics())
                .unwrap();
            if let Some(promotion) = decision.promoted {
                saw_promotion = true;
                assert_eq!(promotion.new_config, configs[1]);
                assert_eq!(decision.incumbent, configs[1]);
            }
        }
        assert!(saw_promotion);
    }
    vault.flush().expect("flush happy path");
    let after = sot_readback(&vault, &vault_dir, &cache_path);
    assert_eq!(after["cache_entries"].as_array().unwrap().len(), 1);
    assert_eq!(after["bandit_rows"].as_array().unwrap().len(), 1);
    assert_eq!(after["ledger_rows"].as_array().unwrap().len(), 1);
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["action"],
        "autotune_promote"
    );
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["details"]["tag"],
        "storage_autotune_promotion_v1"
    );
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["details"]["new_config"]["compaction_interval_ms"],
        5000
    );

    let regression_before = sot_readback(&vault, &vault_dir, &cache_path);
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = StorageScopeTuner::with_parts(cache, &mut ledger, NoopStore);
        let loss_key = storage_key("write-amp-regression");
        tuner
            .install_candidates(loss_key.clone(), configs.clone())
            .unwrap();
        tuner
            .on_observation_for_arm(loss_key.clone(), 0, baseline_metrics())
            .unwrap();
        for _ in 0..3 {
            let decision = tuner
                .on_observation_for_arm(loss_key.clone(), 1, write_amp_regression())
                .unwrap();
            assert!(decision.promoted.is_none());
        }
    }
    vault.flush().expect("flush regression edge");
    let regression_after = sot_readback(&vault, &vault_dir, &cache_path);
    assert_eq!(
        regression_after["cache_entries"],
        regression_before["cache_entries"]
    );
    assert_eq!(
        regression_after["ledger_rows"],
        regression_before["ledger_rows"]
    );

    let invalid_metrics_before = sot_readback(&vault, &vault_dir, &cache_path);
    let invalid_metrics_code = {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = StorageScopeTuner::with_parts(cache, &mut ledger, NoopStorageBanditStore);
        let metric_key = storage_key("invalid-metrics");
        tuner
            .install_candidates(metric_key.clone(), configs.clone())
            .unwrap();
        let mut bad = baseline_metrics();
        bad.cache_miss_milli = 1_001;
        tuner
            .on_observation_for_arm(metric_key, 0, bad)
            .unwrap_err()
            .code
    };
    assert_eq!(invalid_metrics_code, CALYX_STORAGE_SCOPE_INVALID_CONFIG);
    vault.flush().expect("flush invalid metrics edge");
    let invalid_metrics_after = sot_readback(&vault, &vault_dir, &cache_path);
    assert_eq!(
        invalid_metrics_after["cache_entries"],
        invalid_metrics_before["cache_entries"]
    );
    assert_eq!(
        invalid_metrics_after["ledger_rows"],
        invalid_metrics_before["ledger_rows"]
    );

    let invalid_config_code = {
        let bad = StorageConfig {
            prefetch_bytes: 123,
            ..StorageConfig::default()
        };
        calyx_anneal::validate_storage_config(&bad)
            .unwrap_err()
            .code
    };
    assert_eq!(invalid_config_code, CALYX_STORAGE_SCOPE_INVALID_CONFIG);

    write_json(
        &root.join("storage-scope-readback.json"),
        &json!({
            "surface": "anneal.storage_scope_tuner",
            "source_of_truth": {
                "cache_json": cache_path.display().to_string(),
                "bandit_cf": "vault/cf/anneal_bandit",
                "ledger_cf": "vault/cf/ledger",
                "wal": "vault/wal"
            },
            "trigger": "50 synthetic storage observations: incumbent arm0 default, arm1 shorter compaction cadence and larger prefetch promotes after hysteresis",
            "expected": {
                "happy_cache_entries": 1,
                "happy_bandit_rows": 1,
                "happy_ledger_action": "autotune_promote",
                "happy_details_tag": "storage_autotune_promotion_v1",
                "incumbent_compaction_interval_ms": 5000,
                "incumbent_prefetch_bytes": 131072
            },
            "before": before,
            "after_happy_path": after,
            "final_readback": sot_readback(&vault, &vault_dir, &cache_path),
            "edges": [
                {
                    "case": "write_amp_regression_rejected",
                    "before": regression_before,
                    "after": regression_after
                },
                {
                    "case": "invalid_metrics_rejected",
                    "before": invalid_metrics_before,
                    "after": invalid_metrics_after,
                    "expected": CALYX_STORAGE_SCOPE_INVALID_CONFIG,
                    "actual": invalid_metrics_code
                },
                {
                    "case": "invalid_config_rejected",
                    "expected": CALYX_STORAGE_SCOPE_INVALID_CONFIG,
                    "actual": invalid_config_code
                }
            ]
        }),
    );
}

fn open_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        "01J58300000000000000000000".parse().unwrap(),
        b"issue583-salt".to_vec(),
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
        ActorId::Service("calyx-anneal-issue583-fsv".to_string()),
    )
    .unwrap()
}

fn sot_readback(vault: &AsterVault, vault_dir: &Path, cache_path: &Path) -> Value {
    json!({
        "cache_exists": cache_path.exists(),
        "cache_entries": read_cache_entries(cache_path),
        "bandit_rows": read_bandit_rows(vault),
        "ledger_rows": read_ledger_rows(vault),
        "wal_files": wal_files(vault_dir),
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

fn wal_files(vault_dir: &Path) -> Vec<Value> {
    let wal_dir = vault_dir.join("wal");
    if !wal_dir.exists() {
        return Vec::new();
    }
    let mut rows = fs::read_dir(wal_dir)
        .expect("read wal dir")
        .map(|entry| {
            let entry = entry.expect("wal entry");
            let path = entry.path();
            let meta = fs::metadata(&path).expect("wal metadata");
            json!({
                "file": path.file_name().unwrap().to_string_lossy(),
                "bytes": meta.len(),
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|value| value["file"].as_str().unwrap_or("").to_string());
    rows
}

struct NoopStore;

impl StorageBanditPersistence for NoopStore {
    fn load_bandit(
        &self,
        _key_hash: [u8; 32],
    ) -> calyx_core::Result<Option<calyx_anneal::ConfigBandit>> {
        Ok(None)
    }

    fn save_bandit(
        &self,
        _key_hash: [u8; 32],
        _bandit: &calyx_anneal::ConfigBandit,
    ) -> calyx_core::Result<()> {
        Ok(())
    }
}

fn tuned_configs() -> Vec<StorageConfig> {
    vec![
        StorageConfig::default(),
        StorageConfig {
            compaction_interval_ms: 5_000,
            debt_trigger_score_milli: 750,
            prefetch_bytes: 128 * 1024,
            ..StorageConfig::default()
        },
    ]
}

fn baseline_metrics() -> StorageMetrics {
    StorageMetrics {
        p99_read_ns: 1_000,
        write_amp_milli: 1_500,
        cache_miss_milli: 100,
        tier_hot_hit_milli: 900,
        codebook_staleness_secs: 600,
        prefetch_hit_milli: 800,
    }
}

fn winning_metrics() -> StorageMetrics {
    StorageMetrics {
        p99_read_ns: 780,
        write_amp_milli: 1_500,
        cache_miss_milli: 90,
        tier_hot_hit_milli: 930,
        codebook_staleness_secs: 300,
        prefetch_hit_milli: 840,
    }
}

fn write_amp_regression() -> StorageMetrics {
    StorageMetrics {
        p99_read_ns: 700,
        write_amp_milli: 1_501,
        ..baseline_metrics()
    }
}

fn storage_key(label: &str) -> StorageShapeKey {
    StorageShapeKey::new("issue583-vault", label, &[257, 65, 17])
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove old FSV dir");
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
