//! Issue #72 - kNN resolved-neighbor edges into Aster Graph CF on ingest.

#[path = "fsv_support.rs"]
#[allow(dead_code)]
mod support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId};
use calyx_poly::knn_base_rate::ResolvedExemplar;
use calyx_poly::knn_graph_edges::{
    EDGE_KNN_RESOLVED, ERR_KNN_GRAPH_DIM_MISMATCH, ERR_KNN_GRAPH_EMPTY_CORPUS,
    ERR_KNN_GRAPH_INVALID_K, ERR_KNN_GRAPH_NON_FINITE, KnnGraphRun, compute_knn_edges,
    persist_knn_edges_on_ingest,
};
use serde_json::{Value, json};

use support::{collect_files, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue72-knn-graph-edges";
const COLLECTION: &str = "poly_issue72_knn_graph";

#[test]
fn issue072_knn_graph_edges_fsv() {
    let root = issue72_root();
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let ingested = exemplar(10, &[1.0, 0.0], true);
    let corpus = happy_corpus();
    let run = {
        let vault = open_vault(&vault_dir);
        persist_knn_edges_on_ingest(&vault, COLLECTION, "crypto", &ingested, &corpus, 2)
            .expect("persist kNN graph edges")
    };
    assert_eq!(run.edge_count, 2);
    assert_eq!(run.graph_cf_row_count, 7);
    assert_eq!(run.edges[0].dst, cx(1));
    assert_eq!(run.edges[1].dst, cx(2));
    assert!(
        run.edges
            .iter()
            .all(|edge| edge.edge_type == EDGE_KNN_RESOLVED)
    );
    assert!(!run.edges.iter().any(|edge| edge.dst == cx(3)));
    assert_eq!(run.readback_edges.len(), run.edges.len());
    assert_eq!(
        run.readback_edges[0].value.source,
        "calyx_poly::direct_cosine_top_k"
    );

    let graph_readback = reopened_graph_readback(&vault_dir, &run);
    write_json(&root.join("graph-cf-readback.json"), &graph_readback);
    let edge_cases = edge_cases_fail_closed(&root, &ingested, &corpus);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 72,
        "proof_claim": "On ingest of a resolved record, Poly computes deterministic direct cosine top-k over the resolved corpus, persists typed association.knn_resolved edges into Aster Graph CF, and reads those Graph CF rows back byte-for-byte.",
        "minimum_sufficient_proof_corpus": {
            "ingested_resolved_rows": 1,
            "resolved_corpus_rows": corpus.len(),
            "k": run.k,
            "loaded_edges": run.edge_count,
            "why_this_is_sufficient": "One query plus three resolved candidates with k=2 proves top-k selection, nearest ordering, exclusion of a farther candidate, Graph CF persistence, and readback.",
            "why_smaller_is_insufficient": "With fewer than three candidates, k=2 cannot distinguish selected-neighbor persistence from all-corpus fanout.",
            "why_larger_is_wasteful": "The behavior is deterministic direct cosine top-k plus Graph CF persistence; larger corpora repeat the same code path without adding a new proof obligation."
        },
        "graph_run": serde_json::to_value(&run).expect("run JSON"),
        "graph_cf_readback": graph_readback,
        "edge_cases": edge_cases,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue072_knn_graph_edges_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

fn reopened_graph_readback(vault_dir: &Path, run: &KnnGraphRun) -> Value {
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

fn edge_cases_fail_closed(
    root: &Path,
    ingested: &ResolvedExemplar,
    corpus: &[ResolvedExemplar],
) -> Vec<Value> {
    let empty = compute_knn_edges("crypto", ingested, &[], 1).expect_err("empty corpus");
    assert_eq!(empty.code(), ERR_KNN_GRAPH_EMPTY_CORPUS);

    let small = compute_knn_edges("crypto", ingested, &corpus[..1], 2).expect_err("small corpus");
    assert_eq!(small.code(), ERR_KNN_GRAPH_INVALID_K);

    let dim = compute_knn_edges("crypto", ingested, &[exemplar(20, &[1.0], true)], 1)
        .expect_err("dimension mismatch");
    assert_eq!(dim.code(), ERR_KNN_GRAPH_DIM_MISMATCH);

    let bad_query = exemplar(21, &[f32::NAN, 0.0], true);
    let nonfinite =
        compute_knn_edges("crypto", &bad_query, corpus, 1).expect_err("non-finite vector");
    assert_eq!(nonfinite.code(), ERR_KNN_GRAPH_NON_FINITE);

    let edge_report = json!({
        "empty_corpus": empty.diagnostic(),
        "small_corpus": small.diagnostic(),
        "dimension_mismatch": dim.diagnostic(),
        "non_finite_vector": nonfinite.diagnostic()
    });
    write_json(&root.join("edge-cases.json"), &edge_report);
    vec![
        json!({"case": "empty_corpus", "after": edge_report["empty_corpus"]}),
        json!({"case": "small_corpus", "after": edge_report["small_corpus"]}),
        json!({"case": "dimension_mismatch", "after": edge_report["dimension_mismatch"]}),
        json!({"case": "non_finite_vector", "after": edge_report["non_finite_vector"]}),
    ]
}

fn happy_corpus() -> Vec<ResolvedExemplar> {
    vec![
        exemplar(1, &[1.0, 0.0], true),
        exemplar(2, &[0.8, 0.2], true),
        exemplar(3, &[-1.0, 0.0], false),
    ]
}

fn exemplar(id: u8, vector: &[f32], outcome_yes: bool) -> ResolvedExemplar {
    ResolvedExemplar {
        cx_id: cx(id),
        vector: vector.to_vec(),
        outcome_yes,
    }
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
    .expect("open issue72 vault")
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("vault id")
}

fn issue72_root() -> PathBuf {
    if let Some(path) = std::env::var_os("POLY_ISSUE72_FSV_ROOT") {
        return PathBuf::from(path);
    }
    repo_root().join("target/fsv/issue72_knn_graph_edges_20260707")
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
