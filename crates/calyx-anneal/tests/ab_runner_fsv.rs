use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use calyx_anneal::{
    ABPromotionConfig, ABResult, ABRunner, ABVerdict, AnnealLedger, AsterAnnealLedgerStore,
    BanditPolicy, ConfigBandit, DType, ForgeConfig, NoopABBudget, ShapeKey, TripwireRegistry,
    decode_anneal_ledger_payload,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::FixedClock;
use calyx_forge::AutotuneCache;
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use fsv_support::{hex, reset_dir, vault_id, write_json, write_manifest};
use serde_json::{Value, json};

const FSV_TS: u64 = 1_785_500_416;

#[test]
#[ignore = "requires CALYX_ISSUE416_FSV_ROOT in a manual verification run"]
fn issue416_ab_runner_ledger_and_cache_fsv() {
    let root = PathBuf::from(env::var("CALYX_ISSUE416_FSV_ROOT").expect("set FSV root"));
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir);
    let cache_path = root.join("autotune-cache.json");
    let before_rows = read_ledger_rows(&vault);
    assert!(before_rows.is_empty());
    assert!(!cache_path.exists());

    let key = shape_key("issue416-happy");
    let promotion = promotion_config(&key);
    let mut bandit = make_bandit();
    let mut runner = make_runner(&vault, &vault_dir, &cache_path, NoopABBudget::default());
    runner
        .start_trial_with_config(key.clone(), 1, 0, 100, Some(promotion.clone()))
        .unwrap();
    let verdict = run_samples(
        &mut runner,
        &key,
        &mut bandit,
        SampleSpec {
            samples: 100,
            incumbent_latency: 100,
            candidate_latency: 79,
            incumbent_recall: 0.95,
            candidate_recall: 0.95,
        },
    )
    .expect("happy verdict");
    let ABVerdict::Promoted(record) = verdict else {
        panic!("expected promotion");
    };
    assert_eq!(record.samples, 100);
    assert!(record.latency_after_ns < record.latency_before_ns * 80 / 100);
    assert_eq!(bandit.incumbent_idx, 1);
    drop(runner);
    vault.flush().expect("flush happy path");

    let after_promote = read_ledger_rows(&vault);
    assert_eq!(after_promote.len(), before_rows.len() + 1);
    assert_eq!(
        after_promote[0]["payload_json"]["action"],
        "autotune_promote"
    );
    let cache_bytes = fs::read(&cache_path).expect("cache persisted");
    let cache_json: Value = serde_json::from_slice(&cache_bytes).expect("cache JSON");
    assert_eq!(AutotuneCache::load(&cache_path).unwrap().len(), 1);

    let recall_before = read_ledger_rows(&vault);
    let mut recall_bandit = make_bandit();
    let mut recall_runner = make_runner(&vault, &vault_dir, &cache_path, NoopABBudget::default());
    let recall_key = shape_key("issue416-recall-edge");
    recall_runner
        .start_trial_with_config(recall_key.clone(), 1, 0, 1, None)
        .unwrap();
    let recall_verdict = run_samples(
        &mut recall_runner,
        &recall_key,
        &mut recall_bandit,
        SampleSpec {
            samples: 1,
            incumbent_latency: 100,
            candidate_latency: 70,
            incumbent_recall: 0.95,
            candidate_recall: 0.89,
        },
    )
    .expect("recall edge verdict");
    assert!(matches!(recall_verdict, ABVerdict::Kept(_)));
    drop(recall_runner);
    vault.flush().expect("flush recall edge");
    let recall_after = read_ledger_rows(&vault);
    assert_eq!(recall_after.len(), recall_before.len() + 1);
    assert_eq!(recall_after[1]["payload_json"]["action"], "autotune_ab");

    let duplicate_before = read_ledger_rows(&vault);
    let mut duplicate_runner =
        make_runner(&vault, &vault_dir, &cache_path, NoopABBudget::default());
    let duplicate_key = shape_key("issue416-duplicate-edge");
    duplicate_runner
        .start_trial(duplicate_key.clone(), 1, 0)
        .unwrap();
    let duplicate_code = duplicate_runner
        .start_trial(duplicate_key.clone(), 1, 0)
        .unwrap_err()
        .code
        .to_string();
    drop(duplicate_runner);
    vault.flush().expect("flush duplicate edge");
    let duplicate_after = read_ledger_rows(&vault);
    assert_eq!(duplicate_before, duplicate_after);

    let budget_before = read_ledger_rows(&vault);
    let mut budget_bandit = make_bandit();
    let mut budget_runner = make_runner(&vault, &vault_dir, &cache_path, NoopABBudget { ticks: 0 });
    let budget_key = shape_key("issue416-budget-edge");
    budget_runner
        .start_trial_with_config(budget_key.clone(), 1, 0, 1, None)
        .unwrap();
    let budget_verdict = budget_runner
        .record_query(
            &budget_key,
            result(0, 100, 0.95),
            result(1, 70, 0.95),
            &mut budget_bandit,
        )
        .unwrap()
        .expect("budget verdict");
    assert!(matches!(budget_verdict, ABVerdict::Abandoned(_)));
    drop(budget_runner);
    vault.flush().expect("flush budget edge");
    let final_rows = read_ledger_rows(&vault);
    assert_eq!(final_rows.len(), budget_before.len() + 1);
    assert_eq!(
        final_rows[2]["payload_json"]["action"],
        "autotune_abandoned"
    );

    let readback_path = root.join("ab-runner-readback.json");
    write_json(
        &readback_path,
        &json!({
            "surface": "anneal.ab_runner",
            "source_of_truth": "Aster ledger CF rows plus WAL under vault/ and persisted AutotuneCache JSON",
            "vault": vault_dir.display().to_string(),
            "cache": cache_path.display().to_string(),
            "trigger": "100 deterministic A/B query pairs with candidate p99 79ns vs incumbent 100ns",
            "expected": {
                "promoted_action": "autotune_promote",
                "latency_before_ns": 100,
                "latency_after_ns": 79,
                "cache_entries": 1
            },
            "before_rows": before_rows,
            "after_promote": after_promote,
            "cache_bytes_hex": hex(&cache_bytes),
            "cache_json": cache_json,
            "edges": [
                {
                    "case": "recall_regression_keeps_incumbent",
                    "before_rows": recall_before,
                    "after_rows": recall_after
                },
                {
                    "case": "duplicate_trial_fails_closed",
                    "expected": "CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE",
                    "actual_code": duplicate_code,
                    "before_rows": duplicate_before,
                    "after_rows": duplicate_after
                },
                {
                    "case": "budget_exhaustion_abandons",
                    "before_rows": budget_before,
                    "after_rows": final_rows
                }
            ],
            "final_rows": final_rows
        }),
    );
    write_manifest(&root, &[readback_path, cache_path]);
}

