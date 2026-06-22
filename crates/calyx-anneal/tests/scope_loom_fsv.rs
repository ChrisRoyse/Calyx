use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    AsterAnnealLedgerStore, AsterBanditStorage, CALYX_LOOM_PLAN_WRITE_FAIL, ConcatKey,
    ConfigBanditStore, LoomScopeTuner, MatPlanConfig, QueryLog, QueryObservation,
    decode_config_bandit,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{FixedClock, LensId};
use calyx_forge::AutotuneCache;
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use serde_json::{Value, json};

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use fsv_support::write_json;

const FSV_TS: u64 = 1_785_500_415;

#[test]
#[ignore = "requires CALYX_ISSUE415_FSV_ROOT in a manual verification run"]
fn issue415_loom_scope_tuner_fsv() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE415_FSV_ROOT").expect("set CALYX_ISSUE415_FSV_ROOT"));
    reset_dir(&root);
    fs::create_dir_all(&root).expect("create FSV root");
    let vault_dir = root.join("vault");
    let cache_path = root.join("loom-autotune-cache.json");
    let vault = open_vault(&vault_dir);
    let before = sot_readback(&vault, &cache_path);
    assert_eq!(before["cache_exists"], false);
    assert_eq!(before["bandit_rows"].as_array().unwrap().len(), 0);
    assert_eq!(before["ledger_rows"].as_array().unwrap().len(), 0);

    let mut ledger = open_anneal_ledger(&vault);
    let bandit_store = ConfigBanditStore::new(AsterBanditStorage::new(&vault));
    let log = synthetic_log();
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = LoomScopeTuner::with_parts(
            cache,
            MatPlanConfig::default(),
            &mut ledger,
            bandit_store,
            calyx_anneal::NoopLoomMaterializer,
        );
        let mut saw_promotion = false;
        for _ in 0..20 {
            let decision = tuner.on_query_tick(&log).unwrap();
            if let Some(promotion) = decision.promoted {
                saw_promotion = true;
                assert!(promotion.new_plan.eager_pairs.contains(&(lens(1), lens(2))));
                assert!(promotion.bits_after >= promotion.bits_before);
            }
        }
        assert!(saw_promotion);
        assert!(tuner.current_plan.eager_pairs.contains(&(lens(1), lens(2))));
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
    let eager_pairs = after["cache_entries"][0]["config"]["extra"]["eager_pairs"]
        .as_str()
        .unwrap();
    assert!(eager_pairs.contains(&format!("{}:{}", lens(1), lens(2))));

    let lower_bits_before = sot_readback(&vault, &cache_path);
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = LoomScopeTuner::with_parts(
            cache,
            eager_plan(lens(1), lens(2)),
            &mut ledger,
            ConfigBanditStore::new(AsterBanditStorage::new(&vault)),
            calyx_anneal::NoopLoomMaterializer,
        );
        tuner
            .install_candidates(vec![
                eager_plan(lens(1), lens(2)),
                eager_plan(lens(2), lens(3)),
            ])
            .unwrap();
        let log = lower_bits_log();
        for _ in 0..3 {
            let decision = tuner.on_query_tick_for_arm(&log, 1).unwrap();
            assert!(decision.promoted.is_none());
        }
    }
    vault.flush().expect("flush lower bits edge");
    let lower_bits_after = sot_readback(&vault, &cache_path);
    assert_eq!(
        lower_bits_after["cache_entries"],
        lower_bits_before["cache_entries"]
    );
    assert_eq!(
        lower_bits_after["ledger_rows"],
        lower_bits_before["ledger_rows"]
    );

    let zero_before = sot_readback(&vault, &cache_path);
    {
        let cache = AutotuneCache::load(&cache_path).unwrap();
        let mut tuner = LoomScopeTuner::new(cache, MatPlanConfig::default());
        let decision = tuner.on_query_tick(&QueryLog::with_budgets(1, 1)).unwrap();
        assert!(decision.promoted.is_none());
    }
    vault.flush().expect("flush zero edge");
    let zero_after = sot_readback(&vault, &cache_path);
    assert_eq!(zero_after["cache_entries"], zero_before["cache_entries"]);
    assert_eq!(zero_after["ledger_rows"], zero_before["ledger_rows"]);

    let missing_cache = root.join("missing-parent").join("cache.json");
    let cache = AutotuneCache::load(&missing_cache).unwrap();
    let materializer = CountingMaterializer::default();
    let materializer_calls = materializer.calls.clone();
    let mut fail_tuner = LoomScopeTuner::with_parts(
        cache,
        MatPlanConfig::default(),
        calyx_anneal::NoopLoomPromotionWriter,
        calyx_anneal::NoopLoomBanditStore,
        materializer,
    );
    fail_tuner
        .install_candidates(vec![
            MatPlanConfig::default(),
            MatPlanConfig {
                eager_pairs: vec![(lens(1), lens(2))],
                indexed_concat_keys: vec![ConcatKey::new(lens(1), lens(2))],
            },
        ])
        .unwrap();
    let fail_log = synthetic_log();
    fail_tuner.on_query_tick_for_arm(&fail_log, 1).unwrap();
    fail_tuner.on_query_tick_for_arm(&fail_log, 1).unwrap();
    let cache_error = fail_tuner.on_query_tick_for_arm(&fail_log, 1).unwrap_err();
    assert_eq!(cache_error.code, CALYX_LOOM_PLAN_WRITE_FAIL);
    let fail_materializer_calls = *materializer_calls.lock().unwrap();
    assert_eq!(fail_materializer_calls, 0);

    write_json(
        &root.join("loom-scope-readback.json"),
        &json!({
            "surface": "anneal.loom_scope_tuner",
            "source_of_truth": {
                "cache_json": cache_path.display().to_string(),
                "bandit_cf": "vault/cf/anneal_bandit",
                "ledger_cf": "vault/cf/ledger",
                "wal": "vault/wal"
            },
            "trigger": "20 on_query_tick calls with synthetic log: (L1,L2) queried 90%, pair bits 0.4",
            "expected": {
                "happy_cache_entries": 1,
                "happy_bandit_rows": 1,
                "happy_ledger_action": "autotune_promote",
                "eager_pair": format!("{}:{}", lens(1), lens(2)),
                "bits_non_decreasing": true
            },
            "before": before,
            "after_happy_path": after,
            "final_readback": sot_readback(&vault, &cache_path),
            "edges": [
                {"case": "lower_latency_lower_bits_rejected", "before": lower_bits_before, "after": lower_bits_after},
                {"case": "zero_pairs_noop", "before": zero_before, "after": zero_after},
                {"case": "cache_write_fail", "expected": CALYX_LOOM_PLAN_WRITE_FAIL, "actual": cache_error.code, "materializer_calls": fail_materializer_calls}
            ]
        }),
    );
}

