//! Issue #237 - first-light artifact-chain audit.
//!
//! Source of truth: capture state JSON, CalyxNative forecast JSON, score artifacts, and retune
//! reports read back from disk.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::TrustTag;
use calyx_core::FixedClock;
use calyx_ledger::{DirectoryLedgerStore, LedgerAppender};
use calyx_poly::blend_relearning::{BlendRelearningRequest, BlendWeightObservation};
use calyx_poly::calibration_refit::{CalibrationRefitObservation, CalibrationRefitRequest};
use calyx_poly::calyx_native::{
    CalyxNativeRequest, produce_calyx_native_forecast, write_calyx_native_forecast,
};
use calyx_poly::crypto_capture_harness::{
    CRYPTO_CAPTURE_HARNESS_SCHEMA_VERSION, CRYPTO_CAPTURE_STATE_FILE, CryptoCaptureHarnessState,
    CryptoCaptureRecord, CryptoCapturedSnapshotRef,
};
use calyx_poly::first_light_audit::{
    ERR_FIRST_LIGHT_BASELINE_ONLY, ERR_FIRST_LIGHT_LOOKAHEAD, ERR_FIRST_LIGHT_RETUNE_NO_MOVE,
    FirstLightAuditRequest, run_first_light_audit,
};
use calyx_poly::forecast_calibration::{fit_calibration_slope, horizon_bucket};
use calyx_poly::superiority::SuperiorityTiers;
use calyx_poly::{
    BlendRelearningRun, CalibrationRefitRun, ComponentKind, ForecastComponent,
    ForecastScoreRequest, ForecastSource, PendingForecastEntry, PendingForecastStatus,
    ResolvedOutcome, run_blend_relearning, run_calibration_refit, write_forecast_score_artifacts,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const FORECAST_ID: &str = "crypto-snapshot-b000af389508a59ae29f653e195c434e";
const CONDITION_ID: &str = "0x39ea14226927f879d93550628e70f22bc444915cea787c5734c53e89d5cf6d20";
const TOKEN_ID: &str =
    "34747254630927017064599589941321309211596494400768268440049646273862919127907";
const FORECAST_TS: u64 = 100;
const RESOLVED_TS: u64 = 200;

#[test]
fn issue237_first_light_audit_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE237_FSV_ROOT", "issue237-first-light-audit");
    assert_c_drive(&root);
    reset_dir(&root);

    let happy = build_case(&root.join("happy"), Edge::Happy);
    let run = run_first_light_audit(&happy.request()).expect("happy first-light audit");
    assert!(run.report.passed);
    assert_eq!(run.report.forecast_id, FORECAST_ID);
    assert!(run.report.no_lookahead);
    assert!(run.report.retune_moved);
    assert!(run.report.confidence < 1.0);

    let baseline = build_case(&root.join("edge-baseline"), Edge::BaselineOnly);
    let baseline_err = run_first_light_audit(&baseline.request())
        .expect_err("baseline-only capture must fail first-light audit");
    assert_eq!(baseline_err.code(), ERR_FIRST_LIGHT_BASELINE_ONLY);

    let lookahead = build_case(&root.join("edge-lookahead"), Edge::Lookahead);
    let lookahead_err = run_first_light_audit(&lookahead.request())
        .expect_err("lookahead timing must fail first-light audit");
    assert_eq!(lookahead_err.code(), ERR_FIRST_LIGHT_LOOKAHEAD);

    let no_retune = build_case(&root.join("edge-no-retune"), Edge::NoRetuneMove);
    let no_retune_err = run_first_light_audit(&no_retune.request())
        .expect_err("non-moving retune must fail first-light audit");
    assert_eq!(no_retune_err.code(), ERR_FIRST_LIGHT_RETUNE_NO_MOVE);

    let readback: Value = serde_json::from_slice(&fs::read(&run.report_path).expect("read report"))
        .expect("decode report");
    assert_eq!(readback["forecast_id"], json!(FORECAST_ID));

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 237,
        "proof_claim": "A claimed first-light chain is accepted only when the captured pending entry is CalyxNative, the persisted CalyxNative forecast artifact is the score artifact source, forecast timing predates resolution, score artifacts read back, and retune reports show real weight/slope movement.",
        "minimum_sufficient_proof_corpus": {
            "happy_chains": 1,
            "edge_cases": 3,
            "why_this_is_sufficient": "One complete binary forecast chain proves the required artifact links; baseline-only, lookahead, and no-retune edges cover the false-success risks found while waiting for #238.",
            "why_smaller_is_insufficient": "Zero happy chains would not prove accepted readback; omitting any edge would leave a known #237 false-success mode untested.",
            "why_larger_is_wasteful": "More markets would repeat the same artifact-chain checks without proving a new first-light invariant."
        },
        "source_of_truth": "capture state JSON, CalyxNative forecast JSON, score artifact directory, blend relearning report, and calibration refit report",
        "happy_report": readback,
        "edges": {
            "baseline_only": baseline_err.code(),
            "lookahead": lookahead_err.code(),
            "no_retune_move": no_retune_err.code()
        },
        "physical_files": files,
        "passed": true
    });
    let summary_path = root.join("issue237_first_light_audit_fsv_report.json");
    write_json(&summary_path, &summary);
    let summary_readback: Value =
        serde_json::from_slice(&fs::read(&summary_path).expect("read summary"))
            .expect("decode summary");
    assert_eq!(summary_readback, summary);
    write_blake3sums(&root);
}

