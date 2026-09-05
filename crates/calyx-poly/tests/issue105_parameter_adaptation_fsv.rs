//! Issue #105 - scheduled parameter auto-adaptation.
//!
//! Source of truth: persisted observation corpus, rollback artifact, parameter-adaptation report,
//! and append-only parameter-adaptation ledger JSONL, all read back from disk.

use std::path::Path;

use calyx_poly::{
    ERR_PARAMETER_ADAPTATION_INSUFFICIENT_DATA, ERR_PARAMETER_ADAPTATION_LOOKAHEAD,
    ERR_PARAMETER_ADAPTATION_MALFORMED_ROW, ERR_PARAMETER_ADAPTATION_MISSING_ARTIFACT,
    PARAMETER_ADAPTATION_MIN_ROWS, ParameterAdaptationArtifactRef, ParameterAdaptationReport,
    ParameterAdaptationRequest, ParameterAdaptationSchedule, ParameterAdaptationStatus,
    ParameterObservation, ParameterSetSnapshot, read_parameter_adaptation_ledger_entries,
    read_parameter_adaptation_report, run_parameter_adaptation_report,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const ROWS: usize = PARAMETER_ADAPTATION_MIN_ROWS;
const PREVIOUS_RUN_TS: u64 = 1_000;
const SCHEDULED_AT_TS: u64 = 1_020;

#[test]
fn issue105_parameter_adaptation_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE105_FSV_ROOT",
        "poly-issue105-parameter-adaptation",
    );
    reset_dir(&root);

    let happy = happy_promotes_versioned_parameter_set(&root);
    let insufficient = edge_insufficient_sample_fails_loud(&root);
    let malformed = edge_malformed_row_fails_loud(&root);
    let lookahead = edge_lookahead_fails_loud(&root);
    let missing_artifact = edge_missing_artifact_fails_loud(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 105,
        "proof_claim": "Poly refits encoder sigma, quantile edges, TE lag, and kNN k on a schedule from accumulated local observations, versions the promoted parameter set, appends a ledger row for the change, and fails closed on insufficient data, malformed rows, look-ahead observations, and missing artifacts.",
        "minimum_sufficient_corpus": {
            "observations": ROWS,
            "max_te_lag": 3,
            "candidate_knn_k": [1, 3, 5],
            "why_this_is_sufficient": "Eight observations are the smallest corpus here that supports TE lag candidates through 3, leave-one-out kNN candidates through k=5, both outcome classes, and the scheduled new-row floor while exercising all four parameter families.",
            "why_smaller_is_insufficient": "Seven rows cannot satisfy the issue floor and weakens the k=5 leave-one-out proof; fewer rows also reduce lag and outcome-class coverage.",
            "why_larger_is_wasteful": "More rows would repeat the same sigma, quantile, lag scoring, kNN selection, report persistence, ledger append, and readback paths without proving a new #105 behavior."
        },
        "happy_path": happy,
        "edge_cases": {
            "insufficient_sample": insufficient,
            "malformed_row": malformed,
            "lookahead": lookahead,
            "missing_artifact": missing_artifact
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE105_PARAMETER_ADAPTATION_READBACK={}",
        readback_path.display()
    );
}

fn happy_promotes_versioned_parameter_set(root: &Path) -> Value {
    let request = request_with_sources(root, "happy", observations());
    let report = run_and_read(root, "happy", &request);
    assert_eq!(report.status, ParameterAdaptationStatus::Promoted);
    assert_eq!(report.observation_count, ROWS);
    assert_eq!(report.new_observation_count, 4);
    assert_eq!(report.proposed.te_lag, 2);
    assert_eq!(report.proposed.knn_k, 1);
    assert!(report.proposed.encoder_sigma > 0.0);
    assert_eq!(report.proposed.quantile_edges.len(), 5);
    assert!(report.metrics.brier_improvement > 0.10);
    for name in ["encoder_sigma", "quantile_edges", "te_lag", "knn_k"] {
        assert!(report.changed_parameters.contains(&name.to_string()));
    }
    let ledger_path = Path::new(&request.ledger_dir).join("parameter_adaptation_ledger.jsonl");
    let ledger = read_parameter_adaptation_ledger_entries(&ledger_path).expect("read ledger");
    assert_eq!(ledger.len(), 1);
    let entry = ledger.first().expect("ledger entry");
    assert_eq!(entry.new_version, report.proposed.version);
    assert_eq!(entry.report_hash, report.report_hash);
    json!({
        "status": report.status,
        "previous": report.previous,
        "proposed": report.proposed,
        "metrics": report.metrics,
        "changed_parameters": report.changed_parameters,
        "ledger_entry": entry,
        "report_hash": report.report_hash
    })
}

