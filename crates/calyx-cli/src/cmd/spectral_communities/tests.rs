use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId};
use calyx_lodestar::{
    AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION, SpectralCommunityReport,
    encode_assoc_node_props,
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
            max_bridge_candidates: 4,
            max_centrality_candidates: 6,
            ..SpectralCommunitiesArgs::default()
        },
    )
    .unwrap();

    let report_path = only_report(&vault_dir);
    let readback_bytes = fs::read(&report_path).unwrap();
    let report: SpectralCommunityReport = serde_json::from_slice(&readback_bytes).unwrap();

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.node_count, 6);
    assert_eq!(report.edge_count, 13);
    assert_eq!(report.communities.len(), 2);
    assert_eq!(report.bridge_candidates.len(), 1);
    assert_eq!(report.centrality_candidates.len(), 6);
    assert_eq!(report.bridge_candidates[0].src, cx(2));
    assert_eq!(report.bridge_candidates[0].dst, cx(5));
}

#[test]
fn too_small_graph_fails_before_artifact_write() {
    let (home, vault_dir) = seed_home("too-small", SeedShape::SingleNode);

    let err = run_spectral_communities_with_home(
        &home,
        SpectralCommunitiesArgs {
            vault: "too-small".to_string(),
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
            ..SpectralCommunitiesArgs::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
    assert!(err.message().contains("fewer than two communities"));
    assert!(!vault_dir.join("idx").join("spectral_communities").exists());
}

#[derive(Clone, Copy)]
enum SeedShape {
    TwoCommunitiesWithBridge,
    TwoCommunitiesNoBridge,
    SingleNode,
}

fn seed_home(name: &str, shape: SeedShape) -> (PathBuf, PathBuf) {
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
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    match shape {
        SeedShape::SingleNode => put_node(&graph, 1, "solo"),
        SeedShape::TwoCommunitiesWithBridge | SeedShape::TwoCommunitiesNoBridge => {
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
            if matches!(shape, SeedShape::TwoCommunitiesWithBridge) {
                graph.put_edge(cx(2), "assoc", cx(5), b"1").unwrap();
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
