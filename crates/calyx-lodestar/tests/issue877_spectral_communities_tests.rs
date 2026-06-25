use std::fs;

use calyx_core::CxId;
use calyx_lodestar::{SpectralCommunityParams, spectral_community_report};
use calyx_paths::AssocGraph;
use serde_json::json;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue877-spectral-communities")
}

#[test]
fn partitions_planted_graph_and_ranks_inter_community_bridge() {
    let graph = graph();
    let report = spectral_community_report(&graph, &SpectralCommunityParams::default()).unwrap();

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.node_count, 6);
    assert_eq!(report.edge_count, 13);
    assert_eq!(report.communities.len(), 2);
    assert!(report.spectral_gap > 0.0);
    assert!(
        report
            .communities
            .iter()
            .all(|entry| entry.member_count == 3)
    );
    assert!(!report.bridge_candidates.is_empty());
    assert!(!report.centrality_candidates.is_empty());

    let top = &report.bridge_candidates[0];
    assert_eq!(top.src, id("a-bridge"));
    assert_eq!(top.dst, id("b-bridge"));
    assert_ne!(top.src_community, top.dst_community);
    assert!(top.rank_score > 0.0);
    assert!(
        report
            .centrality_candidates
            .iter()
            .any(|candidate| candidate.cx_id == id("a-bridge")
                || candidate.cx_id == id("b-bridge"))
    );
}

#[test]
fn candidate_limits_truncate_after_ranking() {
    let params = SpectralCommunityParams {
        max_bridge_candidates: 1,
        max_centrality_candidates: 2,
        ..SpectralCommunityParams::default()
    };
    let report = spectral_community_report(&graph(), &params).unwrap();

    assert_eq!(report.bridge_candidates.len(), 1);
    assert_eq!(report.bridge_candidates[0].src, id("a-bridge"));
    assert_eq!(report.bridge_candidates[0].dst, id("b-bridge"));
    assert_eq!(report.centrality_candidates.len(), 2);
}

#[test]
fn invalid_params_and_too_small_graph_fail_closed() {
    let mut invalid = SpectralCommunityParams {
        eigen_k: 1,
        ..SpectralCommunityParams::default()
    };
    let err = spectral_community_report(&graph(), &invalid).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    invalid = SpectralCommunityParams {
        max_bridge_candidates: 0,
        ..SpectralCommunityParams::default()
    };
    let err = spectral_community_report(&graph(), &invalid).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut builder = AssocGraph::builder();
    builder.add_node(id("solo"), 1.0).unwrap();
    let err = spectral_community_report(&builder.build(), &SpectralCommunityParams::default())
        .unwrap_err();
    assert_eq!(err.code(), "CALYX_SPECTRAL_GRAPH_TOO_SMALL");
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let report = spectral_community_report(&graph(), &SpectralCommunityParams::default()).unwrap();
    let top = &report.bridge_candidates[0];
    let value = json!({
        "issue": 877,
        "schema_version": report.schema_version,
        "node_count": report.node_count,
        "edge_count": report.edge_count,
        "community_count": report.communities.len(),
        "bridge_candidate_count": report.bridge_candidates.len(),
        "centrality_candidate_count": report.centrality_candidates.len(),
        "spectral_gap": report.spectral_gap,
        "top_bridge_src": top.src.to_string(),
        "top_bridge_dst": top.dst.to_string(),
        "top_bridge_rank_score": top.rank_score,
        "full_report": report,
    });
    fs::create_dir_all(&root).unwrap();
    let path = std::path::Path::new(&root).join("issue877_spectral_communities_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["community_count"], 2);
    assert_eq!(readback["top_bridge_src"], id("a-bridge").to_string());
    assert_eq!(readback["top_bridge_dst"], id("b-bridge").to_string());
    println!("issue877_fsv_path={} bytes={}", path.display(), bytes.len());
}

fn graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for (label, weight) in [
        ("a-left", 1.0),
        ("a-bridge", 5.0),
        ("a-right", 1.0),
        ("b-left", 1.0),
        ("b-bridge", 5.0),
        ("b-right", 1.0),
    ] {
        builder.add_node(id(label), weight).unwrap();
    }
    for (left, right) in [
        ("a-left", "a-bridge"),
        ("a-bridge", "a-right"),
        ("a-left", "a-right"),
        ("b-left", "b-bridge"),
        ("b-bridge", "b-right"),
        ("b-left", "b-right"),
    ] {
        add_undirected(&mut builder, left, right, 0.95);
    }
    builder
        .add_edge(id("a-bridge"), id("b-bridge"), 0.90)
        .unwrap();
    builder.build()
}

fn add_undirected(
    builder: &mut calyx_paths::AssocGraphBuilder,
    left: &str,
    right: &str,
    weight: f32,
) {
    builder.add_edge(id(left), id(right), weight).unwrap();
    builder.add_edge(id(right), id(left), weight).unwrap();
}
