use std::fs;

use calyx_core::CxId;
use calyx_lodestar::{
    DomainBridgeGateVerdict, DomainBridgeInput, DomainBridgeParams, DomainPair, rank_domain_bridges,
};
use calyx_paths::AssocGraph;
use serde_json::json;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue876-domain-bridges")
}

#[test]
fn ranks_bridge_terms_by_frequency_degree_centrality_and_grounding() {
    let graph = graph();
    let inputs = vec![
        input("clinical", "molecular", "bridge-low", 0.40, true, 0.80),
        input("clinical", "molecular", "bridge-top", 0.95, true, 0.90),
        input("clinical", "molecular", "bridge-refused", 0.99, false, 0.90),
    ];

    let report = rank_domain_bridges(&graph, &inputs, &DomainBridgeParams::default()).unwrap();

    assert_eq!(report.input_count, 3);
    assert_eq!(report.pair_reports.len(), 1);
    assert_eq!(report.pair_reports[0].candidate_count, 2);
    assert_eq!(report.pair_reports[0].refused_count, 1);
    assert_eq!(report.pair_reports[0].candidates[0].cx_id, id("bridge-top"));
    assert!(report.pair_reports[0].candidates[0].degree >= 2);
}

#[test]
fn groups_multiple_domain_pairs() {
    let graph = graph();
    let inputs = vec![
        input("clinical", "molecular", "bridge-top", 0.80, true, 0.90),
        input("clinical", "legal", "bridge-low", 0.80, true, 0.90),
    ];

    let report = rank_domain_bridges(&graph, &inputs, &DomainBridgeParams::default()).unwrap();

    assert_eq!(report.pair_reports.len(), 2);
    assert!(
        report
            .pair_reports
            .iter()
            .all(|pair| pair.candidate_count == 1)
    );
}

#[test]
fn max_per_pair_truncates_after_ranking() {
    let graph = graph();
    let inputs = vec![
        input("clinical", "molecular", "bridge-low", 0.40, true, 0.80),
        input("clinical", "molecular", "bridge-top", 0.95, true, 0.90),
    ];
    let params = DomainBridgeParams {
        max_per_pair: 1,
        ..DomainBridgeParams::default()
    };

    let report = rank_domain_bridges(&graph, &inputs, &params).unwrap();

    assert_eq!(report.pair_reports[0].candidate_count, 1);
    assert_eq!(report.pair_reports[0].candidates[0].cx_id, id("bridge-top"));
}

#[test]
fn invalid_input_fails_closed() {
    let graph = graph();
    let mut inputs = vec![input(
        "clinical",
        "molecular",
        "bridge-top",
        f32::NAN,
        true,
        0.90,
    )];
    let err = rank_domain_bridges(&graph, &inputs, &DomainBridgeParams::default()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    inputs[0] = input("clinical", "molecular", "missing", 0.50, true, 0.90);
    let err = rank_domain_bridges(&graph, &inputs, &DomainBridgeParams::default()).unwrap_err();
    assert_eq!(err.code(), "CALYX_GRAPH_UNKNOWN_NODE");
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let graph = graph();
    let inputs = vec![
        input("clinical", "molecular", "bridge-low", 0.40, true, 0.80),
        input("clinical", "molecular", "bridge-top", 0.95, true, 0.90),
        input("clinical", "molecular", "bridge-refused", 0.99, false, 0.90),
        input("clinical", "legal", "bridge-low", 0.70, true, 0.80),
    ];
    let report = rank_domain_bridges(&graph, &inputs, &DomainBridgeParams::default()).unwrap();
    let total_candidates: usize = report
        .pair_reports
        .iter()
        .map(|pair| pair.candidate_count)
        .sum();
    let total_refused: usize = report
        .pair_reports
        .iter()
        .map(|pair| pair.refused_count)
        .sum();
    let top = &report.pair_reports[0].candidates[0];
    let value = json!({
        "issue": 876,
        "schema_version": report.schema_version,
        "input_count": report.input_count,
        "pair_count": report.pair_reports.len(),
        "candidate_count": total_candidates,
        "refused_count": total_refused,
        "top_cx_id": top.cx_id.to_string(),
        "top_rank_score": top.rank_score,
        "top_degree": top.degree,
        "full_report": report,
    });
    fs::create_dir_all(&root).unwrap();
    let path = std::path::Path::new(&root).join("issue876_domain_bridges_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["pair_count"], 2);
    assert_eq!(readback["candidate_count"], 3);
    assert_eq!(readback["refused_count"], 1);
    println!("issue876_fsv_path={} bytes={}", path.display(), bytes.len());
}

fn graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for (label, weight) in [
        ("bridge-low", 1.0),
        ("bridge-top", 5.0),
        ("bridge-refused", 3.0),
        ("left-a", 1.0),
        ("right-c", 1.0),
        ("extra", 1.0),
    ] {
        builder.add_node(id(label), weight).unwrap();
    }
    for (src, dst, weight) in [
        ("left-a", "bridge-top", 0.9),
        ("bridge-top", "right-c", 0.9),
        ("bridge-top", "extra", 0.7),
        ("bridge-low", "right-c", 0.5),
        ("left-a", "bridge-refused", 0.8),
    ] {
        builder.add_edge(id(src), id(dst), weight).unwrap();
    }
    builder.build()
}

fn input(
    left: &str,
    right: &str,
    label: &str,
    centrality_score: f32,
    passed: bool,
    confidence: f32,
) -> DomainBridgeInput {
    DomainBridgeInput {
        pair: DomainPair {
            left: left.to_string(),
            right: right.to_string(),
        },
        cx_id: id(label),
        text: format!("candidate B-term text for {label}"),
        centrality_score,
        cross_domain_distance: Some(3),
        gate: DomainBridgeGateVerdict {
            passed,
            confidence,
            code: if passed {
                "CALYX_DOMAIN_BRIDGE_GATE_PASS".to_string()
            } else {
                "CALYX_DOMAIN_BRIDGE_GATE_REFUSED".to_string()
            },
            reason: "synthetic bridge gate".to_string(),
            evidence: vec!["synthetic sufficiency evidence".to_string()],
        },
        provenance: vec![format!("synthetic provenance for {label}")],
    }
}
