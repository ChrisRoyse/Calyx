//! Issue #237 - settlement verifier for the minimum CalyxNative retune corpus.
//!
//! The default corpus is the exact 30-observation floor already captured for #237: 2 resolved
//! observations from the first-light market plus 14 pending binary CalyxNative capture roots. This
//! test fetches the exact Gamma markets, joins only clean closed winners on copied roots, and runs
//! production retune/audit only when enough real observations exist.

#[path = "support/issue237_live_first_light_support.rs"]
mod live_support;
// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use std::collections::BTreeSet;
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
use calyx_poly::forecast_calibration::MIN_CALIBRATION_SAMPLES;
use calyx_poly::{
    ForecastScoreRequest, ForecastSource, PendingForecastRegister, ResolvedOutcome,
    run_blend_relearning, run_calibration_refit, write_forecast_score_artifacts,
};
use live_support::{
    assert_c_drive, blake3_hex, copy_tree, gamma_resolution_readback, get_market_body, sha256_hex,
};
use serde::Serialize;
use serde_json::{Value, json};
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const RESOLVED_ROOT: &str = "issue237_live_first_light_postsettlement_rounded_20260707";
const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue237-calyx-native-accumulation";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BatchDecision {
    WaitingForSettlement,
    WaitingForMoreRealOutcomes,
    RetuneBlocked,
    AuditPassed,
}

#[derive(Clone)]
struct ScoreJob {
    state_path: PathBuf,
    state: CryptoCaptureHarnessState,
    pair: CryptoPreResolutionPair,
}

struct ScoreLink {
    state_path: PathBuf,
    forecast_path: PathBuf,
    manifest_path: PathBuf,
    actual_win: bool,
}

