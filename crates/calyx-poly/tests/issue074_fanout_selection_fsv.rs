use std::fs;
use std::path::Path;

use calyx_poly::{
    ExpensiveAssociationEstimator, FanoutCandidate, FanoutDropReason, FanoutSelectionRequest,
    FanoutThresholds, PolyError, read_fanout_selection_report, run_fanout_selection_report,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue074_bounded_fanout_selection_fsv() {
    let (root, keep_root) =
        named_fsv_root("POLY_ISSUE074_FSV_ROOT", "poly-issue074-fanout-selection");
    reset_dir(&root);

    let happy = happy_path(&root);
    let empty = edge_case(
        &root,
        "edge-empty-input",
        |mut request| {
            request.candidates.clear();
            request
        },
        "CALYX_POLY_FANOUT_SELECTION_EMPTY",
    );
    let invalid_metric = edge_case(
        &root,
        "edge-invalid-score",
        |mut request| {
            request.candidates[0].abs_spearman = 1.2;
            request
        },
        "CALYX_POLY_FANOUT_SELECTION_INVALID_CANDIDATE",
    );
    let duplicate = edge_case(
        &root,
        "edge-duplicate-pair",
        |mut request| {
            let mut duplicate = request.candidates[0].clone();
            std::mem::swap(&mut duplicate.left_key, &mut duplicate.right_key);
            duplicate.pair_id = "duplicate-reversed".to_string();
            request.candidates.push(duplicate);
            request
        },
        "CALYX_POLY_FANOUT_SELECTION_DUPLICATE_PAIR",
    );
    let invalid_request = edge_case(
        &root,
        "edge-zero-cap",
        |mut request| {
            request.thresholds.max_expensive_candidates = 0;
            request
        },
        "CALYX_POLY_FANOUT_SELECTION_INVALID_REQUEST",
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 74,
            "proof_claim": "Poly selects a bounded expensive-confirm set from cheap NMI, absolute Spearman, and edge-weight screens, and persists every selected and dropped pair with reasons.",
            "minimum_sufficient_corpus": {
                "chosen_candidates": 6,
                "why_sufficient": "Six candidate pairs prove all three cheap screen channels, deterministic top-3 fan-out, two cap drops, one below-threshold drop, report persistence, and readback.",
                "why_smaller_insufficient": "Fewer candidates would not prove all three screen channels plus both drop reasons in one happy-path corpus.",
                "why_larger_wasteful": "More candidates would repeat the same ranking, cap, drop logging, and readback behavior without adding #74 correctness proof."
            },
            "source_of_truth": "local JSON candidate-score corpus plus persisted fan-out selection report",
            "happy_path": happy,
            "edge_cases": {
                "empty": empty,
                "invalid_metric": invalid_metric,
                "duplicate": duplicate,
                "invalid_request": invalid_request
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);

    if keep_root {
        println!("poly_issue074_fsv_root={}", root.display());
    }
}

fn happy_path(root: &Path) -> Value {
    let out_dir = root.join("happy");
    let report_path = out_dir.join("fanout_selection_crypto_v1.json");
    let corpus_path = out_dir.join("cheap-screen-candidates.json");
    let before = report_state(&report_path);
    let request = known_request();
    write_json(
        &corpus_path,
        &serde_json::to_value(&request).expect("request JSON"),
    );
    let corpus_readback: FanoutSelectionRequest =
        serde_json::from_slice(&fs::read(&corpus_path).expect("read corpus"))
            .expect("decode corpus");
    assert_eq!(corpus_readback, request);

    let run = run_fanout_selection_report(&out_dir, &corpus_readback)
        .expect("known fan-out selection should pass");
    let readback = read_fanout_selection_report(&run.report_path).expect("read report");
    assert_eq!(readback, run.report);
    assert_eq!(readback.input_count, 6);
    assert_eq!(readback.selected_count, 3);
    assert_eq!(readback.dropped_count, 3);
    let selected: Vec<_> = readback
        .selected
        .iter()
        .map(|decision| decision.pair_id.as_str())
        .collect();
    assert_eq!(selected, vec!["pair-nmi", "pair-spearman", "pair-edge"]);
    let cap_drops = readback
        .dropped
        .iter()
        .filter(|d| d.drop_reason == Some(FanoutDropReason::FanoutLimit))
        .count();
    let below_drops = readback
        .dropped
        .iter()
        .filter(|d| d.drop_reason == Some(FanoutDropReason::BelowCheapScreen))
        .count();
    assert_eq!(cap_drops, 2);
    assert_eq!(below_drops, 1);
    assert!(
        readback.dropped.iter().all(|d| d.drop_reason.is_some()),
        "every dropped pair must carry an explicit reason"
    );

    let after = report_state(&report_path);
    let evidence = json!({
        "trigger": "screen six known cheap-score pairs with max_expensive_candidates=3",
        "before": before,
        "source_corpus_readback": corpus_readback,
        "after": after,
        "expected": {
            "selected": selected,
            "cap_drops": cap_drops,
            "below_threshold_drops": below_drops
        },
        "readback": readback
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_case<F>(root: &Path, name: &str, mutate: F, expected_code: &str) -> Value
where
    F: FnOnce(FanoutSelectionRequest) -> FanoutSelectionRequest,
{
    let out_dir = root.join(name);
    let report_path = out_dir.join("fanout_selection_crypto_v1.json");
    let corpus_path = out_dir.join("cheap-screen-candidates.json");
    let request = mutate(known_request());
    let before = report_state(&report_path);
    write_json(
        &corpus_path,
        &serde_json::to_value(&request).expect("request JSON"),
    );
    let corpus_readback: FanoutSelectionRequest =
        serde_json::from_slice(&fs::read(&corpus_path).expect("read edge corpus"))
            .expect("decode edge corpus");
    let err =
        run_fanout_selection_report(&out_dir, &corpus_readback).expect_err("edge must fail closed");
    let error = error_json(err);
    assert_eq!(error["code"], json!(expected_code));
    let after = report_state(&report_path);
    assert_eq!(after["present"], json!(false));
    let evidence = json!({
        "trigger": name,
        "expected_code": expected_code,
        "actual_code": error["code"],
        "before": before,
        "source_corpus_readback": corpus_readback,
        "after": after,
        "error": error
    });
    write_json(&root.join(format!("{name}-readback.json")), &evidence);
    evidence
}

fn known_request() -> FanoutSelectionRequest {
    FanoutSelectionRequest {
        domain: "crypto".to_string(),
        panel_version: 1,
        thresholds: FanoutThresholds {
            min_normalized_mutual_info: 0.20,
            min_abs_spearman: 0.20,
            min_edge_weight: 0.15,
            max_expensive_candidates: 3,
        },
        expensive_estimators: vec![
            ExpensiveAssociationEstimator::TransferEntropy,
            ExpensiveAssociationEstimator::DistanceCorrelation,
            ExpensiveAssociationEstimator::PermutationConfirm,
        ],
        candidates: vec![
            candidate("pair-nmi", "price", "outcome", 0.80, 0.04, 0.02),
            candidate("pair-spearman", "flow", "price", 0.05, 0.70, 0.02),
            candidate("pair-edge", "holder", "maker", 0.04, 0.05, 0.45),
            candidate("pair-cap-spearman", "volume", "spread", 0.04, 0.24, 0.02),
            candidate("pair-cap-nmi", "volatility", "category", 0.22, 0.04, 0.02),
            candidate("pair-below", "noise-a", "noise-b", 0.05, 0.04, 0.03),
        ],
    }
}

fn candidate(
    pair_id: &str,
    left_key: &str,
    right_key: &str,
    normalized_mutual_info: f64,
    abs_spearman: f64,
    edge_weight: f64,
) -> FanoutCandidate {
    FanoutCandidate {
        pair_id: pair_id.to_string(),
        left_key: left_key.to_string(),
        right_key: right_key.to_string(),
        normalized_mutual_info,
        abs_spearman,
        edge_weight,
        provenance: "synthetic-known-cheap-screen-scores".to_string(),
    }
}

fn report_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    let parsed = bytes
        .as_ref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok());
    json!({
        "path": path.display().to_string(),
        "present": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len),
        "blake3": bytes.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string()),
        "json": parsed
    })
}

fn error_json(error: PolyError) -> Value {
    json!({
        "code": error.code(),
        "message": error.message()
    })
}
