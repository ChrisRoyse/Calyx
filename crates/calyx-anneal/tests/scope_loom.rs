use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    CALYX_LOOM_PLAN_WRITE_FAIL, ConcatKey, LoomPromotionRecord, LoomPromotionWriter,
    LoomScopeTuner, MatPlanConfig, NoopLoomBanditStore, NoopLoomMaterializer,
    NoopLoomPromotionWriter, QueryLog, QueryObservation, generate_candidate_plan,
    loom_plan_tune_key,
};
use calyx_core::{LensId, Result};
use calyx_forge::AutotuneCache;
use proptest::prelude::*;

#[test]
fn candidate_adds_highest_bits_pair_first() {
    let mut log = QueryLog::with_budgets(1, 1);
    log.push(observation(lens(1), lens(2), 1_000, 800, 760, 0.3));
    log.push(observation(lens(2), lens(3), 1_000, 800, 760, 0.1));

    let candidate = generate_candidate_plan(&MatPlanConfig::default(), &NoAssay, &log);

    assert_eq!(candidate.eager_pairs, vec![(lens(1), lens(2))]);
    assert_eq!(
        candidate.indexed_concat_keys,
        vec![ConcatKey::new(lens(1), lens(2))]
    );
}

#[test]
fn lower_latency_equal_bits_promotes_and_records_ledger_event() {
    let path = temp_path("promotes");
    let writer = RecordingWriter::default();
    let mut tuner = LoomScopeTuner::with_parts(
        AutotuneCache::load(&path).unwrap(),
        eager_plan(lens(1), lens(2)),
        writer.clone(),
        NoopLoomBanditStore,
        NoopLoomMaterializer,
    );
    let candidate = MatPlanConfig {
        eager_pairs: vec![(lens(1), lens(2))],
        indexed_concat_keys: vec![ConcatKey::new(lens(1), lens(2))],
    };
    tuner
        .install_candidates(vec![eager_plan(lens(1), lens(2)), candidate.clone()])
        .unwrap();
    let mut log = QueryLog::with_budgets(1, 1);
    for _ in 0..6 {
        log.push(observation(lens(1), lens(2), 1_000, 900, 700, 0.4));
    }

    tuner.on_query_tick_for_arm(&log, 1).unwrap();
    tuner.on_query_tick_for_arm(&log, 1).unwrap();
    let decision = tuner.on_query_tick_for_arm(&log, 1).unwrap();

    assert_eq!(decision.incumbent, candidate);
    assert_eq!(decision.incumbent_score.bits_sum, 0.4);
    assert_eq!(writer.records().len(), 1);
    let loaded = AutotuneCache::load(&path).unwrap();
    let persisted = loaded.get(&loom_plan_tune_key()).unwrap();
    assert_eq!(persisted.extra.get("eager_pairs_count").unwrap(), "1");
}

#[test]
fn lower_latency_lower_bits_is_not_promoted() {
    let current = eager_plan(lens(1), lens(2));
    let candidate = eager_plan(lens(2), lens(3));
    let writer = RecordingWriter::default();
    let mut tuner = LoomScopeTuner::with_parts(
        AutotuneCache::load(&temp_path("lower_bits")).unwrap(),
        current.clone(),
        writer.clone(),
        NoopLoomBanditStore,
        NoopLoomMaterializer,
    );
    tuner
        .install_candidates(vec![current.clone(), candidate])
        .unwrap();
    let mut log = QueryLog::with_budgets(1, 1);
    log.push(observation(lens(1), lens(2), 1_000, 900, 850, 0.40));
    log.push(observation(lens(2), lens(3), 400, 200, 180, 0.38));

    for _ in 0..3 {
        let decision = tuner.on_query_tick_for_arm(&log, 1).unwrap();
        assert!(decision.promoted.is_none());
    }

    assert_eq!(tuner.current_plan, current);
    assert!(writer.records().is_empty());
}

#[test]
fn edge_cases_return_expected_plans() {
    let current = MatPlanConfig::default();
    assert_eq!(
        generate_candidate_plan(&current, &NoAssay, &QueryLog::with_budgets(1, 1)),
        current
    );

    let mut single = LoomScopeTuner::new(
        AutotuneCache::load(&temp_path("single")).unwrap(),
        MatPlanConfig::default(),
    );
    single
        .install_candidates(vec![eager_plan(lens(1), lens(2))])
        .unwrap();
    assert_eq!(single.bandit.arms.len(), 1);

    let mut log = QueryLog::with_budgets(2, 0);
    log.push(observation(lens(1), lens(2), 900, 800, 700, 0.30));
    log.push(observation(lens(2), lens(3), 900, 800, 700, 0.10));
    log.push(observation(lens(3), lens(4), 900, 800, 700, 0.20));
    let all_eager = MatPlanConfig {
        eager_pairs: vec![(lens(1), lens(2)), (lens(2), lens(3)), (lens(3), lens(4))],
        indexed_concat_keys: Vec::new(),
    };
    let candidate = generate_candidate_plan(&all_eager, &NoAssay, &log);
    assert_eq!(
        candidate.eager_pairs,
        vec![(lens(1), lens(2)), (lens(3), lens(4))]
    );
}