#[test]
#[ignore = "requires live Gamma reads and prior C: #237 capture evidence roots"]
fn issue237_calyx_native_settlement_batch_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE237_SETTLEMENT_BATCH_ROOT",
        "issue237-calyx-native-settlement-batch",
    );
    assert_c_drive(&root);
    reset_dir(&root);

    let resolved_root = resolved_root();
    assert_c_drive(&resolved_root);
    let resolved_state_path = resolved_root
        .join("harness-copy")
        .join(CRYPTO_CAPTURE_STATE_FILE);
    let resolved_state =
        read_crypto_capture_state(&resolved_state_path).expect("read settled #237 state");
    assert_calyx_native_state(&resolved_state);
    let prior_pairs = read_pairs(
        &resolved_root
            .join("harness-copy")
            .join(CRYPTO_PRE_RESOLUTION_CORPUS_FILE),
    );
    assert_eq!(prior_pairs.len(), 2);

    let pending_roots = pending_roots();
    assert_eq!(pending_roots.len(), 14);
    let mut conditions = prior_pairs
        .iter()
        .map(|pair| pair.condition_id.clone())
        .collect::<BTreeSet<_>>();
    let mut market_results = Vec::new();
    let mut raw_files = Vec::new();
    let mut score_jobs = prior_pairs
        .iter()
        .cloned()
        .map(|pair| ScoreJob {
            state_path: resolved_state_path.clone(),
            state: resolved_state.clone(),
            pair,
        })
        .collect::<Vec<_>>();

    for source_root in &pending_roots {
        assert_c_drive(source_root);
        let state_path = source_root.join(CRYPTO_CAPTURE_STATE_FILE);
        let before = read_crypto_capture_state(&state_path).expect("read pending state");
        assert_calyx_native_state(&before);
        let capture = &before.captures[0];
        assert!(conditions.insert(capture.condition_id.clone()));
        let artifact_checks = artifact_checks(&before);
        let raw_body = get_market_body(&capture.market_id);
        let raw_path = root.join(format!("gamma-market-{}-body.json", capture.market_id));
        fs::write(&raw_path, &raw_body).expect("write exact Gamma body");
        let raw_sha256 = sha256_hex(&raw_body);
        raw_files.push(json!({
            "market_id": capture.market_id,
            "path": raw_path.display().to_string(),
            "sha256": raw_sha256
        }));
        let gamma = gamma_resolution_readback(
            &serde_json::from_slice::<Value>(&raw_body).expect("decode Gamma body"),
            &capture.market_id,
            &capture.condition_id,
        )
        .expect("parse Gamma resolution");

        let mut joined_pair_count = 0usize;
        let mut copied_root = Value::Null;
        let decision = if gamma.closed != Some(true) {
            let after = read_crypto_capture_state(&state_path).expect("read unchanged source");
            assert_eq!(after, before);
            "source_still_open"
        } else if gamma.resolution.is_none() {
            let after = read_crypto_capture_state(&state_path).expect("read unchanged source");
            assert_eq!(after, before);
            "closed_without_clean_winner"
        } else {
            let dst = root.join("joined-captures").join(&capture.market_id);
            copy_tree(source_root, &dst);
            copied_root = json!(dst.display().to_string());
            let resolution = gamma.resolution.clone().expect("resolution");
            assert!(resolution.resolved_ts > capture.captured_ts);
            let vault = AsterVault::open(
                dst.join("live-capture-vault"),
                VAULT_ID.parse().unwrap(),
                VAULT_SALT.to_vec(),
                VaultOptions::default(),
            )
            .expect("open copied accumulation vault");
            let mut register = PendingForecastRegister::default();
            let joined =
                join_crypto_capture_resolution(&vault, &mut register, &dst, &resolution, false)
                    .expect("join copied pending capture");
            let corpus = read_pairs(&dst.join(CRYPTO_PRE_RESOLUTION_CORPUS_FILE));
            assert_eq!(corpus, joined.record.pairs);
            assert_eq!(joined.record.pairs.len(), 2);
            joined_pair_count = joined.record.pairs.len();
            for pair in joined.record.pairs {
                score_jobs.push(ScoreJob {
                    state_path: joined.state_path.clone(),
                    state: joined.state.clone(),
                    pair,
                });
            }
            "joined_clean_winner"
        };
        market_results.push(json!({
            "source_root": source_root.display().to_string(),
            "copied_root": copied_root,
            "market_id": capture.market_id,
            "condition_id": capture.condition_id,
            "capture_ts": capture.captured_ts,
            "snapshot_count": capture.snapshots.len(),
            "artifact_checks": artifact_checks,
            "gamma": gamma,
            "raw_sha256": raw_sha256,
            "decision": decision,
            "joined_pair_count": joined_pair_count
        }));
    }

    let resolved_observation_count = score_jobs.len();
    let unresolved_market_count = market_results
        .iter()
        .filter(|row| row.get("decision").and_then(Value::as_str) == Some("source_still_open"))
        .count();
    let no_clean_winner_count = market_results
        .iter()
        .filter(|row| {
            row.get("decision").and_then(Value::as_str) == Some("closed_without_clean_winner")
        })
        .count();
    let (decision, retune, score_count) =
        if unresolved_market_count > 0 || no_clean_winner_count > 0 {
            (BatchDecision::WaitingForSettlement, Value::Null, 0usize)
        } else {
            run_batch_retune_and_audit(&root, &score_jobs)
        };

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 237,
        "proof_claim": "Recheck the exact minimum real CalyxNative corpus captured for #237 and run production retune/audit only after the public sources produce clean resolved outcomes.",
        "minimum_sufficient_proof_corpus": {
            "already_resolved_observations": prior_pairs.len(),
            "pending_capture_roots": pending_roots.len(),
            "pending_observation_capacity": pending_roots.len() * 2,
            "total_observation_capacity": prior_pairs.len() + pending_roots.len() * 2,
            "required_minimum": MIN_CALIBRATION_SAMPLES,
            "why_this_is_sufficient": "These are exactly the captured #237 roots needed to reach the 30-real-observation production calibration floor once all pending markets settle cleanly.",
            "why_smaller_is_insufficient": "Any smaller set has fewer than 30 eventual real observations and cannot prove production calibration refit acceptance.",
            "why_larger_is_wasteful": "Additional markets would exceed the current #237 acceptance floor and add settlement/API/disk work without increasing proof for this boundary."
        },
        "source_of_truth": {
            "resolved_root": resolved_root.display().to_string(),
            "resolved_state_path": resolved_state_path.display().to_string(),
            "pending_capture_roots": pending_roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "raw_gamma_files": raw_files
        },
        "counts": {
            "unique_condition_count": conditions.len(),
            "resolved_observation_count": resolved_observation_count,
            "unresolved_market_count": unresolved_market_count,
            "no_clean_winner_count": no_clean_winner_count,
            "score_artifact_count": score_count
        },
        "decision": decision,
        "retune": retune,
        "markets": market_results,
        "physical_files": files
    });
    let report_path = root.join("issue237_calyx_native_settlement_batch_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(readback, report);
    write_blake3sums(&root);
    println!(
        "ISSUE237_CALYX_NATIVE_SETTLEMENT_BATCH_READBACK={}",
        report_path.display()
    );
}