fn make_runner<'a>(
    vault: &'a AsterVault,
    vault_dir: &Path,
    cache_path: &Path,
    budget: NoopABBudget,
) -> ABRunner<
    AnnealLedger<AsterAnnealLedgerStore<'a, calyx_core::SystemClock>, FixedClock>,
    NoopABBudget,
> {
    let store = AsterAnnealLedgerStore::new(vault);
    let appender = LedgerAppender::open(store, FixedClock::new(FSV_TS)).unwrap();
    let ledger =
        AnnealLedger::new(appender, ActorId::Service("calyx-anneal-fsv".to_string())).unwrap();
    let cache = AutotuneCache::load(cache_path).unwrap();
    ABRunner::new(
        TripwireRegistry::load_from_vault(vault_dir).unwrap(),
        ledger,
        budget,
        Arc::new(FixedClock::new(FSV_TS)),
    )
    .with_cache(cache)
}

fn run_samples<W, B>(
    runner: &mut ABRunner<W, B>,
    key: &ShapeKey,
    bandit: &mut ConfigBandit,
    spec: SampleSpec,
) -> Option<ABVerdict>
where
    W: calyx_anneal::ABLedgerWriter,
    B: calyx_anneal::ABTrialBudget,
{
    let mut verdict = None;
    for _ in 0..spec.samples {
        verdict = runner
            .record_query(
                key,
                result(0, spec.incumbent_latency, spec.incumbent_recall),
                result(1, spec.candidate_latency, spec.candidate_recall),
                bandit,
            )
            .unwrap();
    }
    verdict
}

#[derive(Clone, Copy, Debug)]
struct SampleSpec {
    samples: usize,
    incumbent_latency: u64,
    candidate_latency: u64,
    incumbent_recall: f64,
    candidate_recall: f64,
}

fn read_ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan ledger CF")
        .into_iter()
        .map(|(key, bytes)| {
            let entry = decode_ledger(&bytes).expect("decode ledger entry");
            let anneal =
                decode_anneal_ledger_payload(&entry.payload).expect("decode anneal payload");
            assert_eq!(entry.kind, EntryKind::Anneal);
            assert_eq!(key, ledger_key(entry.seq));
            json!({
                "seq": entry.seq,
                "key_hex": hex(&key),
                "kind": entry.kind.as_str(),
                "prev_hash": hex(&entry.prev_hash),
                "entry_hash": hex(&entry.entry_hash),
                "payload_hex": hex(&entry.payload),
                "payload_json": anneal,
            })
        })
        .collect()
}

fn open_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        b"issue416-ab-runner".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn make_bandit() -> ConfigBandit {
    let mut bandit =
        ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 416).with_hysteresis(1);
    bandit.add_arm(b"incumbent".to_vec());
    bandit.add_arm(b"candidate".to_vec());
    bandit
}

fn shape_key(op: &str) -> ShapeKey {
    ShapeKey::new(op, &[128, 64], DType::Fp32, "cpu0")
}

fn promotion_config(key: &ShapeKey) -> ABPromotionConfig {
    let config = ForgeConfig::default_for(key).to_best_config(key);
    ABPromotionConfig {
        key: key.autotune_key(0.95),
        config,
    }
}

fn result(arm_idx: usize, latency_ns: u64, recall_k: f64) -> ABResult {
    ABResult {
        arm_idx,
        latency_ns,
        recall_k,
        bits_per_anchor: 1.0,
        ts: FSV_TS,
    }
}
