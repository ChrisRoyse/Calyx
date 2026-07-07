//! Issue #237 - targeted live first-light candidate check.
//!
//! This ignored test uses the one live pre-resolution CalyxNative capture from #241. It reads the
//! exact Gamma market after the end window, joins the copied capture state if Gamma is closed with a
//! clean winner, writes real score artifacts, and only runs the first-light audit if real retune
//! reports can be produced from the available resolved observations.

#[path = "support/issue237_live_first_light_support.rs"]
mod live_support;
#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::FixedClock;
use calyx_ledger::{DirectoryLedgerStore, LedgerAppender};
use calyx_poly::blend_relearning::{BlendRelearningRequest, BlendWeightObservation};
use calyx_poly::calibration_refit::{CalibrationRefitObservation, CalibrationRefitRequest};
use calyx_poly::calyx_native::read_calyx_native_forecast;
use calyx_poly::crypto_capture_harness::{
    CRYPTO_CAPTURE_STATE_FILE, CRYPTO_PRE_RESOLUTION_CORPUS_FILE, CryptoCaptureHarnessState,
    CryptoPreResolutionPair, join_crypto_capture_resolution, read_crypto_capture_state,
};
use calyx_poly::first_light_audit::{FirstLightAuditRequest, run_first_light_audit};
use calyx_poly::forecast_calibration::ERR_CAL_SAMPLES;
use calyx_poly::{
    ForecastScoreRequest, ForecastSource, PendingForecastRegister, Resolution, ResolvedOutcome,
    run_blend_relearning, run_calibration_refit, write_forecast_score_artifacts,
};
use live_support::{
    assert_c_drive, blake3_hex, copy_tree, gamma_resolution_readback, get_market_body, sha256_hex,
    source_capture_root,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const MARKET_ID: &str = "2744242";
const CONDITION_ID: &str = "0x39ea14226927f879d93550628e70f22bc444915cea787c5734c53e89d5cf6d20";
const CAPTURE_ROOT_NAME: &str = "issue241_live_calyx_native_capture_mode_20260707_v3";
const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue241-calyx-native-seam";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FirstLightDecision {
    SourceStillOpen,
    NoCleanWinner,
    ScoredButRetuneNeedsMoreRealOutcomes,
    AuditPassed,
}

