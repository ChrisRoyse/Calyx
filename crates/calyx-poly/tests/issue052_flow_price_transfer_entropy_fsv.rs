//! Issue #52 — flow -> price transfer entropy wiring.
//!
//! Source of truth: the persisted Poly flow-price transfer-entropy report JSON
//! written after the real `calyx-assay` TE sweep, then read back independently.

use std::fs;
use std::path::Path;

use calyx_assay::Direction;
use calyx_core::FixedClock;
use calyx_poly::flow_price_transfer_entropy::{
    ERR_FLOW_PRICE_TE_INVALID_REQUEST, ERR_FLOW_PRICE_TE_NO_DIRECTIONAL_SIGNAL,
    FLOW_PRICE_TE_SCHEMA_VERSION, FlowPricePoint, FlowPriceTransferEntropyConfig,
    FlowPriceTransferEntropyRequest, read_flow_price_transfer_entropy_report,
    run_flow_price_transfer_entropy,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const TEST_TS: u64 = 1_785_500_052;

#[test]
fn issue052_flow_price_transfer_entropy_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE052_FSV_ROOT", "poly-issue052-flow-price-te");
    reset_dir(&root);

    let happy = happy_flow_drives_price(&root);
    let underpowered = edge_underpowered_fails_closed(&root);
    let non_finite = edge_non_finite_fails_closed(&root);
    let zero_lag = edge_zero_lag_fails_closed(&root);
    let reverse = edge_reverse_direction_fails_closed(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 52,
        "proof_claim": "Poly wires on-chain flow series to price series through the real calyx-assay transfer_entropy lag sweep, persists the selected flow->price association report, and reads it back as the source of truth.",
        "minimum_sufficient_corpus": {
            "happy_path_series_per_side": 140,
            "candidate_lags": [1, 2, 4, 8],
            "planted_lag": 2,
            "edge_cases": 4,
            "why_this_is_sufficient": "One known-truth flow->price series proves the Poly wrapper uses the real Assay TE sweep, recovers the planted lag, persists the selected association, and reads the report back. The four edge cases cover no usable quorum, invalid sample values, invalid lag configuration, and the wrong causal direction.",
            "why_smaller_is_insufficient": "Fewer than the Assay quorum cannot prove a non-provisional TE association; omitting lag competitors would not prove lag selection; fewer edge cases would leave either invalid request handling or directionality unverified.",
            "why_larger_is_wasteful": "More samples or more markets would repeat the same validation, Assay sweep, selection, write, and readback paths without adding a distinct #52 invariant."
        },
        "source_of_truth": "persisted flow_price_transfer_entropy report JSON read back from disk",
        "happy_path": happy,
        "edge_cases": {
            "underpowered": underpowered,
            "non_finite": non_finite,
            "zero_lag": zero_lag,
            "reverse_direction": reverse
        },
        "physical_files": files
    });
    let summary_path = root.join("issue052_flow_price_transfer_entropy_fsv_report.json");
    write_json(&summary_path, &summary);
    write_blake3sums(&root);
    println!("ISSUE052_FLOW_PRICE_TE_FSV={}", summary_path.display());
}

fn happy_flow_drives_price(root: &Path) -> Value {
    let case_dir = root.join("happy");
    let reports_dir = case_dir.join("reports");
    let before = dir_file_count(&reports_dir);
    let (flow, price) = planted_flow_drives_price(140, 2);
    let request = request(flow, price, vec![1, 2, 4, 8]);
    let run = run_flow_price_transfer_entropy(request, &reports_dir, &clock()).unwrap();
    assert_eq!(run.report.schema_version, FLOW_PRICE_TE_SCHEMA_VERSION);
    assert_eq!(run.report.selected_lag, 2);
    assert_eq!(run.report.selected_direction, Direction::AToB);
    assert!(run.report.selected_flow_to_price_te > run.report.selected_price_to_flow_te + 0.1);
    assert_eq!(run.report.sweep_order, vec![1, 2, 4, 8]);

    let readback = read_flow_price_transfer_entropy_report(&run.report_path).unwrap();
    assert_eq!(readback, run.report);
    let bytes = fs::read(&run.report_path).unwrap();
    let evidence = json!({
        "report_path": run.report_path.display().to_string(),
        "report_blake3": blake3::hash(&bytes).to_hex().to_string(),
        "report_bytes": bytes.len(),
        "reports_before": before,
        "reports_after": dir_file_count(&reports_dir),
        "selected_lag": run.report.selected_lag,
        "selected_direction": format!("{:?}", run.report.selected_direction),
        "selected_difference": run.report.selected_difference,
        "selected_n_samples": run.report.selected_n_samples,
        "sweep_order": run.report.sweep_order
    });
    write_json(&case_dir.join("happy_readback.json"), &evidence);
    evidence
}

