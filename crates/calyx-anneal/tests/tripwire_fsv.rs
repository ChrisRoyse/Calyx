use std::env;
use std::fs;
use std::path::PathBuf;

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use calyx_anneal::{
    CALYX_TRIPWIRE_INVALID_CONFIG, CALYX_TRIPWIRE_INVALID_METRIC, TripwireMetric, TripwireRegistry,
    TripwireResult, tripwire_config_path,
};
use fsv_support::{write_json, write_manifest};
use serde_json::json;

#[test]
#[ignore = "requires CALYX_ISSUE394_FSV_ROOT in a manual verification run"]
fn issue394_tripwire_registry_fsv() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE394_FSV_ROOT").expect("set CALYX_ISSUE394_FSV_ROOT"));
    let vault = root.join("vault");
    fs::create_dir_all(&vault).expect("create FSV vault");

    let mut registry = TripwireRegistry::load_from_vault(&vault).expect("load defaults");
    let before_status = registry.status();

    registry
        .set_tripwire(TripwireMetric::RecallAtK, 0.90, 0.05)
        .expect("persist recall threshold");

    let crossed_085 = registry
        .check(TripwireMetric::RecallAtK, 0.85)
        .expect("0.85 crosses recall");
    let status_085 = registry.status();
    let crossed_091 = registry
        .check(TripwireMetric::RecallAtK, 0.91)
        .expect("0.91 remains crossed inside band");
    let status_091 = registry.status();
    let recovered_097 = registry
        .check(TripwireMetric::RecallAtK, 0.97)
        .expect("0.97 clears recall");
    let status_097 = registry.status();

    assert!(matches!(crossed_085, TripwireResult::Crossed { .. }));
    assert!(matches!(crossed_091, TripwireResult::Crossed { .. }));
    assert_eq!(recovered_097, TripwireResult::Ok);

    let before_nan = registry.status();
    let nan_error = registry
        .check(TripwireMetric::RecallAtK, f64::NAN)
        .expect_err("NaN fails closed");
    let after_nan = registry.status();
    let before_inf = registry.status();
    let inf_error = registry
        .check(TripwireMetric::SearchP99, f64::INFINITY)
        .expect_err("Inf fails closed");
    let after_inf = registry.status();

    registry
        .set_tripwire(TripwireMetric::GuardFAR, 0.01, 0.0)
        .expect("zero hysteresis FAR");
    let before_far_crossed = registry.status();
    let far_crossed = registry
        .check(TripwireMetric::GuardFAR, 0.0101)
        .expect("FAR crosses");
    let after_far_crossed = registry.status();
    let before_far_recovered = registry.status();
    let far_recovered = registry
        .check(TripwireMetric::GuardFAR, 0.01)
        .expect("FAR clears at bound");
    let after_far_recovered = registry.status();

    let before_invalid_config = registry.status();
    let invalid_config = registry
        .set_tripwire(TripwireMetric::RecallAtK, 0.20, 0.21)
        .expect_err("invalid lower-bound hysteresis");
    let after_invalid_config = registry.status();

    assert_eq!(nan_error.code, CALYX_TRIPWIRE_INVALID_METRIC);
    assert_eq!(inf_error.code, CALYX_TRIPWIRE_INVALID_METRIC);
    assert_eq!(invalid_config.code, CALYX_TRIPWIRE_INVALID_CONFIG);
    assert!(matches!(far_crossed, TripwireResult::Crossed { .. }));
    assert_eq!(far_recovered, TripwireResult::Ok);

    let config_path = tripwire_config_path(&vault);
    let config_toml = fs::read_to_string(&config_path).expect("read persisted TOML");
    assert!(config_toml.contains("[thresholds.recall_at_k]"));
    assert!(config_toml.contains("hysteresis = 0.05"));

    write_json(
        &root.join("tripwire-state-sequence.json"),
        &json!({
            "surface": "config.tripwire",
            "source_of_truth": "vault .anneal/tripwire.toml plus in-memory TripwireRegistry state",
            "config_path": config_path,
            "before_status": before_status,
            "steps": [
                {"input": 0.85, "expected": "crossed", "result": crossed_085, "status": status_085},
                {"input": 0.91, "expected": "still_crossed_inside_hysteresis", "result": crossed_091, "status": status_091},
                {"input": 0.97, "expected": "recovered", "result": recovered_097, "status": status_097}
            ]
        }),
    );
    write_json(
        &root.join("tripwire-edge-readback.json"),
        &json!({
            "edges": [
                {
                    "case": "nan_metric",
                    "trigger": "check(recall_at_k, NaN)",
                    "expected": "CALYX_TRIPWIRE_INVALID_METRIC and unchanged state",
                    "before_status": before_nan,
                    "error_code": nan_error.code,
                    "after_status": after_nan
                },
                {
                    "case": "inf_metric",
                    "trigger": "check(search_p99, Inf)",
                    "expected": "CALYX_TRIPWIRE_INVALID_METRIC and unchanged state",
                    "before_status": before_inf,
                    "error_code": inf_error.code,
                    "after_status": after_inf
                },
                {
                    "case": "zero_hysteresis_cross",
                    "trigger": "check(guard_far, 0.0101)",
                    "expected": "Crossed at bound 0.01 with hysteresis 0.0",
                    "before_status": before_far_crossed,
                    "result": far_crossed,
                    "after_status": after_far_crossed
                },
                {
                    "case": "zero_hysteresis_recover",
                    "trigger": "check(guard_far, 0.01)",
                    "expected": "Ok at exact bound with hysteresis 0.0",
                    "before_status": before_far_recovered,
                    "result": far_recovered,
                    "after_status": after_far_recovered
                },
                {
                    "case": "invalid_lower_bound_config",
                    "trigger": "set_tripwire(recall_at_k, 0.20, 0.21)",
                    "expected": "CALYX_TRIPWIRE_INVALID_CONFIG and unchanged persisted state",
                    "before_status": before_invalid_config,
                    "error_code": invalid_config.code,
                    "after_status": after_invalid_config
                }
            ],
            "after_edges_status": registry.status()
        }),
    );
    write_manifest(
        &root,
        &[
            root.join("vault/.anneal/tripwire.toml"),
            root.join("tripwire-state-sequence.json"),
            root.join("tripwire-edge-readback.json"),
        ],
    );
}