#[test]
#[ignore = "requires live Gamma read and the prior #241 live CalyxNative capture evidence root"]
fn issue237_live_first_light_candidate_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE237_LIVE_FIRST_LIGHT_ROOT",
        "issue237-live-first-light",
    );
    assert_c_drive(&root);
    reset_dir(&root);

    let source_root = source_capture_root(CAPTURE_ROOT_NAME);
    assert_c_drive(&source_root);
    let harness_root = root.join("harness-copy");
    copy_tree(&source_root, &harness_root);

    let before_state = read_crypto_capture_state(&harness_root.join(CRYPTO_CAPTURE_STATE_FILE))
        .expect("read copied CalyxNative capture state");
    assert_single_calyx_native_capture(&before_state);
    let artifact_checks = assert_forecast_artifact_hashes(&before_state);

    let raw_path = root.join("gamma-market-2744242-body.json");
    let body = get_market_body(MARKET_ID);
    fs::write(&raw_path, &body).expect("write Gamma body");
    let raw_hash = sha256_hex(&body);
    let gamma = gamma_resolution_readback(
        &serde_json::from_slice::<Value>(&body).expect("decode Gamma body"),
        MARKET_ID,
        CONDITION_ID,
    )
    .expect("parse Gamma resolution fields");

    let mut edge_cases = Vec::new();
    let (decision, proof) = if gamma.closed != Some(true) {
        let after = read_crypto_capture_state(&harness_root.join(CRYPTO_CAPTURE_STATE_FILE))
            .expect("read unchanged open-source state");
        assert_eq!(after, before_state);
        edge_cases.push(json!({
            "case": "source_still_open",
            "state_unchanged": true,
            "capture_count": after.captures.len()
        }));
        (FirstLightDecision::SourceStillOpen, Value::Null)
    } else if gamma.resolution.is_none() {
        let after = read_crypto_capture_state(&harness_root.join(CRYPTO_CAPTURE_STATE_FILE))
            .expect("read unchanged no-winner state");
        assert_eq!(after, before_state);
        edge_cases.push(json!({
            "case": "closed_without_clean_winner",
            "state_unchanged": true,
            "outcome_prices": gamma.outcome_prices
        }));
        (FirstLightDecision::NoCleanWinner, Value::Null)
    } else {
        let resolution = gamma.resolution.clone().expect("resolution");
        assert!(resolution.resolved_ts > before_state.captures[0].captured_ts);
        let joined = join_resolution(&harness_root, &resolution);
        let score = write_real_scores(
            &root,
            &joined.state,
            &joined.record.pairs,
            resolution.resolved_ts,
        );
        let blend = write_real_blend(
            &root,
            &joined.state,
            &joined.record.pairs,
            resolution.resolved_ts,
        );
        let calibration =
            run_real_only_calibration(&root, &joined.record.pairs, resolution.resolved_ts);
        match calibration {
            Ok(calibration_path) => {
                let audit = run_first_light_audit(&FirstLightAuditRequest {
                    report_dir: &root.join("audit"),
                    capture_state_path: &joined.state_path,
                    calyx_native_forecast_path: &score.winning_forecast_path,
                    score_manifest_path: &score.winning_manifest_path,
                    blend_relearning_report_path: &blend.report_path,
                    calibration_refit_report_path: &calibration_path,
                })
                .expect("run live first-light audit");
                (
                    FirstLightDecision::AuditPassed,
                    json!({
                        "join": joined.record,
                        "corpus_path": joined.corpus_path.display().to_string(),
                        "scores": score.score_readbacks,
                        "blend": blend.report,
                        "calibration_path": calibration_path.display().to_string(),
                        "audit": audit.report
                    }),
                )
            }
            Err(err) if err.code() == ERR_CAL_SAMPLES => {
                edge_cases.push(json!({
                    "case": "real_retune_calibration_floor",
                    "error_code": err.code(),
                    "real_resolved_observations": joined.record.pairs.len(),
                    "required_minimum": calyx_poly::forecast_calibration::MIN_CALIBRATION_SAMPLES
                }));
                (
                    FirstLightDecision::ScoredButRetuneNeedsMoreRealOutcomes,
                    json!({
                        "join": joined.record,
                        "corpus_path": joined.corpus_path.display().to_string(),
                        "scores": score.score_readbacks,
                        "blend": blend.report,
                        "calibration_error": err.code()
                    }),
                )
            }
            Err(err) => panic!("unexpected calibration error: {err:?}"),
        }
    };

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 237,
        "proof_claim": "Determine whether the exact live pre-resolution CalyxNative capture from #241 can be resolved, scored, retuned, and accepted by the first-light audit without adding synthetic outcomes or broad replay.",
        "minimum_sufficient_proof_corpus": {
            "market_count": 1,
            "market_id": MARKET_ID,
            "condition_id": CONDITION_ID,
            "copied_capture_count": before_state.captures.len(),
            "copied_snapshot_count": before_state.captures[0].snapshots.len(),
            "why_this_is_sufficient": "The #241 live root contains exactly one real pre-resolution CalyxNative capture for this market; this one Gamma read plus copied capture root is the smallest corpus that can prove whether it can now become first-light evidence.",
            "why_smaller_is_insufficient": "Zero market reads or no copied capture state would not inspect the real pending CalyxNative source of truth.",
            "why_larger_is_wasteful": "Additional markets or backfill would not prove whether this exact first-light candidate resolves, scores, and retunes."
        },
        "source_of_truth": [
            raw_path.display().to_string(),
            harness_root.join(CRYPTO_CAPTURE_STATE_FILE).display().to_string(),
            harness_root.join("live-capture-vault").display().to_string()
        ],
        "raw_sha256": raw_hash,
        "artifact_checks": artifact_checks,
        "gamma_readback": gamma,
        "decision": decision,
        "proof": proof,
        "edge_cases": edge_cases,
        "physical_files": files
    });
    let report_path = root.join("issue237_live_first_light_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(readback, report);
    write_blake3sums(&root);
    println!(
        "ISSUE237_LIVE_FIRST_LIGHT_READBACK={}",
        report_path.display()
    );
}

