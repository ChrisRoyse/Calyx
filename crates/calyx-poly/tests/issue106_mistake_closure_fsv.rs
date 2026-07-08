//! Issue #106 - online mistake-closure heads for resolved forecast errors.
//!
//! Source of truth: local forecast, source snapshot, outcome anchor, score, prompt, rollback, and
//! mistake-closure report JSON files, all physically read back before assertions are recorded.

use std::path::Path;

use calyx_poly::{
    ERR_MISTAKE_CLOSURE_INSUFFICIENT_SAMPLE, ERR_MISTAKE_CLOSURE_LOOKAHEAD,
    ERR_MISTAKE_CLOSURE_MISSING_OUTCOME, MISTAKE_CLOSURE_MIN_ROWS, MistakeClosureArtifactRef,
    MistakeClosureHeadKind, MistakeClosureReport, MistakeClosureRequest, MistakeClosureScoreRow,
    MistakeClosureStatus, MistakeClosureThresholds, read_mistake_closure_report,
    require_mistake_closure_proposed, run_mistake_closure_report,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const GENERATED_AT: u64 = 1_785_700_000;
const ROWS: usize = MISTAKE_CLOSURE_MIN_ROWS;
const HEAD_KINDS: usize = 4;

#[test]
fn issue106_mistake_closure_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE106_FSV_ROOT", "poly-issue106-mistake");
    reset_dir(&root);

    let happy = happy_error_creates_corrective_heads(&root);
    let missing = edge_missing_outcome_fails_loud(&root);
    let insufficient = edge_insufficient_sample_fails_loud(&root);
    let lookahead = edge_lookahead_fails_loud(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 106,
        "proof_claim": "Poly learns from local scored forecast errors only after outcomes resolve, emits versioned mistake-closure proposals for lens, association, prompt, and admission improvements with rollback evidence, and refuses missing outcomes, insufficient samples, and look-ahead leakage.",
        "minimum_sufficient_corpus": {
            "scored_forecast_rows": ROWS,
            "corrective_head_kinds": HEAD_KINDS,
            "why_this_is_sufficient": "Four scored rows are the smallest balanced two-YES/two-NO corpus that proves probability-error measurement while carrying one evidence instance for each required corrective head kind.",
            "why_smaller_is_insufficient": "Fewer rows would violate the module's sample floor or omit one outcome class or one required head kind.",
            "why_larger_is_wasteful": "More rows would repeat the same artifact hashing, causal timing, Brier/effect computation, proposal construction, rollback, and readback paths without proving a new #106 behavior."
        },
        "happy_path": happy,
        "edge_cases": {
            "missing_outcome": missing,
            "insufficient_sample": insufficient,
            "lookahead_leakage": lookahead
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE106_MISTAKE_CLOSURE_READBACK={}",
        readback_path.display()
    );
}

fn happy_error_creates_corrective_heads(root: &Path) -> Value {
    let request = request_with_sources(root, "happy", rows("happy"), thresholds());
    let report = run_and_read(root, "happy", &request);
    require_mistake_closure_proposed(&report).expect("mistake closure proposed");
    assert_eq!(report.status, MistakeClosureStatus::Proposed);
    assert_eq!(report.scored_count, ROWS);
    assert_eq!(report.mistake_count, ROWS);
    assert_eq!(report.proposal_count, HEAD_KINDS);
    assert!(report.aggregate_effect.brier_improvement > 0.30);
    assert!(report.aggregate_effect.calibration_abs_error_improvement > 0.30);
    assert!(report.aggregate_effect.sufficiency_bits_improvement > 0.20);
    assert!(report.aggregate_effect.association_recall_improvement > 0.10);
    assert_eq!(report.report_hash.len(), 64);
    let kinds = report
        .proposals
        .iter()
        .map(|proposal| proposal.kind)
        .collect::<Vec<_>>();
    assert!(kinds.contains(&MistakeClosureHeadKind::Lens));
    assert!(kinds.contains(&MistakeClosureHeadKind::Association));
    assert!(kinds.contains(&MistakeClosureHeadKind::Prompt));
    assert!(kinds.contains(&MistakeClosureHeadKind::Admission));
    json!({
        "status": report.status,
        "artifact_version": report.artifact_version,
        "aggregate_effect": report.aggregate_effect,
        "proposal_count": report.proposal_count,
        "proposals": report.proposals,
        "rollback_artifact": report.rollback_artifact,
        "report_hash": report.report_hash
    })
}

fn edge_missing_outcome_fails_loud(root: &Path) -> Value {
    let mut rows = rows("edge-missing");
    rows[0].actual_win = None;
    rows[0].resolved_ts = None;
    rows[0].outcome_anchor = None;
    let request = request_with_sources(root, "edge-missing", rows, thresholds());
    let err = run_mistake_closure_report(&request, &root.join("edge-missing"))
        .expect_err("missing outcome refused");
    assert_eq!(err.code(), ERR_MISTAKE_CLOSURE_MISSING_OUTCOME);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_insufficient_sample_fails_loud(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "edge-insufficient",
        rows("edge-insufficient")[..3].to_vec(),
        thresholds(),
    );
    let err = run_mistake_closure_report(&request, &root.join("edge-insufficient"))
        .expect_err("insufficient sample refused");
    assert_eq!(err.code(), ERR_MISTAKE_CLOSURE_INSUFFICIENT_SAMPLE);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_lookahead_fails_loud(root: &Path) -> Value {
    let mut rows = rows("edge-lookahead");
    rows[0].source_snapshot_ts = rows[0].forecast_ts + 1;
    let request = request_with_sources(root, "edge-lookahead", rows, thresholds());
    let err = run_mistake_closure_report(&request, &root.join("edge-lookahead"))
        .expect_err("look-ahead refused");
    assert_eq!(err.code(), ERR_MISTAKE_CLOSURE_LOOKAHEAD);
    json!({"code": err.code(), "message": err.message()})
}

fn run_and_read(root: &Path, dir: &str, request: &MistakeClosureRequest) -> MistakeClosureReport {
    let run = run_mistake_closure_report(request, &root.join(dir)).expect("mistake closure run");
    let readback = read_mistake_closure_report(&run.report_path).expect("read report");
    assert_eq!(
        serde_json::to_value(&readback).ok(),
        serde_json::to_value(&run.report).ok()
    );
    readback
}

fn request_with_sources(
    root: &Path,
    dir: &str,
    mut rows: Vec<MistakeClosureScoreRow>,
    thresholds: MistakeClosureThresholds,
) -> MistakeClosureRequest {
    let case_dir = root.join(dir);
    let rollback_artifact = artifact(
        &case_dir.join("rollback.json"),
        json!({"restore": "previous"}),
    );
    for row in &mut rows {
        attach_artifacts(&case_dir, row);
    }
    let scored_history_artifact = case_dir.join("scored_history.json");
    let source_snapshot_artifact = case_dir.join("source_snapshots.json");
    let outcome_anchor_artifact = case_dir.join("outcome_anchors.json");
    write_json(
        &scored_history_artifact,
        &serde_json::to_value(&rows).expect("rows json"),
    );
    write_json(
        &source_snapshot_artifact,
        &json!({"source_snapshot_hashes": rows.iter().map(|row| row.source_snapshot.blake3.clone()).collect::<Vec<_>>()}),
    );
    write_json(
        &outcome_anchor_artifact,
        &json!({"outcome_anchor_hashes": rows.iter().filter_map(|row| row.outcome_anchor.as_ref().map(|artifact| artifact.blake3.clone())).collect::<Vec<_>>()}),
    );
    let readback_rows: Vec<MistakeClosureScoreRow> =
        serde_json::from_slice(&std::fs::read(&scored_history_artifact).expect("read history"))
            .expect("decode history");
    assert_eq!(readback_rows, rows);
    MistakeClosureRequest {
        domain: "crypto".to_string(),
        horizon_bucket: "1h_24h".to_string(),
        scored_history_artifact: scored_history_artifact.display().to_string(),
        source_snapshot_artifact: source_snapshot_artifact.display().to_string(),
        outcome_anchor_artifact: outcome_anchor_artifact.display().to_string(),
        generated_at: GENERATED_AT,
        rollback_artifact,
        thresholds,
        rows,
    }
}

fn attach_artifacts(case_dir: &Path, row: &mut MistakeClosureScoreRow) {
    let row_dir = case_dir.join(&row.forecast_id);
    row.forecast_artifact = artifact(
        &row_dir.join("forecast.json"),
        json!({"forecast_id": row.forecast_id, "probability": row.probability}),
    );
    row.source_snapshot = artifact(
        &row_dir.join("source_snapshot.json"),
        json!({"forecast_id": row.forecast_id, "source_snapshot_ts": row.source_snapshot_ts}),
    );
    row.score_artifact = artifact(
        &row_dir.join("score.json"),
        json!({"forecast_id": row.forecast_id, "brier": row.probability}),
    );
    row.prompt_artifact = artifact(
        &row_dir.join("prompt.json"),
        json!({"forecast_id": row.forecast_id, "pattern_count": row.prompt_pattern_count}),
    );
    if row.actual_win.is_some() {
        row.outcome_anchor = Some(artifact(
            &row_dir.join("outcome_anchor.json"),
            json!({"forecast_id": row.forecast_id, "actual_win": row.actual_win}),
        ));
    }
}

fn artifact(path: &Path, value: Value) -> MistakeClosureArtifactRef {
    write_json(path, &value);
    MistakeClosureArtifactRef {
        path: path.display().to_string(),
        blake3: hex(blake3::hash(&std::fs::read(path).expect("hash artifact")).as_bytes()),
    }
}

fn rows(prefix: &str) -> Vec<MistakeClosureScoreRow> {
    vec![
        row(ScoreRowInput {
            prefix,
            suffix: "a",
            forecast_ts: 1_000,
            actual_win: true,
            probability: 0.10,
            closure_probability: 0.70,
            missing_evidence_count: 1,
            weak_association_count: 1,
            prompt_pattern_count: 1,
        }),
        row(ScoreRowInput {
            prefix,
            suffix: "b",
            forecast_ts: 1_100,
            actual_win: false,
            probability: 0.90,
            closure_probability: 0.30,
            missing_evidence_count: 1,
            weak_association_count: 1,
            prompt_pattern_count: 1,
        }),
        row(ScoreRowInput {
            prefix,
            suffix: "c",
            forecast_ts: 1_200,
            actual_win: true,
            probability: 0.20,
            closure_probability: 0.75,
            missing_evidence_count: 1,
            weak_association_count: 1,
            prompt_pattern_count: 1,
        }),
        row(ScoreRowInput {
            prefix,
            suffix: "d",
            forecast_ts: 1_300,
            actual_win: false,
            probability: 0.80,
            closure_probability: 0.25,
            missing_evidence_count: 1,
            weak_association_count: 1,
            prompt_pattern_count: 1,
        }),
    ]
}

struct ScoreRowInput<'a> {
    prefix: &'a str,
    suffix: &'a str,
    forecast_ts: u64,
    actual_win: bool,
    probability: f64,
    closure_probability: f64,
    missing_evidence_count: usize,
    weak_association_count: usize,
    prompt_pattern_count: usize,
}

fn row(input: ScoreRowInput<'_>) -> MistakeClosureScoreRow {
    MistakeClosureScoreRow {
        forecast_id: format!("{}-{}", input.prefix, input.suffix),
        forecast_ts: input.forecast_ts,
        resolved_ts: Some(input.forecast_ts + 100),
        scored_ts: input.forecast_ts + 200,
        source_snapshot_ts: input.forecast_ts - 10,
        actual_win: Some(input.actual_win),
        probability: input.probability,
        closure_probability: input.closure_probability,
        sufficiency_bits: 0.40,
        closure_sufficiency_bits: 0.70,
        association_recall_ratio: 0.70,
        closure_association_recall_ratio: 0.90,
        calibration_abs_error: (input.probability - if input.actual_win { 1.0 } else { 0.0 }).abs(),
        closure_calibration_abs_error: (input.closure_probability
            - if input.actual_win { 1.0 } else { 0.0 })
        .abs(),
        missing_evidence_count: input.missing_evidence_count,
        weak_association_count: input.weak_association_count,
        prompt_pattern_count: input.prompt_pattern_count,
        forecast_artifact: empty_artifact(),
        outcome_anchor: None,
        source_snapshot: empty_artifact(),
        score_artifact: empty_artifact(),
        prompt_artifact: empty_artifact(),
    }
}

fn empty_artifact() -> MistakeClosureArtifactRef {
    MistakeClosureArtifactRef {
        path: String::new(),
        blake3: "0".repeat(64),
    }
}

fn thresholds() -> MistakeClosureThresholds {
    MistakeClosureThresholds {
        min_sample_size: MISTAKE_CLOSURE_MIN_ROWS,
        min_error_brier: 0.25,
        min_brier_improvement: 0.05,
        max_calibration_abs_error: 0.25,
        min_association_recall_ratio: 0.85,
    }
}
