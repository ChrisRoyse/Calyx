use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId};
use calyx_lodestar::{
    AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION, PANEL_ASTER_ASSOC_COLLECTION,
    SpectralCommunityReport, encode_assoc_node_props,
};

use super::*;
use crate::cmd::vault::vault_salt;

fn toks(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn parse(parts: &[&str]) -> CliResult<SpectralCommunitiesArgs> {
    match super::parse_spectral_communities(&toks(parts))? {
        Subcommand::SpectralCommunities(args) => Ok(args),
        _ => unreachable!("parse_spectral_communities must return SpectralCommunities"),
    }
}

#[test]
fn parses_tuning_flags() {
    let args = parse(&[
        "corpus",
        "--eigen-k",
        "4",
        "--eigen-max-iter",
        "48",
        "--communities",
        "3",
        "--cluster-max-iter",
        "72",
        "--centrality-max-iter",
        "96",
        "--centrality-tol",
        "0.0001",
        "--max-bridge-candidates",
        "7",
        "--max-centrality-candidates",
        "9",
    ])
    .unwrap();

    assert_eq!(args.vault, "corpus");
    assert_eq!(args.eigen_k, 4);
    assert_eq!(args.eigen_max_iter, 48);
    assert_eq!(args.community_count, 3);
    assert_eq!(args.cluster_max_iter, 72);
    assert_eq!(args.centrality_max_iter, 96);
    assert_eq!(args.centrality_tol, 0.0001);
    assert_eq!(args.max_bridge_candidates, 7);
    assert_eq!(args.max_centrality_candidates, 9);
}

#[test]
fn invalid_tolerance_fails_closed() {
    let err = parse(&["corpus", "--centrality-tol", "0"]).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("greater than 0"));
}

#[test]
fn run_persists_report_then_reads_back_source_of_truth() {
    let (home, vault_dir) = seed_home("happy", SeedShape::TwoCommunitiesWithBridge);

    run_spectral_communities_with_home(
        &home,
        SpectralCommunitiesArgs {
            vault: "happy".to_string(),
            eigen_k: 3,
            community_count: 2,
            max_bridge_candidates: 4,
            max_centrality_candidates: 6,
            ..SpectralCommunitiesArgs::default()
        },
    )
    .unwrap();

    let report_path = only_report(&vault_dir);
    let readback_bytes = fs::read(&report_path).unwrap();
    let report: SpectralCommunityReport = serde_json::from_slice(&readback_bytes).unwrap();

    assert_eq!(report.schema_version, 3);
    assert_eq!(report.node_count, 6);
    assert_eq!(report.edge_count, 13);
    assert_eq!(report.communities.len(), 2);
    assert_eq!(report.requested_communities, 2);
    assert_eq!(report.embedding_dimensions, 2);
    assert_eq!(report.degenerate_node_count, 0);
    assert_eq!(
        report.assignment_method,
        "normalized-laplacian-row-l2-farthest-first-lloyd-v3-disconnected-components"
    );
    assert_eq!(report.bridge_candidates.len(), 1);
    assert_eq!(report.centrality_candidates.len(), 6);
    assert_eq!(report.bridge_candidates[0].src, cx(2));
    assert_eq!(report.bridge_candidates[0].dst, cx(5));
}

/// #1973 end-to-end: a vault whose panel-native graph has isolated opinions
/// must no longer fail closed. The CLI persists a report that covers every node
/// (`members == node_count`), records `degenerate_node_count`, and adds one
/// singleton community per isolated node on top of the spectral partition.
#[test]
fn isolated_nodes_persist_covered_report_with_degenerate_provenance() {
    let (home, vault_dir) = seed_home("isolated", SeedShape::TwoCommunitiesWithIsolated);

    run_spectral_communities_with_home(
        &home,
        SpectralCommunitiesArgs {
            vault: "isolated".to_string(),
            eigen_k: 3,
            community_count: 2,
            max_bridge_candidates: 4,
            max_centrality_candidates: 6,
            ..SpectralCommunitiesArgs::default()
        },
    )
    .expect("isolated nodes must be assigned, not fail closed");

    let report: SpectralCommunityReport =
        serde_json::from_slice(&fs::read(only_report(&vault_dir)).unwrap()).unwrap();

    assert_eq!(report.node_count, 9);
    // Total coverage: one membership row per node, no drops.
    assert_eq!(report.members.len(), 9);
    assert_eq!(report.degenerate_node_count, 3);
    // 2 spectral + 3 singleton communities; isolated nodes ordered by cx_id
    // (seeds 7,8,9 -> communities 2,3,4).
    assert_eq!(report.communities.len(), 5);
    let sizes: Vec<usize> = report.communities.iter().map(|c| c.member_count).collect();
    assert_eq!(sizes, vec![3, 3, 1, 1, 1]);
    for (seed, expected) in [(7_u8, 2_u8), (8, 3), (9, 4)] {
        let member = report.members.iter().find(|m| m.cx_id == cx(seed)).unwrap();
        assert_eq!(member.community, expected);
        assert_eq!(member.degree, 0);
    }
}

