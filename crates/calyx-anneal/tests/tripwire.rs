use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use calyx_anneal::{
    CALYX_TRIPWIRE_INVALID_CONFIG, CALYX_TRIPWIRE_INVALID_METRIC, TripwireMetric, TripwireRegistry,
    TripwireResult, read_tripwire_config_from_vault, tripwire_config_path,
};
use proptest::prelude::*;

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "calyx-tripwire-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp root");
        Self(path)
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn recall_tripwire_crosses_and_recovers() {
    let root = TestRoot::new("recall");
    let mut registry = TripwireRegistry::load_from_vault(root.path()).expect("load defaults");
    registry
        .set_tripwire(TripwireMetric::RecallAtK, 0.90, 0.05)
        .expect("persist recall threshold");

    assert_eq!(
        registry.check(TripwireMetric::RecallAtK, 0.85).unwrap(),
        TripwireResult::Crossed {
            metric: TripwireMetric::RecallAtK,
            threshold: 0.90,
            hysteresis: 0.05,
        }
    );
    assert_eq!(
        registry.check(TripwireMetric::RecallAtK, 0.95).unwrap(),
        TripwireResult::Ok
    );

    let recall = registry
        .status()
        .into_iter()
        .find(|status| status.metric == TripwireMetric::RecallAtK)
        .expect("recall status");
    assert_eq!(recall.state.last_value, 0.95);
    assert!(!recall.state.crossed);
}

#[test]
fn recall_hysteresis_band_prevents_oscillation() {
    let root = TestRoot::new("hysteresis");
    let mut registry = TripwireRegistry::load_from_vault(root.path()).expect("load defaults");
    registry
        .set_tripwire(TripwireMetric::RecallAtK, 0.90, 0.05)
        .expect("persist recall threshold");

    assert!(matches!(
        registry.check(TripwireMetric::RecallAtK, 0.85).unwrap(),
        TripwireResult::Crossed { .. }
    ));
    assert!(matches!(
        registry.check(TripwireMetric::RecallAtK, 0.91).unwrap(),
        TripwireResult::Crossed { .. }
    ));
    assert_eq!(
        registry.check(TripwireMetric::RecallAtK, 0.96).unwrap(),
        TripwireResult::Ok
    );
}

#[test]
fn edge_cases_fail_closed_or_reduce_to_simple_threshold() {
    let root = TestRoot::new("edges");
    let mut registry = TripwireRegistry::load_from_vault(root.path()).expect("load defaults");

    let nan = registry
        .check(TripwireMetric::RecallAtK, f64::NAN)
        .expect_err("NaN rejects");
    assert_eq!(nan.code, CALYX_TRIPWIRE_INVALID_METRIC);

    let inf = registry
        .check(TripwireMetric::SearchP99, f64::INFINITY)
        .expect_err("Inf rejects");
    assert_eq!(inf.code, CALYX_TRIPWIRE_INVALID_METRIC);

    registry
        .set_tripwire(TripwireMetric::GuardFAR, 0.01, 0.0)
        .expect("zero hysteresis threshold");
    assert!(matches!(
        registry.check(TripwireMetric::GuardFAR, 0.0101).unwrap(),
        TripwireResult::Crossed { .. }
    ));
    assert_eq!(
        registry.check(TripwireMetric::GuardFAR, 0.01).unwrap(),
        TripwireResult::Ok
    );
}

#[test]
fn invalid_lower_bound_hysteresis_is_config_error() {
    let root = TestRoot::new("invalid-config");
    let mut registry = TripwireRegistry::load_from_vault(root.path()).expect("load defaults");

    let error = registry
        .set_tripwire(TripwireMetric::RecallAtK, 0.20, 0.21)
        .expect_err("hysteresis larger than lower bound");

    assert_eq!(error.code, CALYX_TRIPWIRE_INVALID_CONFIG);
}

#[test]
fn set_tripwire_persists_toml_source_of_truth() {
    let root = TestRoot::new("persist");
    let mut registry = TripwireRegistry::load_from_vault(root.path()).expect("load defaults");
    registry
        .set_tripwire(TripwireMetric::RecallAtK, 0.90, 0.05)
        .expect("persist recall threshold");

    let config_path = tripwire_config_path(root.path());
    let toml = fs::read_to_string(&config_path).expect("read tripwire.toml");
    assert!(toml.contains("[thresholds.recall_at_k]"));
    assert!(toml.contains("bound = 0.9"));
    assert!(toml.contains("hysteresis = 0.05"));

    let readback = read_tripwire_config_from_vault(root.path()).expect("parse readback");
    assert_eq!(readback.config_path, config_path);
    assert_eq!(readback.thresholds.len(), 5);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn lower_bound_stays_crossed_inside_hysteresis(
        bound in 0.01_f64..1000.0,
        hysteresis_fraction in 0.001_f64..0.50,
        recovery_fraction in 0.0_f64..0.999,
    ) {
        let root = TestRoot::new("prop");
        let mut registry = TripwireRegistry::load_from_vault(root.path()).unwrap();
        let hysteresis = bound * hysteresis_fraction;
        registry
            .set_tripwire(TripwireMetric::RecallAtK, bound, hysteresis)
            .unwrap();

        registry.check(TripwireMetric::RecallAtK, bound * 0.5).unwrap();
        let inside_band = bound + (hysteresis * recovery_fraction);
        let result = registry.check(TripwireMetric::RecallAtK, inside_band).unwrap();

        match result {
            TripwireResult::Crossed { .. } => {}
            other => prop_assert!(false, "expected crossed result, got {:?}", other),
        }
    }
}