struct ScoreWriteback {
    winning_manifest_path: PathBuf,
    winning_forecast_path: PathBuf,
    score_readbacks: Vec<Value>,
}

fn write_real_scores(
    root: &Path,
    state: &CryptoCaptureHarnessState,
    pairs: &[CryptoPreResolutionPair],
    resolved_ts: u64,
) -> ScoreWriteback {
    let mut ledger = LedgerAppender::open(
        DirectoryLedgerStore::open(root.join("score-ledger")).expect("score ledger"),
        FixedClock::new(resolved_ts + 20),
    )
    .expect("open score ledger");
    let mut score_readbacks = Vec::new();
    let mut winning_manifest_path = None;
    let mut winning_forecast_path = None;
    for pair in pairs {
        let snapshot = snapshot_for_pair(state, pair);
        let forecast_path = PathBuf::from(snapshot.forecast_artifact_path.as_ref().unwrap());
        let forecast = read_calyx_native_forecast(&forecast_path).expect("read forecast artifact");
        let forecast_hash = blake3_hex(&forecast_path);
        let score_id = format!("score237live{}", pair.outcome_index);
        let manifest = write_forecast_score_artifacts(
            &root.join("scores"),
            &mut ledger,
            &ForecastScoreRequest {
                score_id: score_id.clone(),
                forecast_id: pair.forecast_id.clone(),
                forecast_version: 1,
                current_forecast_version: 1,
                market_id: pair.condition_id.clone(),
                outcome_id: pair.token_id.clone(),
                source: ForecastSource::CalyxNative,
                provider: None,
                probability: forecast.p_model,
                confidence: forecast.confidence,
                forecast_ts: pair.forecast_ts,
                scored_ts: resolved_ts + 20,
                horizon_secs: resolved_ts.saturating_sub(pair.forecast_ts),
                sufficiency_state: if forecast.admissible {
                    "sufficient"
                } else {
                    "refused"
                }
                .to_string(),
                previous_probability: Some(0.5),
                forecast_artifact_hash: forecast_hash,
                outcome: ResolvedOutcome {
                    outcome_id: pair.token_id.clone(),
                    resolved: true,
                    actual_win: pair.actual_win,
                    resolved_ts,
                    source: "gamma-closed-derived".to_string(),
                    version: 1,
                },
                calibration_bin_count: 10,
            },
        )
        .expect("write real score artifacts");
        let score_dir = root.join("scores").join(&score_id);
        let readback: Value =
            serde_json::from_slice(&fs::read(score_dir.join("manifest.json")).unwrap()).unwrap();
        score_readbacks.push(readback);
        if pair.actual_win {
            winning_manifest_path = Some(score_dir.join("manifest.json"));
            winning_forecast_path = Some(forecast_path);
        }
        assert_eq!(manifest.forecast_id, pair.forecast_id);
    }
    ScoreWriteback {
        winning_manifest_path: winning_manifest_path.expect("one winning score"),
        winning_forecast_path: winning_forecast_path.expect("one winning forecast"),
        score_readbacks,
    }
}

