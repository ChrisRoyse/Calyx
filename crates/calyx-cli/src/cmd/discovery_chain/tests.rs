use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId};
use calyx_lodestar::{
    AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION, DiscoveryTermination,
    encode_assoc_node_props,
};

use super::*;
use crate::cmd::vault::vault_salt;

fn toks(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn parse(parts: &[&str]) -> CliResult<DiscoveryChainArgs> {
    match super::parse_discovery_chain(&toks(parts))? {
        Subcommand::DiscoveryChain(args) => Ok(args),
        _ => unreachable!("parse_discovery_chain must return DiscoveryChain"),
    }
}

#[test]
fn parses_required_ids_and_tuning_flags() {
    let args = parse(&[
        "corpus",
        "--start",
        &cx(1).to_string(),
        "--anchor",
        &cx(4).to_string(),
        "--max-hops",
        "12",
        "--branch-width",
        "2",
        "--probe-width",
        "8",
        "--max-groundedness-distance",
        "5",
        "--min-gate-confidence",
        "0.30",
        "--novelty-weight",
        "0.40",
    ])
    .unwrap();

    assert_eq!(args.vault, "corpus");
    assert_eq!(args.starts, vec![cx(1)]);
    assert_eq!(args.anchors, vec![cx(4)]);
    assert!(args.anchor_files.is_empty());
    assert_eq!(args.max_hops, 12);
    assert_eq!(args.branch_width, 2);
    assert_eq!(args.probe_width, 8);
    assert_eq!(args.max_groundedness_distance, 5);
    assert_eq!(args.min_gate_confidence, 0.30);
    assert_eq!(args.novelty_weight, 0.40);
}

#[test]
fn missing_anchor_fails_closed() {
    let err = parse(&["corpus", "--start", &cx(1).to_string()]).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("at least one --anchor"));
}

#[test]
fn parses_anchor_file() {
    let args = parse(&[
        "corpus",
        "--start",
        &cx(1).to_string(),
        "--anchor-file",
        "anchors.txt",
    ])
    .unwrap();

    assert_eq!(args.anchors, Vec::<CxId>::new());
    assert_eq!(args.anchor_files, vec![PathBuf::from("anchors.txt")]);
}

#[test]
fn invalid_start_id_fails_closed() {
    let err = parse(&[
        "corpus",
        "--start",
        "not-a-cxid",
        "--anchor",
        "00000000000000000000000000000004",
    ])
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("parse --start"));
}

#[test]
fn run_persists_chain_then_reads_back_source_of_truth() {
    let (home, vault_dir) = seed_home("happy", SeedShape::GroundedChain);
    let anchor_file = home.join("anchors.txt");
    fs::write(&anchor_file, format!("{}\n", cx(4))).unwrap();

    run_discovery_chain_with_home(
        &home,
        DiscoveryChainArgs {
            vault: "happy".to_string(),
            starts: vec![cx(1)],
            anchors: Vec::new(),
            anchor_files: vec![anchor_file],
            max_hops: 4,
            branch_width: 1,
            probe_width: 4,
            max_groundedness_distance: 3,
            min_gate_confidence: 0.25,
            novelty_weight: 0.35,
            out: None,
        },
    )
    .unwrap();

    let chain_path = only_chain(&vault_dir);
    let readback_bytes = fs::read(&chain_path).unwrap();
    let artifact: DiscoveryChainArtifact = serde_json::from_slice(&readback_bytes).unwrap();

    assert_eq!(artifact.schema_version, 1);
    assert_eq!(artifact.graph_node_count, 5);
    assert_eq!(artifact.graph_edge_count, 5);
    assert_eq!(artifact.log.schema_version, 1);
    assert_eq!(artifact.log.anchors, vec![cx(4)]);
    assert_eq!(
        artifact.log.terminated,
        DiscoveryTermination::FrontierExhausted
    );
    assert_eq!(artifact.log.accepted_hops.len(), 3);
    assert_eq!(artifact.log.accepted_hops[0].to, cx(2));
    assert_eq!(artifact.log.accepted_hops[1].to, cx(3));
    assert_eq!(artifact.log.accepted_hops[2].to, cx(4));
    assert_eq!(
        artifact.node_metadata[&cx(4)]
            .get("term")
            .map(String::as_str),
        Some("clinical anchor")
    );
    assert!(
        artifact.log.candidates.iter().any(|row| {
            row.candidate.to == cx(9) && row.gate.code == "CALYX_DISCOVERY_UNGROUNDED"
        })
    );
}

#[test]
fn strict_gate_refuses_before_artifact_write() {
    let (home, vault_dir) = seed_home("strict", SeedShape::GroundedChain);

    let err = run_discovery_chain_with_home(
        &home,
        DiscoveryChainArgs {
            vault: "strict".to_string(),
            starts: vec![cx(1)],
            anchors: vec![cx(4)],
            anchor_files: Vec::new(),
            max_hops: 4,
            branch_width: 1,
            probe_width: 4,
            max_groundedness_distance: 3,
            min_gate_confidence: 0.99,
            novelty_weight: 0.35,
            out: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
    assert!(err.message().contains("no accepted gate-PASS hops"));
    assert!(!vault_dir.join("idx").join("discovery_chains").exists());
}

#[test]
fn unknown_start_fails_before_artifact_write() {
    let (home, vault_dir) = seed_home("missing", SeedShape::GroundedChain);

    let err = run_discovery_chain_with_home(
        &home,
        DiscoveryChainArgs {
            vault: "missing".to_string(),
            starts: vec![cx(7)],
            anchors: vec![cx(4)],
            anchor_files: Vec::new(),
            max_hops: 4,
            branch_width: 1,
            probe_width: 4,
            max_groundedness_distance: 3,
            min_gate_confidence: 0.25,
            novelty_weight: 0.35,
            out: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_GRAPH_UNKNOWN_NODE");
    assert!(!vault_dir.join("idx").join("discovery_chains").exists());
}

#[derive(Clone, Copy)]
enum SeedShape {
    GroundedChain,
}

fn seed_home(name: &str, _shape: SeedShape) -> (PathBuf, PathBuf) {
    let home = std::env::temp_dir().join(format!(
        "calyx-discovery-chain-{name}-{}",
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
    for (seed, term) in [
        (1, "clinical start"),
        (2, "candidate a"),
        (3, "candidate b"),
        (4, "clinical anchor"),
        (9, "ungrounded distractor"),
    ] {
        put_node(&graph, seed, term);
    }
    for (src, dst) in [(1, 9), (1, 2), (2, 3), (2, 1), (3, 4)] {
        graph.put_edge(cx(src), "assoc", cx(dst), b"1").unwrap();
    }
    vault.flush().unwrap();
    (home, vault_dir)
}

fn put_node<C: calyx_core::Clock>(graph: &PlainGraph<'_, C>, seed: u8, term: &str) {
    let props = AsterAssocNodeProps {
        metadata: BTreeMap::from([
            ("term".to_string(), term.to_string()),
            ("source_id".to_string(), format!("issue878-row-{seed}")),
        ]),
        ..Default::default()
    };
    graph
        .put_node(cx(seed), &encode_assoc_node_props(&props).unwrap())
        .unwrap();
}

fn only_chain(vault_dir: &Path) -> PathBuf {
    let root = vault_dir.join("idx").join("discovery_chains");
    let dirs = fs::read_dir(&root)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(dirs.len(), 1);
    dirs[0].path().join("chain.json")
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