#[derive(Clone, Copy)]
enum Edge {
    Happy,
    BaselineOnly,
    Lookahead,
    NoRetuneMove,
}

struct CasePaths {
    report_dir: PathBuf,
    capture_state_path: PathBuf,
    forecast_path: PathBuf,
    score_manifest_path: PathBuf,
    blend_path: PathBuf,
    calibration_path: PathBuf,
}

impl CasePaths {
    fn request(&self) -> FirstLightAuditRequest<'_> {
        FirstLightAuditRequest {
            report_dir: &self.report_dir,
            capture_state_path: &self.capture_state_path,
            calyx_native_forecast_path: &self.forecast_path,
            score_manifest_path: &self.score_manifest_path,
            blend_relearning_report_path: &self.blend_path,
            calibration_refit_report_path: &self.calibration_path,
        }
    }
}

fn build_case(root: &Path, edge: Edge) -> CasePaths {
    reset_dir(root);
    let forecast = forecast(root);
    let forecast_path = write_calyx_native_forecast(&root.join("forecast"), &forecast)
        .expect("write CalyxNative forecast");
    let forecast_hash = blake3_hex(&forecast_path);
    let capture_state_path = write_capture_state(root, &forecast, edge);
    let score_manifest_path = write_score(root, &forecast_hash, edge);
    let blend = if matches!(edge, Edge::NoRetuneMove) {
        write_equal_blend(root)
    } else {
        write_moved_blend(root)
    };
    let calibration = write_calibration(root);
    CasePaths {
        report_dir: root.join("audit"),
        capture_state_path,
        forecast_path,
        score_manifest_path,
        blend_path: blend.report_path,
        calibration_path: calibration.report_path,
    }
}

fn forecast(root: &Path) -> calyx_poly::calyx_native::CalyxNativeForecast {
    let slope = fit_calibration_slope("crypto", horizon_bucket(3_600.0), &calibration_pairs())
        .expect("fit forecast calibration");
    let req = CalyxNativeRequest {
        domain: "crypto".to_string(),
        condition_id: CONDITION_ID.to_string(),
        token_id: TOKEN_ID.to_string(),
        horizon_bucket: horizon_bucket(3_600.0).to_string(),
        components: vec![
            component(ComponentKind::KnnBaseRate, 0.72, 0.8),
            component(ComponentKind::BitsVote, 0.68, 0.6),
        ],
        calibration: Some(slope),
        raw_confidence: 0.8,
        oracle_flakiness: 0.1,
        oracle_validity: 0.9,
        panel_bits: 1.0,
        anchor_entropy_bits: 1.0,
        superiority_tiers: strong_tiers(),
        evidence: None,
    };
    let forecast = produce_calyx_native_forecast(&req, &FixedClock::new(FORECAST_TS))
        .expect("produce CalyxNative forecast");
    write_json(
        &root.join("forecast-source-summary.json"),
        &json!({
            "p_model": forecast.p_model,
            "confidence": forecast.confidence,
            "provenance_hash": forecast.provenance_hash
        }),
    );
    forecast
}