fn run_batch_retune_and_audit(root: &Path, jobs: &[ScoreJob]) -> (BatchDecision, Value, usize) {
    let max_resolved_ts = jobs
        .iter()
        .map(|job| job.pair.resolved_ts)
        .max()
        .unwrap_or(0);
    let score_links = write_scores(root, jobs, max_resolved_ts + 20);
    let blend = run_blend_relearning(&BlendRelearningRequest {
        out_dir: &root.join("blend"),
        domain: "crypto",
        horizon_bucket: "pre_resolution",
        as_of_millis: (max_resolved_ts + 1) * 1000,
        min_samples_per_component: 1,
        observations: blend_observations(jobs),
    })
    .expect("batch blend relearning");
    let calibration = run_calibration_refit(&CalibrationRefitRequest {
        out_dir: &root.join("calibration-real-only"),
        domain: "crypto",
        horizon_bucket: "pre_resolution",
        previous_version: None,
        as_of_millis: (max_resolved_ts + 1) * 1000,
        observations: jobs
            .iter()
            .map(|job| CalibrationRefitObservation {
                p_raw: job.pair.p_model,
                outcome_yes: job.pair.actual_win,
                resolved_at_millis: job.pair.resolved_ts * 1000,
            })
            .collect(),
    });
    let Ok(calibration) = calibration else {
        let err = calibration.unwrap_err();
        let decision = if jobs.len() < MIN_CALIBRATION_SAMPLES {
            BatchDecision::WaitingForMoreRealOutcomes
        } else {
            BatchDecision::RetuneBlocked
        };
        return (
            decision,
            json!({
                "blend": blend.report,
                "calibration_error": err.code(),
                "real_observation_count": jobs.len(),
                "required_minimum": MIN_CALIBRATION_SAMPLES
            }),
            score_links.len(),
        );
    };
    let link = score_links
        .iter()
        .find(|link| link.actual_win)
        .unwrap_or_else(|| score_links.first().expect("score link"));
    let audit = run_first_light_audit(&FirstLightAuditRequest {
        report_dir: &root.join("audit"),
        capture_state_path: &link.state_path,
        calyx_native_forecast_path: &link.forecast_path,
        score_manifest_path: &link.manifest_path,
        blend_relearning_report_path: &blend.report_path,
        calibration_refit_report_path: &calibration.report_path,
    })
    .expect("batch first-light audit");
    (
        BatchDecision::AuditPassed,
        json!({
            "blend": blend.report,
            "calibration": calibration.report,
            "audit": audit.report
        }),
        score_links.len(),
    )
}

fn write_scores(root: &Path, jobs: &[ScoreJob], scored_ts: u64) -> Vec<ScoreLink> {
    let mut ledger = LedgerAppender::open(
        DirectoryLedgerStore::open(root.join("score-ledger")).expect("score ledger"),
        FixedClock::new(scored_ts),
    )
    .expect("open score ledger");
    jobs.iter()
        .enumerate()
        .map(|(idx, job)| {
            let snapshot = snapshot_for_pair(&job.state, &job.pair);
            let forecast_path = PathBuf::from(snapshot.forecast_artifact_path.as_ref().unwrap());
            let forecast = read_calyx_native_forecast(&forecast_path).expect("read forecast");
            let score_id = format!("score237b{idx:02}o{}", job.pair.outcome_index);
            write_forecast_score_artifacts(
                &root.join("scores"),
                &mut ledger,
                &ForecastScoreRequest {
                    score_id: score_id.clone(),
                    forecast_id: job.pair.forecast_id.clone(),
                    forecast_version: 1,
                    current_forecast_version: 1,
                    market_id: job.pair.condition_id.clone(),
                    outcome_id: job.pair.token_id.clone(),
                    source: ForecastSource::CalyxNative,
                    provider: None,
                    probability: forecast.p_model,
                    confidence: forecast.confidence,
                    forecast_ts: job.pair.forecast_ts,
                    scored_ts,
                    horizon_secs: job.pair.resolved_ts.saturating_sub(job.pair.forecast_ts),
                    sufficiency_state: if forecast.admissible {
                        "sufficient"
                    } else {
                        "refused"
                    }
                    .to_string(),
                    previous_probability: Some(0.5),
                    forecast_artifact_hash: blake3_hex(&forecast_path),
                    outcome: ResolvedOutcome {
                        outcome_id: job.pair.token_id.clone(),
                        resolved: true,
                        actual_win: job.pair.actual_win,
                        resolved_ts: job.pair.resolved_ts,
                        source: "gamma-closed-derived".to_string(),
                        version: 1,
                    },
                    calibration_bin_count: 10,
                },
            )
            .expect("write batch score");
            ScoreLink {
                state_path: job.state_path.clone(),
                forecast_path,
                manifest_path: root.join("scores").join(score_id).join("manifest.json"),
                actual_win: job.pair.actual_win,
            }
        })
        .collect()
}

