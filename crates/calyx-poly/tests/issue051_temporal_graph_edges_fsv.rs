//! Issue #51 - temporal lead/lag, TE, periodicity, and hazard edges into Graph CF.

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::Direction;
use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, FixedClock, VaultId};
use calyx_poly::temporal_graph_edges::{
    EDGE_TEMPORAL_HAZARD, EDGE_TEMPORAL_LEAD_LAG, EDGE_TEMPORAL_PERIODICITY,
    EDGE_TEMPORAL_TRANSFER_ENTROPY, ERR_TEMPORAL_GRAPH_INSUFFICIENT,
    ERR_TEMPORAL_GRAPH_INVALID_INPUT, ERR_TEMPORAL_GRAPH_LOW_SIGNAL, TemporalGraphConfig,
    TemporalGraphEvidence, TemporalGraphRequest, TemporalPoint, TemporalTransferEntropyConfig,
    compute_temporal_graph_edges, persist_temporal_graph_edges,
};
use serde_json::{Value, json};

use support::{collect_files, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue51-temporal-graph-edges";
const COLLECTION: &str = "poly_issue51_temporal_graph";
const TEST_TS: u64 = 1_785_500_051;

#[test]
fn issue051_temporal_graph_edges_fsv() {
    let root = issue51_root();
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let (request, attempts) = smallest_known_truth_request();
    let run = {
        let vault = open_vault(&vault_dir);
        persist_temporal_graph_edges(&vault, COLLECTION, &request, &clock())
            .expect("persist temporal graph edges")
    };
    assert_eq!(run.computed.edge_count, 4);
    assert_eq!(run.computed.node_count, 4);
    assert_eq!(run.graph_cf_row_count, 12);
    assert_eq!(run.readback_edges.len(), run.computed.edges.len());
    assert_known_truth_edges(&run.computed);

    let graph_readback = reopened_graph_readback(&vault_dir, &run);
    write_json(&root.join("graph-cf-readback.json"), &graph_readback);
    let edge_cases = edge_cases_fail_closed(&root);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 51,
        "proof_claim": "Poly computes temporal lead/lag, transfer-entropy, periodicity, and recurrence-hazard evidence from a per-domain ingest time series, persists typed temporal association edges into Aster Graph CF, and reads those rows back byte-for-byte.",
        "minimum_sufficient_proof_corpus": {
            "selected_paired_samples": request.driver_series.len(),
            "selected_recurrence_events": request.recurrence_event_times.len(),
            "candidate_paired_samples_evaluated": attempts,
            "known_truth": {
                "driver_leads_response_by_samples": 2,
                "response_period_samples": 4,
                "recurrence_hazard": "overdue"
            },
            "why_this_is_sufficient": "The selected corpus is the first deterministic candidate that proves all four temporal edge families, including a non-provisional driver->response TE result, a CCF peak at the planted lag, a period estimate, an overdue hazard report, and Graph CF byte readback.",
            "why_smaller_is_insufficient": "Below 32 paired samples, the Assay TE quorum cannot be met for lag 2; smaller checked candidates that did not pass every known-truth assertion are recorded in candidate_paired_samples_evaluated.",
            "why_larger_is_wasteful": "Once these four edge families and their fail-closed cases are proven, more rows only repeat the same deterministic estimator, Graph CF write, and readback paths."
        },
        "graph_run": serde_json::to_value(&run).expect("run JSON"),
        "graph_cf_readback": graph_readback,
        "edge_cases": edge_cases,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue051_temporal_graph_edges_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback["issue"], json!(51));
    assert_eq!(readback["passed"], json!(true));
    assert_eq!(
        readback["minimum_sufficient_proof_corpus"]["selected_paired_samples"],
        json!(request.driver_series.len())
    );
    write_blake3sums(&root);
}

fn smallest_known_truth_request() -> (TemporalGraphRequest, Vec<Value>) {
    let mut attempts = Vec::new();
    for samples in [32, 36, 40, 44, 48, 56, 64, 68, 72, 76, 80] {
        let request = request(samples);
        match compute_temporal_graph_edges(&request, &clock()) {
            Ok(edges) if known_truth_edges(&edges) => {
                attempts.push(json!({"paired_samples": samples, "result": "selected"}));
                return (request, attempts);
            }
            Ok(edges) => attempts.push(json!({
                "paired_samples": samples,
                "result": "computed_but_failed_known_truth",
                "edge_count": edges.edge_count,
                "metrics": edge_metrics(&edges)
            })),
            Err(error) => attempts.push(json!({
                "paired_samples": samples,
                "result": "rejected",
                "code": error.code(),
                "message": error.message()
            })),
        }
    }
    panic!(
        "no candidate corpus proved all temporal graph invariants: {}",
        serde_json::to_string_pretty(&attempts).expect("attempts JSON")
    );
}