fn write_capture_state(
    root: &Path,
    forecast: &calyx_poly::calyx_native::CalyxNativeForecast,
    edge: Edge,
) -> PathBuf {
    let forecast_ts = if matches!(edge, Edge::Lookahead) {
        RESOLVED_TS
    } else {
        FORECAST_TS
    };
    let source = if matches!(edge, Edge::BaselineOnly) {
        ForecastSource::BaselineMarket
    } else {
        ForecastSource::CalyxNative
    };
    let pending = PendingForecastEntry {
        forecast_id: FORECAST_ID.to_string(),
        source,
        condition_id: CONDITION_ID.to_string(),
        token_id: TOKEN_ID.to_string(),
        outcome_index: 0,
        domain: "crypto".to_string(),
        horizon_bucket: "lt_1h".to_string(),
        forecast_version: 1,
        p_model: forecast.p_model,
        confidence: forecast.confidence,
        forecast_ts,
        provenance_hash: forecast.provenance_hash.clone(),
        status: PendingForecastStatus::Pending,
        registered_ledger_seq: Some(0),
        terminal_ledger_seq: None,
        terminal_resolution_id: None,
        terminal_actual_win: None,
    };
    let state = CryptoCaptureHarnessState {
        schema_version: CRYPTO_CAPTURE_HARNESS_SCHEMA_VERSION.to_string(),
        domain: "crypto".to_string(),
        interval_secs: 60,
        captures: vec![CryptoCaptureRecord {
            capture_id: "capture237".to_string(),
            due_slot: 1,
            captured_ts: FORECAST_TS,
            market_id: CONDITION_ID.to_string(),
            condition_id: CONDITION_ID.to_string(),
            token_count: 1,
            run_hash_blake3: "a".repeat(64),
            snapshots: vec![CryptoCapturedSnapshotRef {
                cx_id: "cx237".to_string(),
                token_id: TOKEN_ID.to_string(),
                forecast_id: FORECAST_ID.to_string(),
                forecast_artifact_path: None,
                forecast_artifact_blake3: None,
                outcome_index: 0,
                forecast_ts,
                pending_entry: pending,
            }],
        }],
        matured_resolutions: Vec::new(),
    };
    let path = calyx_poly::diagnostics_store::write_json(root, CRYPTO_CAPTURE_STATE_FILE, &state)
        .expect("write capture state");
    let readback: CryptoCaptureHarnessState =
        calyx_poly::diagnostics_store::read_json(&path).expect("read capture state");
    assert_eq!(readback, state);
    path
}

fn write_score(root: &Path, forecast_hash: &str, edge: Edge) -> PathBuf {
    let score_root = root.join("scores");
    let ledger_dir = root.join("score-ledger");
    let mut ledger = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).expect("open score ledger"),
        FixedClock::new(RESOLVED_TS + 20),
    )
    .expect("open ledger");
    let forecast_ts = if matches!(edge, Edge::Lookahead) {
        RESOLVED_TS
    } else {
        FORECAST_TS
    };
    let request = ForecastScoreRequest {
        score_id: "score237happy".to_string(),
        forecast_id: FORECAST_ID.to_string(),
        forecast_version: 1,
        current_forecast_version: 1,
        market_id: CONDITION_ID.to_string(),
        outcome_id: TOKEN_ID.to_string(),
        source: ForecastSource::CalyxNative,
        provider: None,
        probability: score_probability(root),
        confidence: score_confidence(root),
        forecast_ts,
        scored_ts: RESOLVED_TS + 20,
        horizon_secs: RESOLVED_TS.saturating_sub(forecast_ts),
        sufficiency_state: "sufficient".to_string(),
        previous_probability: Some(0.50),
        forecast_artifact_hash: forecast_hash.to_string(),
        outcome: ResolvedOutcome {
            outcome_id: TOKEN_ID.to_string(),
            resolved: true,
            actual_win: true,
            resolved_ts: RESOLVED_TS,
            source: "gamma-closed-derived".to_string(),
            version: 1,
        },
        calibration_bin_count: 10,
    };
    write_forecast_score_artifacts(&score_root, &mut ledger, &request).expect("write score");
    score_root.join("score237happy").join("manifest.json")
}

