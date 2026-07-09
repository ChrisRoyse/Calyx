use std::collections::BTreeMap;
use std::fs;

use calyx_aster::plain_graph::{PhysicalPlainGraph, PlainGraph};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId};
use calyx_mincut::{betweenness, eigenvector_centrality};
use calyx_paths::{AssocGraph, reach_scored};
use serde_json::json;
use sha2::{Digest, Sha256};

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

#[test]
fn persisted_csr_carries_normalized_evidence_weights_into_graph_scores() {
    let dir = temp_dir("weighted-csr");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"issue1213", VaultOptions::default()).unwrap();
    let graph = PlainGraph::new(&vault, "issue1213").unwrap();
    for id in [cx(1), cx(2), cx(3), cx(4)] {
        graph.put_node(id, b"{}").unwrap();
    }
    graph
        .put_edge(cx(1), "assoc", cx(2), br#"{"weight":10,"evidence":"high"}"#)
        .unwrap();
    graph
        .put_edge(cx(2), "assoc", cx(4), br#"{"weight":10}"#)
        .unwrap();
    graph
        .put_edge(cx(1), "assoc", cx(3), br#"{"weight":1,"evidence":"low"}"#)
        .unwrap();
    graph
        .put_edge(cx(3), "assoc", cx(4), br#"{"weight":10}"#)
        .unwrap();

    let commit = graph.rebuild_csr(vault.latest_seq()).unwrap();
    vault.flush().unwrap();
    drop(graph);
    drop(vault);

    let physical = PhysicalPlainGraph::open_latest(&dir, "issue1213").unwrap();
    let raw_csr = physical
        .read_csr_bytes()
        .unwrap()
        .expect("persisted CSR bytes");
    let csr = physical.read_csr().unwrap().expect("decoded CSR");
    let assoc = physical.assoc_graph().unwrap();
    let graph_weights = graph_weights(&assoc);
    let high_first = graph_weights[&(cx(1), cx(2))];
    let low_first = graph_weights[&(cx(1), cx(3))];
    let high_mid_score = score_for(&assoc, cx(1), cx(2));
    let low_mid_score = score_for(&assoc, cx(1), cx(3));
    let weighted_between = betweenness(&assoc).unwrap();
    let spectral = eigenvector_centrality(&assoc, 128, 1e-5).unwrap();
    let spectral_map = spectral.iter().copied().collect::<BTreeMap<_, _>>();

    assert_eq!(commit.projection.edges.len(), 4);
    assert_eq!(csr.edges.len(), 4);
    assert!((high_first - 1.0).abs() < 1e-6);
    assert!((low_first - 0.1).abs() < 1e-6);
    assert!(high_mid_score > low_mid_score);
    assert!(weighted_between[&cx(2)] > weighted_between[&cx(3)]);
    assert!(spectral_map[&cx(2)] > spectral_map[&cx(3)]);

    let edge_cases = edge_case_codes();
    assert_eq!(edge_cases["empty_value"], json!("CALYX_GRAPH_CORRUPT_ROW"));
    assert_eq!(edge_cases["zero_weight"], json!("CALYX_GRAPH_CORRUPT_ROW"));
    assert_eq!(
        edge_cases["malformed_weight"],
        json!("CALYX_GRAPH_CORRUPT_ROW")
    );

    maybe_write_fsv(json!({
        "source_of_truth": "physical Aster Graph CF reopened through PhysicalPlainGraph read_csr_bytes/read_csr/assoc_graph",
        "vault_dir": dir,
        "csr_source_snapshot": csr.source_snapshot,
        "csr_sha256": sha256_hex(&raw_csr),
        "csr_edges": csr.edges,
        "assoc_graph_edges": graph_weights.iter().map(|((src, dst), weight)| {
            json!({"src": src, "dst": dst, "weight": weight})
        }).collect::<Vec<_>>(),
        "reach_scored": {
            "high_mid": high_mid_score,
            "low_mid": low_mid_score,
            "high_gt_low": high_mid_score > low_mid_score,
        },
        "betweenness": {
            "high_mid": weighted_between[&cx(2)],
            "low_mid": weighted_between[&cx(3)],
            "high_gt_low": weighted_between[&cx(2)] > weighted_between[&cx(3)],
        },
        "spectral": {
            "high_mid": spectral_map[&cx(2)],
            "low_mid": spectral_map[&cx(3)],
            "high_gt_low": spectral_map[&cx(2)] > spectral_map[&cx(3)],
        },
        "edge_cases": edge_cases,
    }));

    if calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_none() {
        let _ = fs::remove_dir_all(dir);
    }
}

fn graph_weights(graph: &AssocGraph) -> BTreeMap<(CxId, CxId), f32> {
    graph
        .edges()
        .iter()
        .map(|edge| (graph.edge_endpoints(*edge), edge.weight))
        .collect()
}

fn score_for(graph: &AssocGraph, start: CxId, target: CxId) -> f32 {
    reach_scored(graph, start, 2)
        .unwrap()
        .into_iter()
        .find_map(|(id, score)| (id == target).then_some(score))
        .expect("target reach score")
}

fn edge_case_codes() -> serde_json::Value {
    json!({
        "empty_value": edge_case_code("empty", b""),
        "zero_weight": edge_case_code("zero", b"0"),
        "malformed_weight": edge_case_code("malformed", br#"{"support":1}"#),
    })
}

fn edge_case_code(name: &str, edge_value: &[u8]) -> String {
    let vault = AsterVault::new(vault_id(), b"issue1213-edge-case");
    let graph = PlainGraph::new(&vault, name).unwrap();
    graph.put_node(cx(1), b"{}").unwrap();
    graph.put_node(cx(2), b"{}").unwrap();
    graph.put_edge(cx(1), "assoc", cx(2), edge_value).unwrap();
    graph
        .csr_projection(vault.latest_seq())
        .unwrap_err()
        .code
        .to_string()
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx-issue1213-{label}-{}-{unique}",
        std::process::id()
    ))
}

fn maybe_write_fsv(readback: serde_json::Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).unwrap();
    let path = root.join("issue1213_weighted_csr_readback.json");
    let bytes = serde_json::to_vec_pretty(&readback).unwrap();
    fs::write(&path, &bytes).unwrap();
    let stored = fs::read(&path).unwrap();
    assert_eq!(stored, bytes);
    println!("ISSUE1213_WEIGHTED_CSR_READBACK={}", path.display());
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