fn blend_observations(jobs: &[ScoreJob]) -> Vec<BlendWeightObservation> {
    let mut observations = Vec::new();
    for job in jobs {
        let snapshot = snapshot_for_pair(&job.state, &job.pair);
        let forecast = read_calyx_native_forecast(&PathBuf::from(
            snapshot.forecast_artifact_path.as_ref().unwrap(),
        ))
        .expect("read forecast for blend");
        for component in forecast.components {
            observations.push(BlendWeightObservation {
                component: component.kind,
                p_yes: component.p,
                outcome_yes: job.pair.actual_win,
                observed_at_millis: job.pair.resolved_ts * 1000,
            });
        }
    }
    observations
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

fn assert_calyx_native_state(state: &CryptoCaptureHarnessState) {
    assert_eq!(state.captures.len(), 1);
    assert_eq!(state.captures[0].snapshots.len(), 2);
    assert!(state.captures[0].snapshots.iter().all(|snapshot| {
        snapshot.pending_entry.source == ForecastSource::CalyxNative
            && snapshot.forecast_artifact_path.is_some()
            && snapshot.forecast_artifact_blake3.is_some()
    }));
}

fn artifact_checks(state: &CryptoCaptureHarnessState) -> Vec<Value> {
    state.captures[0]
        .snapshots
        .iter()
        .map(|snapshot| {
            let path = snapshot.forecast_artifact_path.as_ref().unwrap();
            let actual = blake3_hex(Path::new(path));
            assert_eq!(Some(&actual), snapshot.forecast_artifact_blake3.as_ref());
            json!({
                "forecast_id": snapshot.forecast_id,
                "token_id": snapshot.token_id,
                "artifact_path": path,
                "artifact_blake3": actual
            })
        })
        .collect()
}

fn read_pairs(path: &Path) -> Vec<CryptoPreResolutionPair> {
    serde_json::from_slice(&fs::read(path).expect("read pre-resolution corpus"))
        .expect("parse pre-resolution corpus")
}

fn resolved_root() -> PathBuf {
    std::env::var_os("POLY_ISSUE237_RESOLVED_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target").join("fsv").join(RESOLVED_ROOT))
}

fn pending_roots() -> Vec<PathBuf> {
    if let Some(raw) = std::env::var_os("POLY_ISSUE237_PENDING_CAPTURE_ROOTS") {
        return raw
            .to_string_lossy()
            .split(';')
            .filter(|part| !part.trim().is_empty())
            .map(PathBuf::from)
            .collect();
    }
    default_pending_root_names()
        .into_iter()
        .map(|name| repo_root().join("target").join("fsv").join(name))
        .collect()
}

fn default_pending_root_names() -> Vec<&'static str> {
    vec![
        "issue237_calyx_native_accumulation_capture_20260707_1640",
        "issue237_calyx_native_accumulation_distinct_20260707",
        "issue237_calyx_native_accumulation_distinct2_20260707",
        "issue237_calyx_native_accumulation_distinct3_20260707",
        "issue237_calyx_native_accumulation_distinct4_20260707",
        "issue237_calyx_native_accumulation_distinct5_20260707",
        "issue237_calyx_native_accumulation_distinct6_20260707",
        "issue237_calyx_native_accumulation_distinct7_20260707",
        "issue237_calyx_native_accumulation_distinct8_20260707",
        "issue237_calyx_native_accumulation_distinct9_20260707",
        "issue237_calyx_native_accumulation_distinct10_20260707",
        "issue237_calyx_native_accumulation_distinct11_20260707",
        "issue237_calyx_native_accumulation_distinct12_20260707",
        "issue237_calyx_native_accumulation_distinct13_20260707",
    ]
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}