fn score_probability(root: &Path) -> f64 {
    let value: Value =
        serde_json::from_slice(&fs::read(root.join("forecast-source-summary.json")).unwrap())
            .unwrap();
    value["p_model"].as_f64().unwrap()
}

fn score_confidence(root: &Path) -> f64 {
    let value: Value =
        serde_json::from_slice(&fs::read(root.join("forecast-source-summary.json")).unwrap())
            .unwrap();
    value["confidence"].as_f64().unwrap()
}

fn write_moved_blend(root: &Path) -> BlendRelearningRun {
    run_blend_relearning(&BlendRelearningRequest {
        out_dir: &root.join("blend"),
        domain: "crypto",
        horizon_bucket: "lt_1h",
        as_of_millis: (RESOLVED_TS + 1) * 1000,
        min_samples_per_component: 2,
        observations: vec![
            blend_obs(ComponentKind::KnnBaseRate, 0.9, true, 1),
            blend_obs(ComponentKind::KnnBaseRate, 0.1, false, 2),
            blend_obs(ComponentKind::BitsVote, 0.6, true, 1),
            blend_obs(ComponentKind::BitsVote, 0.4, false, 2),
        ],
    })
    .expect("moved blend")
}

fn write_equal_blend(root: &Path) -> BlendRelearningRun {
    run_blend_relearning(&BlendRelearningRequest {
        out_dir: &root.join("blend-equal"),
        domain: "crypto",
        horizon_bucket: "lt_1h",
        as_of_millis: (RESOLVED_TS + 1) * 1000,
        min_samples_per_component: 2,
        observations: vec![
            blend_obs(ComponentKind::KnnBaseRate, 0.8, true, 1),
            blend_obs(ComponentKind::KnnBaseRate, 0.2, false, 2),
            blend_obs(ComponentKind::BitsVote, 0.8, true, 1),
            blend_obs(ComponentKind::BitsVote, 0.2, false, 2),
        ],
    })
    .expect("equal blend")
}

fn write_calibration(root: &Path) -> CalibrationRefitRun {
    run_calibration_refit(&CalibrationRefitRequest {
        out_dir: &root.join("calibration"),
        domain: "crypto",
        horizon_bucket: "lt_1h",
        previous_version: Some("crypto:lt_1h:prior"),
        as_of_millis: (RESOLVED_TS + 1) * 1000,
        observations: calibration_pairs()
            .into_iter()
            .map(|(p_raw, outcome_yes)| CalibrationRefitObservation {
                p_raw,
                outcome_yes,
                resolved_at_millis: RESOLVED_TS * 1000,
            })
            .collect(),
    })
    .expect("calibration refit")
}

fn component(kind: ComponentKind, p: f64, reliability: f64) -> ForecastComponent {
    ForecastComponent::new(kind, p, reliability, 40, TrustTag::Trusted, "issue237")
        .expect("component")
}

fn blend_obs(
    component: ComponentKind,
    p_yes: f64,
    outcome_yes: bool,
    observed_at_millis: u64,
) -> BlendWeightObservation {
    BlendWeightObservation {
        component,
        p_yes,
        outcome_yes,
        observed_at_millis,
    }
}

fn calibration_pairs() -> Vec<(f64, bool)> {
    let mut pairs = Vec::new();
    for i in 0..20 {
        pairs.push((0.6, i % 5 != 0));
        pairs.push((0.4, i % 5 == 0));
    }
    pairs
}

fn strong_tiers() -> SuperiorityTiers {
    SuperiorityTiers {
        oracle_self_consistency: 0.9,
        panel_sufficient: true,
        kernel_recall_ratio: 0.97,
        min_kernel_recall_ratio: 0.95,
        calibrated: true,
        goodhart_defended: true,
        mistake_closed: true,
    }
}

fn blake3_hex(path: &Path) -> String {
    blake3::hash(&fs::read(path).expect("read hash file"))
        .to_hex()
        .to_string()
}

fn assert_c_drive(path: &Path) {
    support::assert_host_fsv_root(path, "FSV root");
}