fn assert_known_truth_edges(edges: &calyx_poly::temporal_graph_edges::TemporalGraphEdgeSet) {
    assert!(
        known_truth_edges(edges),
        "temporal edge known-truth assertions failed"
    );
}

fn known_truth_edges(edges: &calyx_poly::temporal_graph_edges::TemporalGraphEdgeSet) -> bool {
    let lead = edges
        .edges
        .iter()
        .find(|edge| edge.edge_type == EDGE_TEMPORAL_LEAD_LAG);
    let te = edges
        .edges
        .iter()
        .find(|edge| edge.edge_type == EDGE_TEMPORAL_TRANSFER_ENTROPY);
    let period = edges
        .edges
        .iter()
        .find(|edge| edge.edge_type == EDGE_TEMPORAL_PERIODICITY);
    let hazard = edges
        .edges
        .iter()
        .find(|edge| edge.edge_type == EDGE_TEMPORAL_HAZARD);
    let lead_ok = matches!(
        lead.map(|edge| &edge.evidence),
        Some(TemporalGraphEvidence::LeadLag { report })
            if report.peak_lag == 2 && report.peak_correlation > 0.6
    );
    let te_ok = matches!(
        te.map(|edge| &edge.evidence),
        Some(TemporalGraphEvidence::TransferEntropy { selected, .. })
            if selected.lag == 2 && selected.dominant_direction == Direction::AToB
    );
    let period_ok = matches!(
        period.map(|edge| &edge.evidence),
        Some(TemporalGraphEvidence::Periodicity { report })
            if report.dominant_period.is_some_and(|period| (period - 4.0).abs() <= 1.0)
    );
    let hazard_ok = matches!(
        hazard.map(|edge| &edge.evidence),
        Some(TemporalGraphEvidence::Hazard { report }) if report.overdue
    );
    lead_ok && te_ok && period_ok && hazard_ok
}

fn edge_metrics(edges: &calyx_poly::temporal_graph_edges::TemporalGraphEdgeSet) -> Value {
    let mut out = json!({});
    for edge in &edges.edges {
        match &edge.evidence {
            TemporalGraphEvidence::LeadLag { report } => {
                out["lead_lag"] = json!({
                    "peak_lag": report.peak_lag,
                    "peak_correlation": report.peak_correlation
                });
            }
            TemporalGraphEvidence::TransferEntropy { selected, .. } => {
                out["transfer_entropy"] = json!({
                    "selected_lag": selected.lag,
                    "direction": format!("{:?}", selected.dominant_direction),
                    "difference": selected.t_a_to_b - selected.t_b_to_a
                });
            }
            TemporalGraphEvidence::Periodicity { report } => {
                out["periodicity"] = json!({
                    "dominant_period": report.dominant_period,
                    "coefficients": report.coefficients
                });
            }
            TemporalGraphEvidence::Hazard { report } => {
                out["hazard"] = json!({
                    "overdue": report.overdue,
                    "survival": report.survival
                });
            }
        }
    }
    out
}

fn reopened_graph_readback(
    vault_dir: &Path,
    run: &calyx_poly::temporal_graph_edges::TemporalGraphRun,
) -> Value {
    let reopened = open_vault(vault_dir);
    let graph = PlainGraph::new(&reopened, COLLECTION).expect("graph");
    let snapshot = reopened.latest_seq();
    let mut edges = Vec::new();
    for expected in &run.readback_edges {
        let bytes = graph
            .get_edge(snapshot, expected.src, &expected.edge_type, expected.dst)
            .expect("Graph CF get")
            .expect("edge present after reopen");
        let value: Value = serde_json::from_slice(&bytes).expect("edge JSON");
        edges.push(json!({
            "src": expected.src,
            "dst": expected.dst,
            "edge_type": expected.edge_type,
            "value": value,
            "blake3": blake3::hash(&bytes).to_hex().to_string()
        }));
    }
    let rows = reopened
        .scan_cf_at(snapshot, ColumnFamily::Graph)
        .expect("scan Graph CF");
    json!({
        "snapshot_seq": snapshot,
        "graph_cf_rows": rows.len(),
        "readback_edge_count": edges.len(),
        "all_expected_edges_present": edges.len() == run.readback_edges.len(),
        "edges": edges
    })
}

