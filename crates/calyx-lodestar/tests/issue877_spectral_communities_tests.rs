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

    assert_eq!(report.schema_version, 3);
    assert_eq!(report.node_count, 6);
    assert_eq!(report.edge_count, 13);
    assert_eq!(report.communities.len(), 2);
    assert_eq!(report.requested_communities, 2);
    assert_eq!(report.embedding_dimensions, 2);
    // Fully connected planted graph: no degenerate nodes, so the v3 method
    // reduces to the frozen v2 numerics (proven by the unchanged partition and
    // bridge ranking below) and produces no extra communities.
    assert_eq!(report.degenerate_node_count, 0);
    assert_eq!(
        report.assignment_method,
        "normalized-laplacian-row-l2-farthest-first-lloyd-v3-disconnected-components"
    );
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

/// #1973: after #1857's `edge_score_threshold` sparsifies the woven graph,
/// some opinions become isolated. A degree-0 node is a decoupled 1x1 block of
/// the normalized Laplacian with eigenvalue 1, so its coordinate is exactly 0
/// in every bottom-k eigenvector (all eigenvalues < 1). The v2 method failed
/// closed on the first such all-zero embedding row; v3 must keep the main
/// spectral partition unchanged and give each disconnected node a deterministic
/// per-component community instead.
#[test]
fn isolated_nodes_become_deterministic_singleton_communities() {
    let isolated = ["iso-charlie", "iso-alpha", "iso-bravo"];
    let graph = graph_with_isolated(&isolated);

    let report = spectral_community_report(&graph, &SpectralCommunityParams::default()).unwrap();

    // Coverage: every node keeps a membership row, none dropped.
    assert_eq!(report.node_count, 9);
    assert_eq!(report.members.len(), 9);
    // Provenance records the 3 disconnected nodes.
    assert_eq!(report.degenerate_node_count, 3);
    assert_eq!(
        report.assignment_method,
        "normalized-laplacian-row-l2-farthest-first-lloyd-v3-disconnected-components"
    );

    // 2 spectral communities of the connected core (unchanged, 3 members each)
    // plus one singleton community per isolated node.
    assert_eq!(report.requested_communities, 2);
    assert_eq!(report.communities.len(), 5);
    let sizes: Vec<usize> = report.communities.iter().map(|c| c.member_count).collect();
    assert_eq!(sizes, vec![3, 3, 1, 1, 1]);
    assert_eq!(
        report
            .communities
            .iter()
            .map(|c| c.community)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );

    // The connected core is exactly communities {0,1}; the isolated nodes are
    // exactly communities {2,3,4}, ordered by cx_id.
    let community_of = |label: &str| {
        report
            .members
            .iter()
            .find(|m| m.cx_id == id(label))
            .unwrap()
            .community
    };
    let mut expected = isolated.to_vec();
    expected.sort_by(|left, right| id(left).as_bytes().cmp(id(right).as_bytes()));
    for (offset, label) in expected.iter().enumerate() {
        assert_eq!(community_of(label), (2 + offset) as u8, "label={label}");
    }
    // Core nodes stay in the two spectral communities.
    for label in [
        "a-left", "a-bridge", "a-right", "b-left", "b-bridge", "b-right",
    ] {
        assert!(
            community_of(label) < 2,
            "core label {label} escaped spectral range"
        );
    }

    // The bridge ranking over the connected core is unchanged by the isolates.
    assert_eq!(report.bridge_candidates[0].src, id("a-bridge"));
    assert_eq!(report.bridge_candidates[0].dst, id("b-bridge"));
}

/// The full report is a pure function of the input graph: identical graph ->
/// byte-identical serialized artifact (frozen-numerics determinism, #1973 req 2).
#[test]
fn identical_graph_yields_byte_identical_report() {
    let isolated = ["iso-charlie", "iso-alpha", "iso-bravo"];
    let first = spectral_community_report(
        &graph_with_isolated(&isolated),
        &SpectralCommunityParams::default(),
    )
    .unwrap();
    let second = spectral_community_report(
        &graph_with_isolated(&isolated),
        &SpectralCommunityParams::default(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
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
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
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
    let path = root.join("issue877_spectral_communities_readback.json");
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

/// The planted two-community core plus `isolated` degree-0 nodes appended after
/// the connected core (so the core keeps node indices `0..6` and its spectral
/// partition is bit-identical to `graph()`).
fn graph_with_isolated(isolated: &[&str]) -> AssocGraph {
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
    for label in isolated {
        builder.add_node(id(label), 1.0).unwrap();
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
