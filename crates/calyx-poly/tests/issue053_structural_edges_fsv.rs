//! Issue #53 - structural/arbitrage edges into Aster Graph CF.

#[path = "fsv_support.rs"]
#[allow(dead_code)]
mod support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId};
use calyx_poly::structural_edges::{
    EDGE_EVENT_SIBLING, EDGE_NEGRISK_SIBLING, EDGE_NESTED_DATE_CONTAINS, EDGE_YES_NO_COMPLEMENT,
    ERR_STRUCTURAL_GRAPH_INVALID_INPUT, StructuralDateRange, StructuralMarketInput,
    compute_structural_edges, persist_structural_edges_to_graph,
};
use serde_json::{Value, json};

use support::{collect_files, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue53-structural-edges";
const COLLECTION: &str = "poly_issue53_structural";

#[test]
fn issue053_structural_edges_fsv() {
    let root = issue53_root();
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let inputs = happy_inputs();
    let run = {
        let vault = open_vault(&vault_dir);
        persist_structural_edges_to_graph(&vault, COLLECTION, &inputs)
            .expect("persist structural graph")
    };
    assert_eq!(run.computed.input_count, 6);
    assert_eq!(edge_count(&run, EDGE_YES_NO_COMPLEMENT), 2);
    assert_eq!(edge_count(&run, EDGE_NEGRISK_SIBLING), 6);
    assert_eq!(edge_count(&run, EDGE_EVENT_SIBLING), 12);
    assert_eq!(edge_count(&run, EDGE_NESTED_DATE_CONTAINS), 1);
    assert_eq!(run.readback_edges.len(), run.computed.edges.len());

    let graph_readback = reopened_graph_readback(&vault_dir, &run);
    write_json(&root.join("graph-cf-readback.json"), &graph_readback);
    let edge_cases = edge_cases_fail_closed(&root);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 53,
        "proof_claim": "Structural YES/NO, negRisk, sibling, and nested-date edges are computed from explicit market metadata, persisted into Aster Graph CF, and read back from reopened Graph CF rows; missing relation evidence is Absent and non-finite prices fail closed.",
        "minimum_sufficient_proof_corpus": {
            "market_nodes": 6,
            "loaded_edges": run.computed.edge_count,
            "why_this_is_sufficient": "The six known-truth nodes cover every required relation: one binary YES/NO pair, one complete three-outcome negRisk set, event siblings, and one parent date range containing a child range.",
            "why_smaller_is_insufficient": "Fewer nodes cannot simultaneously prove a binary pair, a complete three-outcome negRisk residual, sibling fanout, and nested-date containment.",
            "why_larger_is_wasteful": "The behavior is deterministic grouping/residual logic plus Graph CF persistence; more markets would repeat the same code path without adding proof."
        },
        "graph_run": serde_json::to_value(&run).expect("run JSON"),
        "graph_cf_readback": graph_readback,
        "edge_cases": edge_cases,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue053_structural_edges_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

fn reopened_graph_readback(
    vault_dir: &Path,
    run: &calyx_poly::structural_edges::StructuralGraphRun,
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
        "edges": edges
    })
}