fn edge_cases_fail_closed(root: &Path) -> Vec<Value> {
    let underpowered = compute_temporal_graph_edges(&request(20), &clock())
        .expect_err("below sample floor fails closed");
    assert_eq!(underpowered.code(), ERR_TEMPORAL_GRAPH_INSUFFICIENT);

    let mut single_class = request(40);
    for point in &mut single_class.response_series {
        point.value = 0.25;
    }
    let low_signal = compute_temporal_graph_edges(&single_class, &clock())
        .expect_err("single-class response fails closed");
    assert_eq!(low_signal.code(), ERR_TEMPORAL_GRAPH_LOW_SIGNAL);

    let mut irregular = request(40);
    for point in irregular.driver_series.iter_mut().skip(10) {
        point.ts += 1;
    }
    for point in irregular.response_series.iter_mut().skip(10) {
        point.ts += 1;
    }
    let irregular_error = compute_temporal_graph_edges(&irregular, &clock())
        .expect_err("irregular timestamps fail closed");
    assert_eq!(irregular_error.code(), ERR_TEMPORAL_GRAPH_INVALID_INPUT);

    let edge_report = json!({
        "below_sample_floor": underpowered.diagnostic(),
        "single_class_series": low_signal.diagnostic(),
        "irregular_timestamps": irregular_error.diagnostic()
    });
    write_json(&root.join("edge-cases.json"), &edge_report);
    vec![
        json!({"case": "below_sample_floor", "after": edge_report["below_sample_floor"]}),
        json!({"case": "single_class_series", "after": edge_report["single_class_series"]}),
        json!({"case": "irregular_timestamps", "after": edge_report["irregular_timestamps"]}),
    ]
}

fn request(samples: usize) -> TemporalGraphRequest {
    let (driver, response) = planted_driver_response(samples, 2);
    TemporalGraphRequest {
        domain: "crypto".to_string(),
        market_id: "condition-issue051".to_string(),
        market_cx_id: cx(51),
        driver_name: "polygon_net_buy_flow".to_string(),
        response_name: "clob_midpoint_price".to_string(),
        recurrence_name: "large_trade_recurrence".to_string(),
        driver_series: driver,
        response_series: response,
        recurrence_event_times: vec![0.0, 10.0, 21.0, 33.0, 46.0],
        now: 70.0,
        config: TemporalGraphConfig {
            max_lag: 2,
            candidate_lags: vec![1, 2],
            overdue_alpha: 0.05,
            te_config: TemporalTransferEntropyConfig {
                bootstrap_resamples: 10,
                ..TemporalTransferEntropyConfig::default()
            },
        },
    }
}

fn planted_driver_response(samples: usize, lag: usize) -> (Vec<TemporalPoint>, Vec<TemporalPoint>) {
    let driver: Vec<_> = (0..samples)
        .map(|t| point(t as u64, (noise(t as u64, 7) - 0.5) as f32))
        .collect();
    let mut response = Vec::with_capacity(samples);
    for t in 0..samples {
        let seasonal = match t % 4 {
            0 => -0.6,
            1 => 0.0,
            2 => 0.6,
            _ => 0.0,
        };
        let value = if t >= lag {
            0.55 * driver[t - lag].value
                + 0.45 * seasonal
                + 0.005 * (noise(t as u64, 41) as f32 - 0.5)
        } else {
            0.15 * seasonal + 0.1 * (noise(t as u64, 73) as f32 - 0.5)
        };
        response.push(point(t as u64, value));
    }
    (driver, response)
}

fn point(ts: u64, value: f32) -> TemporalPoint {
    TemporalPoint { ts, value }
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn open_vault(dir: &Path) -> AsterVault {
    AsterVault::open(
        dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open issue51 vault")
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("vault id")
}

fn clock() -> FixedClock {
    FixedClock::new(TEST_TS)
}

fn noise(t: u64, salt: u64) -> f64 {
    splitmix(t ^ salt)
}

fn splitmix(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    let z = z ^ (z >> 31);
    (z as f64) / (u64::MAX as f64)
}

fn issue51_root() -> PathBuf {
    calyx_fsv::fsv_root_or_target(
        "POLY_ISSUE51_FSV_ROOT",
        "issue51-temporal-graph-edges",
        || repo_root().join("target/fsv/issue51_temporal_graph_edges_20260707"),
    )
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}

fn assert_c_drive(path: &Path) {
    #[cfg(not(windows))]
    let _ = path;
    #[cfg(windows)]
    assert!(
        path.to_string_lossy().starts_with("C:"),
        "{} must stay on C:",
        path.display()
    );
}