fn edge_insufficient_sample_fails_loud(root: &Path) -> Value {
    let request = request_with_sources(root, "edge-insufficient", observations()[..7].to_vec());
    let err = run_parameter_adaptation_report(&request, &root.join("edge-insufficient"))
        .expect_err("insufficient sample refused");
    assert_eq!(err.code(), ERR_PARAMETER_ADAPTATION_INSUFFICIENT_DATA);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_malformed_row_fails_loud(root: &Path) -> Value {
    let mut request = request_with_sources(root, "edge-malformed", observations());
    request.observations[2].scalar_value = f64::NAN;
    let err = run_parameter_adaptation_report(&request, &root.join("edge-malformed"))
        .expect_err("malformed row refused");
    assert_eq!(err.code(), ERR_PARAMETER_ADAPTATION_MALFORMED_ROW);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_lookahead_fails_loud(root: &Path) -> Value {
    let mut request = request_with_sources(root, "edge-lookahead", observations());
    request.observations[7].ts = SCHEDULED_AT_TS + 1;
    let err = run_parameter_adaptation_report(&request, &root.join("edge-lookahead"))
        .expect_err("look-ahead row refused");
    assert_eq!(err.code(), ERR_PARAMETER_ADAPTATION_LOOKAHEAD);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_missing_artifact_fails_loud(root: &Path) -> Value {
    let mut request = request_with_sources(root, "edge-missing-artifact", observations());
    request.rollback_artifact.path = root
        .join("edge-missing-artifact")
        .join("missing-rollback.json")
        .display()
        .to_string();
    let err = run_parameter_adaptation_report(&request, &root.join("edge-missing-artifact"))
        .expect_err("missing rollback refused");
    assert_eq!(err.code(), ERR_PARAMETER_ADAPTATION_MISSING_ARTIFACT);
    json!({"code": err.code(), "message": err.message()})
}

fn run_and_read(
    root: &Path,
    dir: &str,
    request: &ParameterAdaptationRequest,
) -> ParameterAdaptationReport {
    let run =
        run_parameter_adaptation_report(request, &root.join(dir)).expect("parameter adaptation");
    let report = read_parameter_adaptation_report(&run.report_path).expect("read report");
    assert_eq!(report.report_hash, run.report.report_hash);
    assert_eq!(report.proposed.version, run.report.proposed.version);
    assert_eq!(report.ledger_entry, run.report.ledger_entry);
    report
}

fn request_with_sources(
    root: &Path,
    dir: &str,
    observations: Vec<ParameterObservation>,
) -> ParameterAdaptationRequest {
    let case_dir = root.join(dir);
    let observations_path = case_dir.join("observations.json");
    write_json(
        &observations_path,
        &serde_json::to_value(&observations).expect("observations json"),
    );
    let observations_artifact = artifact_ref(&observations_path);
    let rollback_path = case_dir.join("rollback_parameters.json");
    write_json(
        &rollback_path,
        &json!({"restore_version": "crypto:1h_24h:previous"}),
    );
    let rollback_artifact = artifact_ref(&rollback_path);
    ParameterAdaptationRequest {
        domain: "crypto".to_string(),
        horizon_bucket: "1h_24h".to_string(),
        observations_artifact,
        rollback_artifact,
        ledger_dir: case_dir.join("ledger").display().to_string(),
        current: ParameterSetSnapshot {
            version: "crypto:1h_24h:previous".to_string(),
            encoder_sigma: 0.01,
            quantile_edges: vec![0.0, 1.0, 2.0],
            te_lag: 1,
            knn_k: 5,
        },
        schedule: ParameterAdaptationSchedule {
            previous_run_ts: PREVIOUS_RUN_TS + 3,
            scheduled_at_ts: SCHEDULED_AT_TS,
            min_rows: PARAMETER_ADAPTATION_MIN_ROWS,
            min_new_rows: 4,
            max_te_lag: 3,
            candidate_knn_k: vec![1, 3, 5],
            min_brier_improvement: 0.05,
        },
        observations,
    }
}

fn artifact_ref(path: &Path) -> ParameterAdaptationArtifactRef {
    ParameterAdaptationArtifactRef {
        path: path.display().to_string(),
        blake3: hex(blake3::hash(&std::fs::read(path).expect("hash artifact")).as_bytes()),
    }
}

fn observations() -> Vec<ParameterObservation> {
    let lag_signal = [0.05, 0.10, 0.95, 0.90, 0.15, 0.20, 0.85, 0.80];
    let outcomes = [false, false, false, false, true, true, false, false];
    (0..ROWS)
        .map(|idx| ParameterObservation {
            ts: PREVIOUS_RUN_TS + idx as u64,
            scalar_value: 0.20 + idx as f64 * 0.05,
            heavy_tail_value: [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0][idx],
            lag_signal: lag_signal[idx],
            outcome_yes: outcomes[idx],
            knn_vector: if outcomes[idx] {
                vec![1.0, 0.05 * idx as f32]
            } else {
                vec![-1.0, 0.05 * idx as f32]
            },
        })
        .collect()
}