fn edge_cases_fail_closed(root: &Path) -> Vec<Value> {
    let single = compute_structural_edges(&[market(
        20,
        "single-condition",
        0,
        None,
        false,
        None,
        Some(0.61),
        None,
    )])
    .expect("single outcome computes absence");
    assert!(has_absence(&single, "single_outcome_market"));

    let incomplete = compute_structural_edges(&[
        market(
            21,
            "nr-a",
            0,
            Some("nr-missing"),
            true,
            Some(3),
            Some(0.30),
            None,
        ),
        market(
            22,
            "nr-b",
            1,
            Some("nr-missing"),
            true,
            Some(3),
            Some(0.35),
            None,
        ),
    ])
    .expect("incomplete negRisk computes absence");
    assert!(has_absence(&incomplete, "incomplete_negrisk_set"));
    assert_eq!(edge_count_set(&incomplete, EDGE_NEGRISK_SIBLING), 0);

    let lonely = compute_structural_edges(&[market(
        23,
        "lonely-event",
        0,
        Some("lonely"),
        false,
        None,
        Some(0.51),
        None,
    )])
    .expect("lonely event computes absence");
    assert!(has_absence(&lonely, "missing_sibling"));

    let bad = compute_structural_edges(&[market(
        24,
        "bad-price",
        0,
        None,
        false,
        None,
        Some(f64::NAN),
        None,
    )])
    .expect_err("non-finite price fails closed");
    assert_eq!(bad.code(), ERR_STRUCTURAL_GRAPH_INVALID_INPUT);

    let edge_report = json!({
        "single_outcome_market": single,
        "incomplete_negrisk_set": incomplete,
        "missing_sibling": lonely,
        "non_finite_price": bad.diagnostic()
    });
    write_json(&root.join("edge-cases.json"), &edge_report);
    vec![
        json!({"case": "single_outcome_market", "after": edge_report["single_outcome_market"]}),
        json!({"case": "incomplete_negrisk_set", "after": edge_report["incomplete_negrisk_set"]}),
        json!({"case": "missing_sibling", "after": edge_report["missing_sibling"]}),
        json!({"case": "non_finite_price", "after": edge_report["non_finite_price"]}),
    ]
}

fn happy_inputs() -> Vec<StructuralMarketInput> {
    vec![
        market(1, "binary-temp", 0, None, false, None, Some(0.62), None),
        market(2, "binary-temp", 1, None, false, None, Some(0.41), None),
        market(
            3,
            "election-a",
            0,
            Some("event-election"),
            true,
            Some(3),
            Some(0.20),
            Some(range(100, 200)),
        ),
        market(
            4,
            "election-b",
            1,
            Some("event-election"),
            true,
            Some(3),
            Some(0.30),
            None,
        ),
        market(
            5,
            "election-c",
            2,
            Some("event-election"),
            true,
            Some(3),
            Some(0.45),
            None,
        ),
        market(
            6,
            "election-parent-month",
            0,
            Some("event-election"),
            false,
            None,
            Some(0.50),
            Some(range(50, 250)),
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn market(
    id: u8,
    condition_id: &str,
    outcome_index: u32,
    event_id: Option<&str>,
    neg_risk: bool,
    expected: Option<usize>,
    price: Option<f64>,
    date_range: Option<StructuralDateRange>,
) -> StructuralMarketInput {
    StructuralMarketInput {
        cx_id: cx(id),
        condition_id: condition_id.to_string(),
        token_id: format!("token-{id}"),
        outcome_index,
        event_id: event_id.map(str::to_string),
        neg_risk,
        expected_neg_risk_outcomes: expected,
        price,
        date_range,
    }
}

fn edge_count(run: &calyx_poly::structural_edges::StructuralGraphRun, edge_type: &str) -> usize {
    edge_count_set(&run.computed, edge_type)
}

fn edge_count_set(set: &calyx_poly::structural_edges::StructuralEdgeSet, edge_type: &str) -> usize {
    set.edges
        .iter()
        .filter(|edge| edge.edge_type == edge_type)
        .count()
}

fn has_absence(set: &calyx_poly::structural_edges::StructuralEdgeSet, code: &str) -> bool {
    set.absent.iter().any(|absence| absence.code == code)
}

fn range(start_ts: u64, end_ts: u64) -> StructuralDateRange {
    StructuralDateRange { start_ts, end_ts }
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
    .expect("open issue53 vault")
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("vault id")
}

fn issue53_root() -> PathBuf {
    if let Some(path) = std::env::var_os("POLY_ISSUE53_FSV_ROOT") {
        return PathBuf::from(path);
    }
    repo_root().join("target/fsv/issue53_structural_edges_20260707")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}

fn assert_c_drive(path: &Path) {
    #[cfg(windows)]
    assert!(
        path.to_string_lossy().starts_with("C:"),
        "{} must stay on C:",
        path.display()
    );
}
