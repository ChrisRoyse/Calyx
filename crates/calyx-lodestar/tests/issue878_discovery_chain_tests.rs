use calyx_core::CxId;
use calyx_lodestar::{
    DiscoveryChainParams, DiscoveryTermination, LodestarError, run_grounded_discovery_chain,
};
use calyx_paths::AssocGraph;
use serde_json::json;
use std::fs;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue878-discovery-chain")
}

#[test]
fn grounded_chain_logs_passes_refusals_and_selected_path() {
    let start = id("start");
    let a = id("a");
    let b = id("b");
    let anchor = id("anchor");
    let ungrounded = id("ungrounded");
    let mut builder = AssocGraph::builder();
    for node in [start, a, b, anchor, ungrounded] {
        builder.add_node(node, 1.0).unwrap();
    }
    builder.add_edge(start, ungrounded, 0.99).unwrap();
    builder.add_edge(start, a, 0.90).unwrap();
    builder.add_edge(a, b, 0.80).unwrap();
    builder.add_edge(a, start, 0.70).unwrap();
    builder.add_edge(b, anchor, 0.70).unwrap();
    let graph = builder.build();
    let params = DiscoveryChainParams {
        max_hops: 4,
        branch_width: 1,
        probe_width: 4,
        max_groundedness_distance: 3,
        min_gate_confidence: 0.25,
        novelty_weight: 0.35,
    };

    let log = run_grounded_discovery_chain(&graph, &[start], &[anchor], &params).unwrap();

    assert_eq!(log.terminated, DiscoveryTermination::FrontierExhausted);
    assert_eq!(log.accepted_hops.len(), 3);
    assert_eq!(log.accepted_hops[0].to, a);
    assert_eq!(log.accepted_hops[1].to, b);
    assert_eq!(log.accepted_hops[2].to, anchor);
    assert_eq!(log.accepted_hops[2].path, vec![start, a, b, anchor]);
    assert!(log.gate_pass_count >= 3);
    assert!(log.refused_count >= 2);
    assert!(log.candidates.iter().any(|row| {
        !row.gate.passed
            && row.candidate.to == ungrounded
            && row.gate.code == "CALYX_DISCOVERY_UNGROUNDED"
    }));
    assert!(
        log.candidates
            .iter()
            .any(|row| { !row.gate.passed && row.gate.code == "CALYX_DISCOVERY_VISITED_LOOP" })
    );
}

#[test]
fn branch_width_keeps_top_passed_candidates() {
    let start = id("branch-start");
    let strong = id("strong");
    let weak = id("weak");
    let anchor = id("branch-anchor");
    let mut builder = AssocGraph::builder();
    for node in [start, strong, weak, anchor] {
        builder.add_node(node, 1.0).unwrap();
    }
    builder.add_edge(start, weak, 0.40).unwrap();
    builder.add_edge(start, strong, 0.95).unwrap();
    builder.add_edge(strong, anchor, 0.90).unwrap();
    builder.add_edge(weak, anchor, 0.90).unwrap();
    let graph = builder.build();
    let params = DiscoveryChainParams {
        max_hops: 1,
        branch_width: 1,
        probe_width: 4,
        max_groundedness_distance: 2,
        min_gate_confidence: 0.25,
        novelty_weight: 0.35,
    };

    let log = run_grounded_discovery_chain(&graph, &[start], &[anchor], &params).unwrap();

    assert_eq!(log.accepted_hops.len(), 1);
    assert_eq!(log.accepted_hops[0].to, strong);
    assert!(
        log.candidates
            .iter()
            .any(|row| row.candidate.to == weak && row.gate.passed && !row.selected)
    );
}

#[test]
fn invalid_params_fail_closed() {
    let start = id("invalid-start");
    let anchor = id("invalid-anchor");
    let mut builder = AssocGraph::builder();
    builder.add_node(start, 1.0).unwrap();
    builder.add_node(anchor, 1.0).unwrap();
    let graph = builder.build();
    let params = DiscoveryChainParams {
        branch_width: 0,
        ..DiscoveryChainParams::default()
    };

    let err = run_grounded_discovery_chain(&graph, &[start], &[anchor], &params).unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
    assert!(matches!(err, LodestarError::KernelInvalidParams { .. }));
}

#[test]
fn unknown_start_fails_closed_as_graph_error() {
    let start = id("known-start");
    let missing = id("missing-start");
    let mut builder = AssocGraph::builder();
    builder.add_node(start, 1.0).unwrap();
    let graph = builder.build();

    let err =
        run_grounded_discovery_chain(&graph, &[missing], &[], &DiscoveryChainParams::default())
            .unwrap_err();

    assert_eq!(err.code(), "CALYX_GRAPH_UNKNOWN_NODE");
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let start = id("fsv-start");
    let mid = id("fsv-mid");
    let anchor = id("fsv-anchor");
    let stray = id("fsv-stray");
    let mut builder = AssocGraph::builder();
    for node in [start, mid, anchor, stray] {
        builder.add_node(node, 1.0).unwrap();
    }
    builder.add_edge(start, mid, 0.90).unwrap();
    builder.add_edge(start, stray, 0.99).unwrap();
    builder.add_edge(mid, anchor, 0.80).unwrap();
    let graph = builder.build();
    let params = DiscoveryChainParams {
        max_hops: 3,
        branch_width: 1,
        probe_width: 4,
        max_groundedness_distance: 2,
        min_gate_confidence: 0.25,
        novelty_weight: 0.35,
    };
    let log = run_grounded_discovery_chain(&graph, &[start], &[anchor], &params).unwrap();
    let value = json!({
        "issue": 878,
        "schema_version": log.schema_version,
        "accepted_count": log.accepted_hops.len(),
        "gate_pass_count": log.gate_pass_count,
        "refused_count": log.refused_count,
        "termination": log.terminated,
        "accepted_to": log.accepted_hops.iter().map(|hop| hop.to.to_string()).collect::<Vec<_>>(),
        "refusal_codes": log.candidates.iter()
            .filter(|row| !row.gate.passed)
            .map(|row| row.gate.code.clone())
            .collect::<Vec<_>>(),
        "full_log": log,
    });
    fs::create_dir_all(&root).unwrap();
    let path = std::path::Path::new(&root).join("issue878_discovery_chain_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["accepted_count"], 2);
    assert_eq!(readback["refused_count"], 1);
    assert_eq!(readback["termination"], "frontier_exhausted");
    println!("issue878_fsv_path={} bytes={}", path.display(), bytes.len());
}
