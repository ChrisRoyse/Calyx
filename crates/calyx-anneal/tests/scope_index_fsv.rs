use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    AsterAnnealLedgerStore, AsterBanditStorage, CALYX_INDEX_CACHE_WRITE_FAIL,
    CALYX_INDEX_SCOPE_INVALID_CONFIG, ConfigBanditStore, IndexConfig, IndexScopeTuner,
    IndexSlotHealth, IndexTuneSkip, QuantPromotionEvidence, decode_config_bandit,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{FixedClock, SlotId};
use calyx_forge::AutotuneCache;
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use serde_json::{Value, json};

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use fsv_support::write_json;

const FSV_TS: u64 = 1_785_500_414;

#[test]
#[ignore = "requires CALYX_ISSUE614_FSV_ROOT in a manual verification run"]
fn issue614_quant_promotion_ledger_details_fsv() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE614_FSV_ROOT").expect("set CALYX_ISSUE614_FSV_ROOT"));
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
                .on_search_for_arm_with_quant_evidence(
                    slot,
                    1,
                    latency,
                    0.990,
                    9.999_999_5,
                    Some(quant_evidence()),
                )
                .unwrap();
            if let Some(promotion) = decision.promoted {
                saw_promotion = true;
                assert_eq!(promotion.latency_after_ns, 780);
                assert_eq!(promotion.quant_evidence, Some(quant_evidence()));
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
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["details"]["tag"],
        "quant_compression_promotion_v1"
    );
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["details"]["slot"],
        0
    );
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["details"]["slot_hash_bytes"],
        json!(calyx_anneal::shape_key_hash(
            &calyx_anneal::index_slot_label(slot)
        ))
    );
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["details"]["level_before_bits"],
        16
    );
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["details"]["level_after_bits"],
        8
    );
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["details"]["cosine_error_after"],
        0.0004
    );
    assert_eq!(
        after["ledger_rows"][0]["payload_json"]["details"]["guard_far_after"],
        0.001
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

    let missing_evidence_before = sot_readback(&vault, &cache_path);
    let missing_evidence_code = {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = IndexScopeTuner::with_parts(cache, &mut ledger, NoopStore, NotParked);
        tuner
            .install_candidates(SlotId::new(3), configs.clone())
            .unwrap();
        tuner
            .on_search_for_arm_with_quant_evidence(
                SlotId::new(3),
                0,
                1_000,
                0.990,
                10.0,
                Some(quant_evidence()),
            )
            .unwrap();
        tuner
            .on_search_for_arm_with_quant_evidence(
                SlotId::new(3),
                1,
                800,
                0.990,
                10.0,
                Some(quant_evidence()),
            )
            .unwrap();
        tuner
            .on_search_for_arm_with_quant_evidence(
                SlotId::new(3),
                1,
                790,
                0.990,
                10.0,
                Some(quant_evidence()),
            )
            .unwrap();
        tuner
            .on_search_for_arm(SlotId::new(3), 1, 780, 0.990, 10.0)
            .unwrap_err()
            .code
    };
    assert_eq!(missing_evidence_code, CALYX_INDEX_SCOPE_INVALID_CONFIG);
    vault.flush().expect("flush missing evidence edge");
    let missing_evidence_after = sot_readback(&vault, &cache_path);
    assert_eq!(
        missing_evidence_after["cache_entries"],
        missing_evidence_before["cache_entries"]
    );
    assert_eq!(
        missing_evidence_after["ledger_rows"],
        missing_evidence_before["ledger_rows"]
    );

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
        .on_search_for_arm_with_quant_evidence(slot, 1, 800, 0.990, 10.0, Some(quant_evidence()))
        .unwrap();
    fail_tuner
        .on_search_for_arm_with_quant_evidence(slot, 1, 790, 0.990, 10.0, Some(quant_evidence()))
        .unwrap();
    let cache_error = fail_tuner
        .on_search_for_arm_with_quant_evidence(slot, 1, 780, 0.990, 10.0, Some(quant_evidence()))
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
                "incumbent_quant_bits": 8,
                "ledger_details_tag": "quant_compression_promotion_v1",
                "ledger_slot_hash_bytes": calyx_anneal::shape_key_hash(&calyx_anneal::index_slot_label(slot)),
                "cosine_error_after": 0.0004,
                "guard_far_after": 0.001
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
                    "case": "missing_quant_evidence",
                    "before": missing_evidence_before,
                    "after": missing_evidence_after,
                    "expected": CALYX_INDEX_SCOPE_INVALID_CONFIG,
                    "actual": missing_evidence_code
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
        "01J61400000000000000000000".parse().unwrap(),
        b"issue614-salt".to_vec(),
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
        ActorId::Service("calyx-anneal-issue614-fsv".to_string()),
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

fn quant_evidence() -> QuantPromotionEvidence {
    QuantPromotionEvidence {
        cosine_error_before: 0.0,
        cosine_error_after: 0.000_4,
        max_cosine_error: 0.001,
        guard_far_before: 0.001,
        guard_far_after: 0.001,
    }
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove old FSV dir");
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