fn write_real_blend(
    root: &Path,
    state: &CryptoCaptureHarnessState,
    pairs: &[CryptoPreResolutionPair],
    resolved_ts: u64,
) -> calyx_poly::blend_relearning::BlendRelearningRun {
    let mut observations = Vec::new();
    for pair in pairs {
        let snapshot = snapshot_for_pair(state, pair);
        let forecast = read_calyx_native_forecast(&PathBuf::from(
            snapshot.forecast_artifact_path.as_ref().unwrap(),
        ))
        .expect("read forecast for blend");
        for component in forecast.components {
            observations.push(BlendWeightObservation {
                component: component.kind,
                p_yes: component.p,
                outcome_yes: pair.actual_win,
                observed_at_millis: resolved_ts * 1000,
            });
        }
    }
    run_blend_relearning(&BlendRelearningRequest {
        out_dir: &root.join("blend"),
        domain: "crypto",
        horizon_bucket: "pre_resolution",
        as_of_millis: (resolved_ts + 1) * 1000,
        min_samples_per_component: 1,
        observations,
    })
    .expect("real blend relearning")
}

fn run_real_only_calibration(
    root: &Path,
    pairs: &[CryptoPreResolutionPair],
    resolved_ts: u64,
) -> calyx_poly::Result<PathBuf> {
    run_calibration_refit(&CalibrationRefitRequest {
        out_dir: &root.join("calibration-real-only"),
        domain: "crypto",
        horizon_bucket: "pre_resolution",
        previous_version: None,
        as_of_millis: (resolved_ts + 1) * 1000,
        observations: pairs
            .iter()
            .map(|pair| CalibrationRefitObservation {
                p_raw: pair.p_model,
                outcome_yes: pair.actual_win,
                resolved_at_millis: resolved_ts * 1000,
            })
            .collect(),
    })
    .map(|run| run.report_path)
}

fn join_resolution(
    harness_root: &Path,
    resolution: &Resolution,
) -> calyx_poly::crypto_capture_harness::CryptoCaptureResolutionRun {
    let vault = AsterVault::open(
        harness_root.join("live-capture-vault"),
        VAULT_ID.parse().unwrap(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open copied live capture vault");
    let mut register = PendingForecastRegister::default();
    let joined =
        join_crypto_capture_resolution(&vault, &mut register, harness_root, resolution, false)
            .expect("join CalyxNative capture resolution");
    let corpus: Vec<CryptoPreResolutionPair> = serde_json::from_slice(
        &fs::read(harness_root.join(CRYPTO_PRE_RESOLUTION_CORPUS_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(corpus, joined.record.pairs);
    assert_eq!(joined.record.pairs.len(), 2);
    joined
}

fn snapshot_for_pair<'a>(
    state: &'a CryptoCaptureHarnessState,
    pair: &CryptoPreResolutionPair,
) -> &'a calyx_poly::crypto_capture_harness::CryptoCapturedSnapshotRef {
    state
        .captures
        .iter()
        .flat_map(|capture| capture.snapshots.iter())
        .find(|snapshot| snapshot.forecast_id == pair.forecast_id)
        .expect("snapshot for pair")
}

fn assert_single_calyx_native_capture(state: &CryptoCaptureHarnessState) {
    assert_eq!(state.captures.len(), 1);
    assert_eq!(state.captures[0].market_id, MARKET_ID);
    assert_eq!(state.captures[0].condition_id, CONDITION_ID);
    assert_eq!(state.captures[0].snapshots.len(), 2);
    assert!(
        state.captures[0]
            .snapshots
            .iter()
            .all(|snapshot| snapshot.pending_entry.source == ForecastSource::CalyxNative)
    );
}

fn assert_forecast_artifact_hashes(state: &CryptoCaptureHarnessState) -> Vec<Value> {
    state.captures[0]
        .snapshots
        .iter()
        .map(|snapshot| {
            let path = snapshot
                .forecast_artifact_path
                .as_ref()
                .expect("artifact path");
            let expected = snapshot
                .forecast_artifact_blake3
                .as_ref()
                .expect("artifact blake3");
            let actual = blake3_hex(Path::new(path));
            assert_eq!(&actual, expected);
            json!({
                "forecast_id": snapshot.forecast_id,
                "token_id": snapshot.token_id,
                "artifact_path": path,
                "artifact_blake3": actual
            })
        })
        .collect()
}