fn edge_underpowered_fails_closed(root: &Path) -> Value {
    let case_dir = root.join("edge-underpowered");
    let reports_dir = case_dir.join("reports");
    let request = request(
        vec![point(0, 0.1), point(1, 0.2)],
        vec![point(0, 0.3), point(1, 0.4)],
        vec![1],
    );
    let err = run_flow_price_transfer_entropy(request, &reports_dir, &clock()).unwrap_err();
    assert_eq!(err.code(), ERR_FLOW_PRICE_TE_NO_DIRECTIONAL_SIGNAL);
    let evidence = json!({
        "before": {"samples_per_side": 2, "lag": 1},
        "after": {"code": err.code(), "reports_written": dir_file_count(&reports_dir)}
    });
    write_json(&case_dir.join("edge_underpowered.json"), &evidence);
    evidence
}

fn edge_non_finite_fails_closed(root: &Path) -> Value {
    let case_dir = root.join("edge-non-finite");
    let reports_dir = case_dir.join("reports");
    let (mut flow, price) = planted_flow_drives_price(40, 2);
    flow[5].value = f32::NAN;
    let err =
        run_flow_price_transfer_entropy(request(flow, price, vec![1, 2]), &reports_dir, &clock())
            .unwrap_err();
    assert_eq!(err.code(), ERR_FLOW_PRICE_TE_INVALID_REQUEST);
    let evidence = json!({
        "before": {"flow_sample": 5, "value": "NaN"},
        "after": {"code": err.code(), "reports_written": dir_file_count(&reports_dir)}
    });
    write_json(&case_dir.join("edge_non_finite.json"), &evidence);
    evidence
}

fn edge_zero_lag_fails_closed(root: &Path) -> Value {
    let case_dir = root.join("edge-zero-lag");
    let reports_dir = case_dir.join("reports");
    let (flow, price) = planted_flow_drives_price(40, 2);
    let err =
        run_flow_price_transfer_entropy(request(flow, price, vec![0]), &reports_dir, &clock())
            .unwrap_err();
    assert_eq!(err.code(), ERR_FLOW_PRICE_TE_INVALID_REQUEST);
    let evidence = json!({
        "before": {"candidate_lags": [0]},
        "after": {"code": err.code(), "reports_written": dir_file_count(&reports_dir)}
    });
    write_json(&case_dir.join("edge_zero_lag.json"), &evidence);
    evidence
}

fn edge_reverse_direction_fails_closed(root: &Path) -> Value {
    let case_dir = root.join("edge-reverse-direction");
    let reports_dir = case_dir.join("reports");
    let (price_driver, flow_target) = planted_flow_drives_price(140, 2);
    let err = run_flow_price_transfer_entropy(
        request(flow_target, price_driver, vec![1, 2, 4, 8]),
        &reports_dir,
        &clock(),
    )
    .unwrap_err();
    assert_eq!(err.code(), ERR_FLOW_PRICE_TE_NO_DIRECTIONAL_SIGNAL);
    let evidence = json!({
        "before": {"known_truth": "price drives flow at lag 2, not flow drives price"},
        "after": {"code": err.code(), "reports_written": dir_file_count(&reports_dir)}
    });
    write_json(&case_dir.join("edge_reverse_direction.json"), &evidence);
    evidence
}

fn request(
    flow_series: Vec<FlowPricePoint>,
    price_series: Vec<FlowPricePoint>,
    candidate_lags: Vec<usize>,
) -> FlowPriceTransferEntropyRequest {
    FlowPriceTransferEntropyRequest {
        domain: "crypto".to_string(),
        market_id: "condition-issue052".to_string(),
        flow_source: "polygon_rpc_order_filled_net_flow".to_string(),
        price_source: "clob_midpoint_price".to_string(),
        flow_series,
        price_series,
        candidate_lags,
        config: FlowPriceTransferEntropyConfig {
            bootstrap_resamples: 20,
            ..FlowPriceTransferEntropyConfig::default()
        },
    }
}

fn planted_flow_drives_price(n: usize, lag: usize) -> (Vec<FlowPricePoint>, Vec<FlowPricePoint>) {
    let flow: Vec<_> = (0..n)
        .map(|t| point(t as u64, 0.2 + 0.6 * noise(t as u64, 7)))
        .collect();
    let mut price = Vec::with_capacity(n);
    for t in 0..n {
        let value = if t >= lag {
            flow[t - lag].value + 0.01 * (noise(t as u64, 41) - 0.5)
        } else {
            noise(t as u64, 73)
        };
        price.push(point(t as u64, value));
    }
    (flow, price)
}

fn point(ts: u64, value: f32) -> FlowPricePoint {
    FlowPricePoint { ts, value }
}

fn dir_file_count(dir: &Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    fs::read_dir(dir).unwrap().count()
}

fn clock() -> FixedClock {
    FixedClock::new(TEST_TS)
}

fn noise(t: u64, salt: u64) -> f32 {
    splitmix(t ^ salt) as f32
}

fn splitmix(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    let z = z ^ (z >> 31);
    (z as f64) / (u64::MAX as f64)
}