#[test]
fn explicit_existing_report_fails_before_mutation_and_preserves_bytes() {
    let (home, _) = seed_home("existing-report", SeedShape::TwoCommunitiesWithBridge);
    let report_path = home.join("immutable-report.json");
    fs::write(&report_path, b"physically-existing-evidence\n").unwrap();
    let before = fs::read(&report_path).unwrap();

    let error = run_spectral_communities_with_home(
        &home,
        SpectralCommunitiesArgs {
            vault: "existing-report".to_string(),
            eigen_k: 3,
            community_count: 2,
            out: Some(report_path.clone()),
            ..SpectralCommunitiesArgs::default()
        },
    )
    .expect_err("existing explicit evidence must fail closed");

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.message().contains("evidence is immutable"));
    assert_eq!(fs::read(&report_path).unwrap(), before);
}

#[test]
fn too_small_graph_fails_before_artifact_write() {
    let (home, vault_dir) = seed_home("too-small", SeedShape::SingleNode);

    let err = run_spectral_communities_with_home(
        &home,
        SpectralCommunitiesArgs {
            vault: "too-small".to_string(),
            eigen_k: 3,
            community_count: 2,
            ..SpectralCommunitiesArgs::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_SPECTRAL_GRAPH_TOO_SMALL");
    assert!(!vault_dir.join("idx").join("spectral_communities").exists());
}

#[test]
fn disconnected_graph_fails_before_artifact_write() {
    let (home, vault_dir) = seed_home("no-bridge", SeedShape::TwoCommunitiesNoBridge);

    let err = run_spectral_communities_with_home(
        &home,
        SpectralCommunitiesArgs {
            vault: "no-bridge".to_string(),
            eigen_k: 3,
            community_count: 2,
            ..SpectralCommunitiesArgs::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
    assert!(
        err.message()
            .contains("no inter-community bridge candidates")
    );
    assert!(!vault_dir.join("idx").join("spectral_communities").exists());
}

#[test]
fn parses_collection_flag_defaulting_to_panel_native() {
    let default = parse(&["corpus"]).unwrap();
    assert_eq!(default.collection, PANEL_ASTER_ASSOC_COLLECTION);

    let overridden = parse(&["corpus", "--collection", "default"]).unwrap();
    assert_eq!(overridden.collection, "default");

    let empty = parse(&["corpus", "--collection", ""]).unwrap_err();
    assert_eq!(empty.code(), "CALYX_CLI_USAGE_ERROR");
}

/// #1971 regression: the panel-native migration (#1857) moved the woven assoc
/// graph out of the `default` collection into `kernel-panel-v3`. Reading the
/// default assoc collection (the pre-fix behaviour) now finds nothing, so
/// spectral-communities must (a) read `kernel-panel-v3` by default and (b) fail
/// closed with a specific code — never a silent-empty / misleading "too small".
#[test]
fn reads_panel_native_collection_by_default() {
    let (home, vault_dir) = seed_home_in(
        "panel-native",
        SeedShape::TwoCommunitiesWithBridge,
        PANEL_ASTER_ASSOC_COLLECTION,
    );

    run_spectral_communities_with_home(
        &home,
        SpectralCommunitiesArgs {
            vault: "panel-native".to_string(),
            eigen_k: 3,
            community_count: 2,
            max_bridge_candidates: 4,
            max_centrality_candidates: 6,
            ..SpectralCommunitiesArgs::default()
        },
    )
    .expect("panel-native assoc graph must be readable by default");

    let report: SpectralCommunityReport =
        serde_json::from_slice(&fs::read(only_report(&vault_dir)).unwrap()).unwrap();
    assert_eq!(report.node_count, 6);
    assert_eq!(report.communities.len(), 2);
}

#[test]
fn missing_panel_collection_fails_closed_with_specific_code() {
    // Graph only lives in the legacy `default` collection; the default run
    // reads `kernel-panel-v3`, which is empty. This is the exact #1971 miss.
    let (home, vault_dir) = seed_home_in(
        "legacy-only",
        SeedShape::TwoCommunitiesWithBridge,
        DEFAULT_ASTER_ASSOC_COLLECTION,
    );

    let err = run_spectral_communities_with_home(
        &home,
        SpectralCommunitiesArgs {
            vault: "legacy-only".to_string(),
            eigen_k: 3,
            community_count: 2,
            ..SpectralCommunitiesArgs::default()
        },
    )
    .expect_err("empty panel collection must fail closed, not return an empty report");

    assert_eq!(err.code(), "CALYX_SPECTRAL_GRAPH_NOT_PERSISTED");
    assert!(err.message().contains("kernel-panel-v3"));
    assert!(!vault_dir.join("idx").join("spectral_communities").exists());
}

#[test]
fn legacy_default_collection_readable_via_flag() {
    // Backward compatibility: pre-#1857 vaults still work with --collection default.
    let (home, vault_dir) = seed_home_in(
        "legacy-flag",
        SeedShape::TwoCommunitiesWithBridge,
        DEFAULT_ASTER_ASSOC_COLLECTION,
    );

    run_spectral_communities_with_home(
        &home,
        SpectralCommunitiesArgs {
            vault: "legacy-flag".to_string(),
            collection: DEFAULT_ASTER_ASSOC_COLLECTION.to_string(),
            eigen_k: 3,
            community_count: 2,
            max_bridge_candidates: 4,
            max_centrality_candidates: 6,
            ..SpectralCommunitiesArgs::default()
        },
    )
    .expect("legacy default collection must be readable via --collection default");

    let report: SpectralCommunityReport =
        serde_json::from_slice(&fs::read(only_report(&vault_dir)).unwrap()).unwrap();
    assert_eq!(report.node_count, 6);
}

#[derive(Clone, Copy)]
enum SeedShape {
    TwoCommunitiesWithBridge,
    TwoCommunitiesNoBridge,
    /// The bridged two-community core plus three degree-0 opinions (seeds 7,8,9)
    /// that model the isolated nodes #1857's edge threshold leaves behind (#1973).
    TwoCommunitiesWithIsolated,
    SingleNode,
}

fn seed_home(name: &str, shape: SeedShape) -> (PathBuf, PathBuf) {
    seed_home_in(name, shape, PANEL_ASTER_ASSOC_COLLECTION)
}

fn seed_home_in(name: &str, shape: SeedShape, collection: &str) -> (PathBuf, PathBuf) {
    let home = std::env::temp_dir().join(format!(
        "calyx-spectral-communities-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join("vaults")).unwrap();
    let vault_id = vault_id();
    let vault_dir = home.join("vaults").join(vault_id.to_string());
    fs::write(
        home.join("vaults").join("index.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "vaults": [{
                "name": name,
                "vault_id": vault_id.to_string(),
                "path": format!("vaults/{vault_id}"),
                "panel_template": "text-default"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions::default(),
    )
    .unwrap();
    let graph = PlainGraph::new(&vault, collection).unwrap();
    match shape {
        SeedShape::SingleNode => put_node(&graph, 1, "solo"),
        SeedShape::TwoCommunitiesWithBridge
        | SeedShape::TwoCommunitiesNoBridge
        | SeedShape::TwoCommunitiesWithIsolated => {
            for (seed, label) in [
                (1, "a-left"),
                (2, "a-bridge"),
                (3, "a-right"),
                (4, "b-left"),
                (5, "b-bridge"),
                (6, "b-right"),
            ] {
                put_node(&graph, seed, label);
            }
            for (left, right) in [(1, 2), (2, 3), (1, 3), (4, 5), (5, 6), (4, 6)] {
                put_undirected(&graph, left, right);
            }
            if matches!(
                shape,
                SeedShape::TwoCommunitiesWithBridge | SeedShape::TwoCommunitiesWithIsolated
            ) {
                graph.put_edge(cx(2), "assoc", cx(5), b"1").unwrap();
            }
            if matches!(shape, SeedShape::TwoCommunitiesWithIsolated) {
                for (seed, label) in [(7, "iso-1"), (8, "iso-2"), (9, "iso-3")] {
                    put_node(&graph, seed, label);
                }
            }
        }
    }
    vault.flush().unwrap();
    (home, vault_dir)
}

fn put_node<C: calyx_core::Clock>(graph: &PlainGraph<'_, C>, seed: u8, label: &str) {
    let props = AsterAssocNodeProps {
        metadata: BTreeMap::from([("term".to_string(), label.to_string())]),
        ..Default::default()
    };
    graph
        .put_node(cx(seed), &encode_assoc_node_props(&props).unwrap())
        .unwrap();
}

fn put_undirected<C: calyx_core::Clock>(graph: &PlainGraph<'_, C>, left: u8, right: u8) {
    graph.put_edge(cx(left), "assoc", cx(right), b"1").unwrap();
    graph.put_edge(cx(right), "assoc", cx(left), b"1").unwrap();
}

fn only_report(vault_dir: &Path) -> PathBuf {
    let root = vault_dir.join("idx").join("spectral_communities");
    let dirs = fs::read_dir(&root)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(dirs.len(), 1);
    dirs[0].path().join("report.json")
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
