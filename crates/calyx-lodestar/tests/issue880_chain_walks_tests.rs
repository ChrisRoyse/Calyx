use std::fs;

use calyx_core::CxId;
use calyx_lodestar::{
    ChainWalkParams, ChainWalkSeed, ChainWalkSeedKind, DiscoveryChainParams,
    run_grounded_chain_walks,
};
use calyx_paths::AssocGraph;
use serde_json::json;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue880-chain-walks")
}

#[test]
fn runs_static_and_operator_seeds_into_terminal_abc_hypotheses() {
    let (graph, anchors) = graph();
    let seeds = seeds();
    let report = run_grounded_chain_walks(&graph, &seeds, &anchors, &params()).unwrap();

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.seed_count, 2);
    assert_eq!(report.completed_chain_count, 2);
    assert_eq!(report.hypothesis_count, 2);

    let static_hypothesis = &report.results[0].hypotheses[0];
    assert_eq!(static_hypothesis.seed_id, "static-top");
    assert_eq!(
        static_hypothesis.seed_kind,
        ChainWalkSeedKind::StaticCandidate
    );
    assert_eq!(static_hypothesis.a, id("static-start"));
    assert_eq!(static_hypothesis.b, id("static-bridge"));
    assert_eq!(static_hypothesis.c, id("static-anchor"));
    assert_eq!(static_hypothesis.cross_domain_distance, 2);
    assert!(static_hypothesis.terminal_confidence >= 1.0);
    assert!(
        static_hypothesis
            .provenance
            .iter()
            .any(|row| row == "sweep_rank=1")
    );

    let operator_hypothesis = &report.results[1].hypotheses[0];
    assert_eq!(
        operator_hypothesis.seed_kind,
        ChainWalkSeedKind::OperatorQuestion
    );
    assert_eq!(operator_hypothesis.a, id("operator-start"));
    assert_eq!(operator_hypothesis.b, id("operator-bridge"));
    assert_eq!(operator_hypothesis.c, id("operator-anchor"));
    assert_eq!(anchors.len(), 2);
}

#[test]
fn hypothesis_limit_truncates_after_ranking() {
    let mut params = params();
    params.max_hypotheses_per_seed = 1;
    let seed = ChainWalkSeed {
        seed_id: "long-chain".to_string(),
        kind: ChainWalkSeedKind::StaticCandidate,
        start: id("long-start"),
        question: None,
        rationale: "synthetic long chain".to_string(),
        provenance: vec!["long_chain_fixture".to_string()],
    };
    let (graph, anchors) = long_graph();

    let report = run_grounded_chain_walks(&graph, &[seed], &anchors, &params).unwrap();

    assert_eq!(report.hypothesis_count, 1);
    assert_eq!(report.results[0].hypotheses.len(), 1);
}

#[test]
fn invalid_seeds_fail_closed() {
    let (graph, anchors) = graph();
    let err = run_grounded_chain_walks(&graph, &[], &anchors, &params()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut duplicate = seeds();
    duplicate[1].seed_id = duplicate[0].seed_id.clone();
    let err = run_grounded_chain_walks(&graph, &duplicate, &anchors, &params()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut missing_question = seeds();
    missing_question[1].question = None;
    let err = run_grounded_chain_walks(&graph, &missing_question, &anchors, &params()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut missing_start = seeds();
    missing_start[0].start = id("missing-start");
    let err = run_grounded_chain_walks(&graph, &missing_start, &anchors, &params()).unwrap_err();
    assert_eq!(err.code(), "CALYX_GRAPH_UNKNOWN_NODE");
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let (graph, anchors) = graph();
    let report = run_grounded_chain_walks(&graph, &seeds(), &anchors, &params()).unwrap();
    let top = &report.results[0].hypotheses[0];
    let value = json!({
        "issue": 880,
        "schema_version": report.schema_version,
        "seed_count": report.seed_count,
        "completed_chain_count": report.completed_chain_count,
        "hypothesis_count": report.hypothesis_count,
        "top_seed_id": top.seed_id,
        "top_a": top.a.to_string(),
        "top_b": top.b.to_string(),
        "top_c": top.c.to_string(),
        "top_rank_score": top.rank_score,
        "top_cross_domain_distance": top.cross_domain_distance,
        "full_report": report,
    });
    fs::create_dir_all(&root).unwrap();
    let path = std::path::Path::new(&root).join("issue880_chain_walks_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["seed_count"], 2);
    assert_eq!(readback["completed_chain_count"], 2);
    assert_eq!(readback["hypothesis_count"], 2);
    println!("issue880_fsv_path={} bytes={}", path.display(), bytes.len());
}

fn graph() -> (AssocGraph, Vec<CxId>) {
    let mut builder = AssocGraph::builder();
    for label in [
        "static-start",
        "static-bridge",
        "static-anchor",
        "static-stray",
        "operator-start",
        "operator-bridge",
        "operator-anchor",
    ] {
        builder.add_node(id(label), 1.0).unwrap();
    }
    builder
        .add_edge(id("static-start"), id("static-stray"), 0.99)
        .unwrap();
    builder
        .add_edge(id("static-start"), id("static-bridge"), 0.90)
        .unwrap();
    builder
        .add_edge(id("static-bridge"), id("static-anchor"), 0.80)
        .unwrap();
    builder
        .add_edge(id("operator-start"), id("operator-bridge"), 0.88)
        .unwrap();
    builder
        .add_edge(id("operator-bridge"), id("operator-anchor"), 0.82)
        .unwrap();
    (
        builder.build(),
        vec![id("static-anchor"), id("operator-anchor")],
    )
}

fn long_graph() -> (AssocGraph, Vec<CxId>) {
    let mut builder = AssocGraph::builder();
    for label in ["long-start", "long-bridge", "long-mid", "long-anchor"] {
        builder.add_node(id(label), 1.0).unwrap();
    }
    builder
        .add_edge(id("long-start"), id("long-bridge"), 0.90)
        .unwrap();
    builder
        .add_edge(id("long-bridge"), id("long-mid"), 0.88)
        .unwrap();
    builder
        .add_edge(id("long-mid"), id("long-anchor"), 0.86)
        .unwrap();
    (builder.build(), vec![id("long-anchor")])
}

fn seeds() -> Vec<ChainWalkSeed> {
    vec![
        ChainWalkSeed {
            seed_id: "static-top".to_string(),
            kind: ChainWalkSeedKind::StaticCandidate,
            start: id("static-start"),
            question: None,
            rationale: "highest novelty gate-passing static sweep candidate".to_string(),
            provenance: vec!["sweep_rank=1".to_string()],
        },
        ChainWalkSeed {
            seed_id: "operator-q1".to_string(),
            kind: ChainWalkSeedKind::OperatorQuestion,
            start: id("operator-start"),
            question: Some("operator supplied disease/target seed".to_string()),
            rationale: "operator question seed".to_string(),
            provenance: vec!["operator_question=q1".to_string()],
        },
    ]
}

fn params() -> ChainWalkParams {
    ChainWalkParams {
        chain: DiscoveryChainParams {
            max_hops: 4,
            branch_width: 1,
            probe_width: 4,
            max_groundedness_distance: 2,
            min_gate_confidence: 0.25,
            novelty_weight: 0.35,
        },
        max_hypotheses_per_seed: 8,
        min_terminal_confidence: 0.25,
    }
}
