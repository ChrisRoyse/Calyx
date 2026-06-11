use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    AsterAnnealLedgerStore, AsterBanditStorage, CALYX_INDEX_CACHE_WRITE_FAIL, ConfigBanditStore,
    IndexConfig, IndexScopeTuner, IndexSlotHealth, IndexTuneSkip, decode_config_bandit,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{FixedClock, SlotId};
use calyx_forge::AutotuneCache;
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use serde_json::{Value, json};

const FSV_TS: u64 = 1_785_500_414;

#[test]
#[ignore = "requires CALYX_ISSUE414_FSV_ROOT on aiwonder"]
fn issue414_index_scope_tuner_fsv() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE414_FSV_ROOT").expect("set CALYX_ISSUE414_FSV_ROOT"));
    reset_dir(&root);
    fs::create_dir_all(&root).expect("create FSV root");
    let vault_dir = root.join("vault");
    let cache_path = root.join("index-autotune-cache.json");
    let vault = open_vault(&vault_dir);
    let slot = SlotId::new(0);
    let configs = quant_configs();

    let before = sot_readback(&vault, &cache_path);
    assert_eq!(before["cache_exists"], false);
    assert_eq!(before["bandit_rows"].as_array().unwrap().len(), 0);
    assert_eq!(before["ledger_rows"].as_array().unwrap().len(), 0);

    let mut ledger = open_anneal_ledger(&vault);
    let bandit_store = ConfigBanditStore::new(AsterBanditStorage::new(&vault));
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = IndexScopeTuner::with_parts(cache, &mut ledger, bandit_store, NotParked);
        tuner.install_candidates(slot, configs.clone()).unwrap();
        tuner
            .on_search_for_arm(slot, 0, 1_000, 0.990, 10.0)
            .unwrap();
        let mut saw_promotion = false;
        for idx in 0..50 {
            let latency = match idx {
                0 => 800,
                1 => 790,
                _ => 780,
            };
            let decision = tuner
                .on_search_for_arm(slot, 1, latency, 0.990, 9.999_999_5)
                .unwrap();
            if let Some(promotion) = decision.promoted {
                saw_promotion = true;
                assert_eq!(promotion.latency_after_ns, 780);
                assert_eq!(decision.incumbent, configs[1]);
            }
        }
        assert!(saw_promotion);
        assert_eq!(tuner.get_incumbent_config(slot).unwrap(), configs[1]);
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

    let bits_loss_before = sot_readback(&vault, &cache_path);
    let loss_slot = SlotId::new(1);
    let loss_store = ConfigBanditStore::new(AsterBanditStorage::new(&vault));
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = IndexScopeTuner::with_parts(cache, &mut ledger, loss_store, NotParked);
        tuner
            .install_candidates(loss_slot, configs.clone())
            .unwrap();
        tuner
            .on_search_for_arm(loss_slot, 0, 1_000, 0.990, 10.0)
            .unwrap();
        for _ in 0..3 {
            let decision = tuner
                .on_search_for_arm(loss_slot, 1, 700, 0.990, 9.5)
                .unwrap();
            assert!(decision.promoted.is_none());
        }
    }
    vault.flush().expect("flush bits loss edge");
    let bits_loss_after = sot_readback(&vault, &cache_path);
    assert_eq!(
        bits_loss_after["cache_entries"],
        bits_loss_before["cache_entries"]
    );
    assert_eq!(
        bits_loss_after["ledger_rows"],
        bits_loss_before["ledger_rows"]
    );

    let parked_before = sot_readback(&vault, &cache_path);
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = IndexScopeTuner::with_parts(cache, &mut ledger, NoopStore, AlwaysParked);
        let decision = tuner.on_search(SlotId::new(2), 700, 0.99, 0.01).unwrap();
        assert_eq!(decision.skipped, Some(IndexTuneSkip::ParkedSlot));
    }
    vault.flush().expect("flush parked edge");
    let parked_after = sot_readback(&vault, &cache_path);
    assert_eq!(
        parked_after["cache_entries"],
        parked_before["cache_entries"]
    );
    assert_eq!(parked_after["ledger_rows"], parked_before["ledger_rows"]);

    let missing_cache = root.join("missing-parent").join("cache.json");
    let cache = AutotuneCache::load(&missing_cache).unwrap();
    let mut fail_tuner = IndexScopeTuner::new(cache);
    fail_tuner
        .install_candidates(slot, configs.clone())
        .unwrap();
    fail_tuner
        .on_search_for_arm(slot, 0, 1_000, 0.990, 10.0)
        .unwrap();
    fail_tuner
        .on_search_for_arm(slot, 1, 800, 0.990, 10.0)
        .unwrap();
    fail_tuner
        .on_search_for_arm(slot, 1, 790, 0.990, 10.0)
        .unwrap();
    let cache_error = fail_tuner
        .on_search_for_arm(slot, 1, 780, 0.990, 10.0)
        .unwrap_err();
    assert_eq!(cache_error.code, CALYX_INDEX_CACHE_WRITE_FAIL);

    write_json(
        &root.join("index-scope-readback.json"),
        &json!({
            "surface": "anneal.index_scope_tuner",
            "source_of_truth": {
                "cache_json": cache_path.display().to_string(),
                "bandit_cf": "vault/cf/anneal_bandit",
                "ledger_cf": "vault/cf/ledger",
                "wal": "vault/wal"
            },
            "trigger": "50 slot_0 synthetic searches: incumbent arm0 ef=64 quant=16, arm1 ef=128 quant=8 promotes after hysteresis and remains incumbent through all 50 arm-B observations",
            "expected": {
                "happy_cache_entries": 1,
                "happy_bandit_rows": 1,
                "happy_ledger_action": "autotune_promote",
                "happy_latency_before_ns": 1000,
                "happy_latency_after_ns": 780,
                "incumbent_hnsw_ef": 128,
                "incumbent_quant_bits": 8
            },
            "before": before,
            "after_happy_path": after,
            "final_readback": sot_readback(&vault, &cache_path),
            "edges": [
                {
                    "case": "quant_downgrade_bits_loss_rejected",
                    "before": bits_loss_before,
                    "after": bits_loss_after
                },
                {
                    "case": "parked_slot_noop",
                    "before": parked_before,
                    "after": parked_after
                },
                {
                    "case": "cache_write_fail",
                    "expected": CALYX_INDEX_CACHE_WRITE_FAIL,
                    "actual": cache_error.code,
                    "in_memory_incumbent_after": fail_tuner.get_incumbent_config(slot).unwrap()
                }
            ]
        }),
    );
}

fn open_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        "01J41400000000000000000000".parse().unwrap(),
        b"issue414-salt".to_vec(),
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
        ActorId::Service("calyx-anneal-issue414-fsv".to_string()),
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

struct NotParked;

impl IndexSlotHealth for NotParked {
    fn is_slot_parked(&self, _slot_id: SlotId) -> bool {
        false
    }
}

struct AlwaysParked;

impl IndexSlotHealth for AlwaysParked {
    fn is_slot_parked(&self, _slot_id: SlotId) -> bool {
        true
    }
}

struct NoopStore;

impl calyx_anneal::IndexBanditPersistence for NoopStore {
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

fn quant_configs() -> Vec<IndexConfig> {
    vec![
        IndexConfig::default(),
        IndexConfig {
            hnsw_ef: 128,
            quant_bits: 8,
            ..IndexConfig::default()
        },
    ]
}

fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize readback"),
    )
    .expect("write readback");
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove old FSV dir");
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