#[derive(Default)]
struct CountingMaterializer {
    calls: Arc<Mutex<usize>>,
}

impl calyx_anneal::LoomMaterializer for CountingMaterializer {
    fn apply_plan(
        &self,
        _old_plan: &MatPlanConfig,
        _new_plan: &MatPlanConfig,
    ) -> calyx_core::Result<()> {
        *self.calls.lock().unwrap() += 1;
        Ok(())
    }
}

fn synthetic_log() -> QueryLog {
    let mut log = QueryLog::with_budgets(1, 1);
    for _ in 0..18 {
        log.push(QueryObservation::new(
            lens(1),
            lens(2),
            1_000,
            780,
            Some(760),
            0.4,
        ));
    }
    for _ in 0..2 {
        log.push(QueryObservation::new(
            lens(2),
            lens(3),
            950,
            850,
            Some(820),
            0.1,
        ));
    }
    log
}

fn lower_bits_log() -> QueryLog {
    let mut log = QueryLog::with_budgets(1, 1);
    log.push(QueryObservation::new(
        lens(1),
        lens(2),
        1_000,
        900,
        Some(850),
        0.40,
    ));
    log.push(QueryObservation::new(
        lens(2),
        lens(3),
        400,
        200,
        Some(180),
        0.38,
    ));
    log
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
    serde_json::from_slice::<Value>(&fs::read(path).expect("read cache"))
        .expect("parse cache")
        .get("entries")
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn read_bandit_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealBandit)
        .expect("scan anneal_bandit")
        .into_iter()
        .map(|(key, value)| {
            let bandit = decode_config_bandit(&value).expect("decode bandit");
            json!({"key_hex": hex(&key), "value_hex": hex(&value), "incumbent": bandit.incumbent_idx, "arm_count": bandit.arms.len()})
        })
        .collect()
}

fn read_ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan ledger")
        .into_iter()
        .map(|(key, value)| {
            let entry = decode_ledger(&value).expect("decode ledger");
            let payload = calyx_anneal::decode_anneal_ledger_payload(&entry.payload).unwrap();
            json!({"key_hex": hex(&key), "ledger_key": ledger_key(entry.seq), "entry_kind": EntryKind::Anneal, "payload_json": payload})
        })
        .collect()
}

fn open_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        "01J41500000000000000000000".parse().unwrap(),
        b"issue415-salt".to_vec(),
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
        ActorId::Service("calyx-anneal-issue415-fsv".to_string()),
    )
    .unwrap()
}

fn eager_plan(a: LensId, b: LensId) -> MatPlanConfig {
    MatPlanConfig {
        eager_pairs: vec![(a, b)],
        indexed_concat_keys: Vec::new(),
    }
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove old FSV root");
    }
}

fn lens(seed: u8) -> LensId {
    LensId::from_bytes([seed; 16])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