#[test]
fn cache_write_failure_is_fail_closed_but_incumbent_survives() {
    let cache_path = temp_path("missing_parent").join("cache.json");
    let materializer = CountingMaterializer::default();
    let materializer_calls = materializer.calls.clone();
    let mut tuner = LoomScopeTuner::with_parts(
        AutotuneCache::load(&cache_path).unwrap(),
        current_plan(),
        NoopLoomPromotionWriter,
        NoopLoomBanditStore,
        materializer,
    );
    let candidate = MatPlanConfig {
        eager_pairs: vec![(lens(1), lens(2))],
        indexed_concat_keys: vec![ConcatKey::new(lens(1), lens(2))],
    };
    tuner
        .install_candidates(vec![current_plan(), candidate.clone()])
        .unwrap();
    let mut log = QueryLog::with_budgets(1, 1);
    for _ in 0..4 {
        log.push(observation(lens(1), lens(2), 1_000, 900, 700, 0.4));
    }
    tuner.on_query_tick_for_arm(&log, 1).unwrap();
    tuner.on_query_tick_for_arm(&log, 1).unwrap();

    let err = tuner.on_query_tick_for_arm(&log, 1).unwrap_err();

    assert_eq!(err.code, CALYX_LOOM_PLAN_WRITE_FAIL);
    assert_eq!(tuner.current_plan, current_plan());
    assert_eq!(tuner.bandit.incumbent_idx, 1);
    assert_eq!(*materializer_calls.lock().unwrap(), 0);
}

proptest! {
    #[test]
    fn incumbent_bits_sum_is_non_decreasing(bits in prop::collection::vec(5_u32..100, 1..20)) {
        let mut tuner = LoomScopeTuner::new(
            AutotuneCache::load(&temp_path("proptest")).unwrap(),
            MatPlanConfig::default(),
        );
        let mut prior_promoted_bits = 0.0;
        for value in bits {
            let mut log = QueryLog::with_budgets(1, 1);
            let bits = f64::from(value) / 100.0;
            log.push(observation(lens(1), lens(2), 1_000, 700, 650, bits));
            let decision = tuner.on_query_tick(&log).unwrap();
            if let Some(promotion) = decision.promoted {
                prop_assert!(promotion.bits_after + 1e-12 >= prior_promoted_bits);
                prior_promoted_bits = promotion.bits_after;
            }
        }
    }
}

#[derive(Clone, Default)]
struct RecordingWriter {
    records: Arc<Mutex<Vec<LoomPromotionRecord>>>,
}

impl RecordingWriter {
    fn records(&self) -> Vec<LoomPromotionRecord> {
        self.records.lock().unwrap().clone()
    }
}

impl LoomPromotionWriter for RecordingWriter {
    fn write_autotune_promote(&mut self, event: &LoomPromotionRecord) -> Result<()> {
        self.records.lock().unwrap().push(event.clone());
        Ok(())
    }
}

#[derive(Default)]
struct CountingMaterializer {
    calls: Arc<Mutex<usize>>,
}

impl calyx_anneal::LoomMaterializer for CountingMaterializer {
    fn apply_plan(&self, _old_plan: &MatPlanConfig, _new_plan: &MatPlanConfig) -> Result<()> {
        *self.calls.lock().unwrap() += 1;
        Ok(())
    }
}

struct NoAssay;

impl calyx_anneal::AssayMetrics for NoAssay {
    fn signal_samples(&self) -> Result<Vec<calyx_anneal::SignalSample>> {
        Ok(Vec::new())
    }
}

fn current_plan() -> MatPlanConfig {
    MatPlanConfig::default()
}

fn eager_plan(a: LensId, b: LensId) -> MatPlanConfig {
    MatPlanConfig {
        eager_pairs: vec![(a, b)],
        indexed_concat_keys: Vec::new(),
    }
}

fn observation(
    a: LensId,
    b: LensId,
    lazy: u64,
    eager: u64,
    indexed: u64,
    bits: f64,
) -> QueryObservation {
    QueryObservation::new(a, b, lazy, eager, Some(indexed), bits)
}

fn lens(seed: u8) -> LensId {
    LensId::from_bytes([seed; 16])
}

fn temp_path(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "calyx_scope_loom_{label}_{}_{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_file(&path);
    path
}
