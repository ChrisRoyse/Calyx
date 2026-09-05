//! Issue #30 - persisted derived-feature computation stage FSV.
//!
//! Source of truth: raw derived-feature input JSON and normalized derived-feature row JSON read
//! back from disk.

use std::path::Path;

use calyx_poly::derived_feature_stage::{
    DERIVED_FEATURE_DEGRADED, DERIVED_FEATURE_INPUT_ARTIFACT_KIND, DERIVED_FEATURE_READY,
    DERIVED_FEATURE_SCHEMA_VERSION, DerivedFeatureInput, DerivedFeatureRow,
    run_derived_feature_stage,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const SNAPSHOT_TS: u64 = 1_785_600_030;

#[test]
fn issue030_derived_feature_stage_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE030_FSV_ROOT",
        "poly-issue030-derived-feature-stage",
    );
    reset_dir(&root);

    let happy = happy_computes_all_known_truth_features(&root);
    let zero_flow = edge_zero_flow_marks_ofi_absent(&root);
    let short_returns = edge_short_returns_marks_rv_absent(&root);
    let invalid = edge_invalid_raw_field_fails_closed_after_input_readback(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 30,
        "proof_claim": "Poly persists raw per-snapshot feature inputs, reads them back, computes OFI, holder/maker concentration, arbitrage residuals, distance-from-50, and realized volatility through the real feature formulas, persists the normalized row, and fails or degrades without fabricating missing values.",
        "minimum_sufficient_corpus": {
            "snapshots": 4,
            "happy_snapshots": 1,
            "edge_snapshots": 3,
            "why_this_is_sufficient": "One complete known-truth snapshot exercises every #30 derived feature; zero-flow and one-return snapshots prove absent/degraded feature paths; one invalid finite raw field proves fail-closed validation after input readback.",
            "why_smaller_is_insufficient": "Fewer than four snapshots would omit either a required feature family or one required fail/degrade behavior.",
            "why_larger_is_wasteful": "More snapshots repeat the same pure formula and JSON readback paths; scale is not the #30 proof claim."
        },
        "happy_path": happy,
        "edge_cases": {
            "zero_flow": zero_flow,
            "short_returns": short_returns,
            "invalid_raw_field": invalid
        },
        "physical_files": files
    });
    let readback_path = root.join("issue030_derived_feature_stage_fsv_report.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE030_DERIVED_FEATURE_STAGE_FSV={}",
        readback_path.display()
    );
}

fn happy_computes_all_known_truth_features(root: &Path) -> Value {
    let run = run_derived_feature_stage(&complete_input("happy"), &root.join("happy"))
        .expect("happy derived feature stage");
    let row = read_row(&run.row_path);
    assert_eq!(row, run.row);
    assert_eq!(row.status_code, DERIVED_FEATURE_READY);
    assert!(!row.degraded);
    assert!(row.absent_features.is_empty());
    assert_close(row.ofi.unwrap(), 0.50);
    assert_close(row.holder_herfindahl.unwrap(), 0.375);
    assert_close(row.top_holder_share.unwrap(), 0.50);
    assert_close(row.maker_herfindahl.unwrap(), 0.50);
    assert_close(row.top_maker_share.unwrap(), 0.50);
    assert_close(row.yes_no_residual.unwrap(), 0.04);
    assert_close(row.negrisk_sum_residual.unwrap(), -0.10);
    assert_close(row.distance_from_50.unwrap(), 0.12);
    assert_close(row.realized_vol.unwrap(), 0.02);
    evidence(&row)
}

fn edge_zero_flow_marks_ofi_absent(root: &Path) -> Value {
    let mut input = complete_input("edge-zero-flow");
    input.buy_volume = Some(0.0);
    input.sell_volume = Some(0.0);
    let run = run_derived_feature_stage(&input, &root.join("edge-zero-flow"))
        .expect("zero flow degrades without fabricating OFI");
    let row = read_row(&run.row_path);
    assert_eq!(row.status_code, DERIVED_FEATURE_DEGRADED);
    assert!(row.degraded);
    assert_eq!(row.ofi, None);
    assert!(row.absent_features.contains(&"ofi".to_string()));
    evidence(&row)
}

fn edge_short_returns_marks_rv_absent(root: &Path) -> Value {
    let mut input = complete_input("edge-short-returns");
    input.returns = vec![0.01];
    let run = run_derived_feature_stage(&input, &root.join("edge-short-returns"))
        .expect("short returns degrade without fabricating RV");
    let row = read_row(&run.row_path);
    assert_eq!(row.status_code, DERIVED_FEATURE_DEGRADED);
    assert!(row.degraded);
    assert_eq!(row.realized_vol, None);
    assert!(row.absent_features.contains(&"realized_vol".to_string()));
    evidence(&row)
}

fn edge_invalid_raw_field_fails_closed_after_input_readback(root: &Path) -> Value {
    let mut input = complete_input("edge-invalid-price");
    input.price = Some(1.20);
    let dir = root.join("edge-invalid-price");
    let err = run_derived_feature_stage(&input, &dir).expect_err("invalid price must fail closed");
    assert_eq!(err.code(), "CALYX_POLY_DERIVED_FEATURE_INVALID_REQUEST");
    assert!(
        err.message()
            .contains("price must be finite in [0, 1] when present")
    );
    let input_path = dir.join("edge-invalid-price-derived-feature-input.json");
    let readback: DerivedFeatureInput =
        serde_json::from_slice(&std::fs::read(&input_path).expect("read invalid input"))
            .expect("decode invalid input");
    assert_eq!(readback, input);
    assert!(
        !dir.join("edge-invalid-price-derived-feature-row.json")
            .exists()
    );
    json!({
        "error_code": err.code(),
        "error_message": err.message(),
        "input_path": input_path.display().to_string(),
        "row_written": false
    })
}

fn complete_input(source_id: &str) -> DerivedFeatureInput {
    DerivedFeatureInput {
        schema_version: DERIVED_FEATURE_SCHEMA_VERSION.to_string(),
        artifact_kind: DERIVED_FEATURE_INPUT_ARTIFACT_KIND.to_string(),
        source_id: source_id.to_string(),
        token_id: format!("tok-{source_id}"),
        condition_id: format!("cond-{source_id}"),
        snapshot_ts: SNAPSHOT_TS,
        buy_volume: Some(75.0),
        sell_volume: Some(25.0),
        holder_amounts: vec![50.0, 25.0, 25.0],
        maker_sizes: vec![10.0, 10.0],
        yes_price: Some(0.62),
        no_price: Some(0.42),
        negrisk_yes_prices: vec![0.30, 0.30, 0.30],
        price: Some(0.62),
        mid: Some(0.61),
        returns: vec![0.01, 0.03, 0.05],
    }
}

fn read_row(path: &Path) -> DerivedFeatureRow {
    serde_json::from_slice(&std::fs::read(path).expect("read derived row"))
        .expect("decode derived row")
}

fn evidence(row: &DerivedFeatureRow) -> Value {
    json!({
        "source_id": row.source_id,
        "status_code": row.status_code,
        "degraded": row.degraded,
        "absent_features": row.absent_features,
        "ofi": row.ofi,
        "holder_herfindahl": row.holder_herfindahl,
        "top_holder_share": row.top_holder_share,
        "maker_herfindahl": row.maker_herfindahl,
        "top_maker_share": row.top_maker_share,
        "yes_no_residual": row.yes_no_residual,
        "negrisk_sum_residual": row.negrisk_sum_residual,
        "distance_from_50": row.distance_from_50,
        "realized_vol": row.realized_vol
    })
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-9,
        "expected {expected}, got {actual}"
    );
}
