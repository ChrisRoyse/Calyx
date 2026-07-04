use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::{PhysicalPlainGraph, PlainGraph};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId};
use calyx_lodestar::DEFAULT_ASTER_ASSOC_COLLECTION;
use serde_json::Value;
use ulid::Ulid;

use super::{MaterializeGraphCsrArgs, run_materialize_graph_csr_with_home};
use crate::cmd::vault::vault_salt;

fn temp_home(name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!(
        "calyx-graph-csr-{name}-{}-{}",
        std::process::id(),
        crate::cmd::vault::now_ms()
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("create temp home");
    home
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn graph_vault(home: &Path, name: &str, edge_pairs: &[(u8, u8)]) -> PathBuf {
    graph_vault_with_options(home, name, edge_pairs, VaultOptions::default())
}

fn graph_vault_with_options(
    home: &Path,
    name: &str,
    edge_pairs: &[(u8, u8)],
    options: VaultOptions,
) -> PathBuf {
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = home.join("vaults").join(vault_id.to_string());
    let vault = AsterVault::new_durable(&path, vault_id, vault_salt(vault_id, name), options)
        .expect("create graph vault");
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION).expect("plain graph");
    let mut nodes = std::collections::BTreeSet::new();
    for (src, dst) in edge_pairs {
        nodes.insert(*src);
        nodes.insert(*dst);
    }
    for node in &nodes {
        graph.put_node(cx(*node), b"{}").expect("put node");
    }
    for (src, dst) in edge_pairs {
        graph
            .put_edge(cx(*src), "assoc", cx(*dst), b"1")
            .expect("put edge");
    }
    vault.flush().expect("flush graph vault");
    path
}

#[test]
fn materialize_persists_csr_and_physical_reader_uses_it() {
    let home = temp_home("happy");
    let vault_path = graph_vault(&home, "happy", &[(1, 2), (2, 3), (1, 3)]);

    // Before: no persisted CSR — physical reader would row-scan.
    let physical =
        PhysicalPlainGraph::open_latest(&vault_path, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    let before = physical.read_csr_bytes().unwrap();
    println!("graph_csr_before_bytes={:?}", before.as_ref().map(Vec::len));
    assert!(before.is_none(), "CSR must not pre-exist");
    drop(physical);

    run_materialize_graph_csr_with_home(
        &home,
        MaterializeGraphCsrArgs {
            vault: vault_path.display().to_string(),
            collection: DEFAULT_ASTER_ASSOC_COLLECTION.to_string(),
        },
    )
    .expect("materialize CSR");

    // After: source of truth — the physical Graph CF has the CSR row and the
    // assoc graph loads from it with the exact node/edge counts.
    let physical =
        PhysicalPlainGraph::open_latest(&vault_path, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    let bytes = physical
        .read_csr_bytes()
        .unwrap()
        .expect("CSR row persisted");
    println!("graph_csr_after_bytes={}", bytes.len());
    let csr = physical.read_csr().unwrap().expect("decode CSR");
    assert_eq!(csr.nodes.len(), 3);
    assert_eq!(csr.edges.len(), 3);
    let graph = physical.assoc_graph().unwrap();
    assert_eq!(graph.node_count(), 3);
    assert_eq!(graph.edge_count(), csr.association_edge_count);
    std::fs::remove_dir_all(home).ok();
}

/// Regression for the real-vault #996 FSV failure: graph rows checkpointed
/// through router-flush SSTs (forced here with a tiny memtable cap) were
/// invisible to the default full-MVCC-restore open, so the projection
/// committed 0 edges while the physical rows still existed. The command must
/// now read the router-latest state and cross-check physical key counts.
#[test]
fn materialize_sees_router_flushed_rows_after_reopen() {
    let home = temp_home("router-flush");
    let vault_path = graph_vault_with_options(
        &home,
        "router-flush",
        &[(1, 2), (2, 3), (3, 4), (1, 4)],
        VaultOptions {
            // Small enough that every graph row forces a router memtable
            // flush into Router SSTs, large enough to accept each row.
            memtable_byte_cap: 256,
            ..VaultOptions::default()
        },
    );
    let physical =
        PhysicalPlainGraph::open_latest(&vault_path, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    let physical_edges = physical.edge_out_key_count().unwrap();
    let physical_nodes = physical.node_key_count().unwrap();
    println!(
        "graph_csr_router_flush physical_nodes={physical_nodes} physical_edges={physical_edges}"
    );
    assert_eq!(physical_nodes, 4);
    assert_eq!(physical_edges, 4);
    drop(physical);

    run_materialize_graph_csr_with_home(
        &home,
        MaterializeGraphCsrArgs {
            vault: vault_path.display().to_string(),
            collection: DEFAULT_ASTER_ASSOC_COLLECTION.to_string(),
        },
    )
    .expect("materialize CSR over router-flushed rows");

    let physical =
        PhysicalPlainGraph::open_latest(&vault_path, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    let csr = physical.read_csr().unwrap().expect("decode CSR");
    assert_eq!(csr.nodes.len(), 4, "all router-flushed nodes in CSR");
    assert_eq!(csr.edges.len(), 4, "all router-flushed edges in CSR");
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn materialize_rebuilds_unsupported_prior_csr_version() {
    let home = temp_home("old-csr-version");
    let vault_id = VaultId::from_ulid(Ulid::new());
    let vault_path = home.join("vaults").join(vault_id.to_string());
    let vault = AsterVault::new_durable(
        &vault_path,
        vault_id,
        vault_salt(vault_id, "old-csr-version"),
        VaultOptions::default(),
    )
    .expect("create graph vault");
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION).expect("plain graph");
    for id in [cx(1), cx(2)] {
        graph.put_node(id, b"{}").unwrap();
    }
    graph.put_edge(cx(1), "assoc", cx(2), b"1").unwrap();
    let commit = graph.rebuild_csr(vault.latest_seq()).unwrap();
    let mut manifest: Value = serde_json::from_slice(
        &vault
            .read_cf_at(commit.seq, ColumnFamily::Graph, &commit.key)
            .unwrap()
            .expect("CSR manifest row"),
    )
    .unwrap();
    manifest["csr_manifest_version"] = Value::from(3_u64);
    vault
        .write_cf(
            ColumnFamily::Graph,
            commit.key.clone(),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    vault.flush().unwrap();
    drop(graph);
    drop(vault);

    let physical =
        PhysicalPlainGraph::open_latest(&vault_path, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    let stale = physical.read_csr_bytes().unwrap_err();
    assert!(
        stale
            .message
            .contains("persisted CSR manifest version 3 is not supported")
    );
    drop(physical);

    run_materialize_graph_csr_with_home(
        &home,
        MaterializeGraphCsrArgs {
            vault: vault_path.display().to_string(),
            collection: DEFAULT_ASTER_ASSOC_COLLECTION.to_string(),
        },
    )
    .expect("materialize over unsupported prior CSR");
    let physical =
        PhysicalPlainGraph::open_latest(&vault_path, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    let raw = physical
        .read_csr_bytes()
        .unwrap()
        .expect("rebuilt CSR bytes");
    assert_eq!(&raw[..8], b"CALYXCSR");
    let csr = physical.read_csr().unwrap().expect("rebuilt CSR");
    assert_eq!(csr.edges.len(), 1);
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn materialize_upgrades_legacy_unweighted_edge_values() {
    let home = temp_home("legacy-unweighted");
    let vault_id = VaultId::from_ulid(Ulid::new());
    let vault_path = home.join("vaults").join(vault_id.to_string());
    let vault = AsterVault::new_durable(
        &vault_path,
        vault_id,
        vault_salt(vault_id, "legacy-unweighted"),
        VaultOptions::default(),
    )
    .expect("create graph vault");
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION).expect("plain graph");
    for id in [cx(1), cx(2), cx(3)] {
        graph.put_node(id, b"{}").unwrap();
    }
    graph
        .put_edge(cx(1), "assoc", cx(2), b"legacy-bytes")
        .unwrap();
    graph
        .put_edge(cx(2), "assoc", cx(3), br#"{"support":1}"#)
        .unwrap();
    assert!(graph.csr_projection(vault.latest_seq()).is_err());
    vault.flush().unwrap();
    drop(graph);
    drop(vault);

    run_materialize_graph_csr_with_home(
        &home,
        MaterializeGraphCsrArgs {
            vault: vault_path.display().to_string(),
            collection: DEFAULT_ASTER_ASSOC_COLLECTION.to_string(),
        },
    )
    .expect("materialize legacy unweighted CSR");
    let physical =
        PhysicalPlainGraph::open_latest(&vault_path, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    let raw = physical
        .read_csr_bytes()
        .unwrap()
        .expect("rebuilt CSR bytes");
    assert_eq!(&raw[..8], b"CALYXCSR");
    let csr = physical.read_csr().unwrap().expect("rebuilt CSR");
    assert_eq!(csr.edges.len(), 2);
    assert!(
        csr.edges
            .iter()
            .all(|edge| (edge.weight - 1.0).abs() < f32::EPSILON)
    );
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn empty_graph_materializes_empty_csr() {
    let home = temp_home("empty");
    let vault_path = graph_vault(&home, "empty", &[]);
    run_materialize_graph_csr_with_home(
        &home,
        MaterializeGraphCsrArgs {
            vault: vault_path.display().to_string(),
            collection: DEFAULT_ASTER_ASSOC_COLLECTION.to_string(),
        },
    )
    .expect("materialize empty CSR");
    let physical =
        PhysicalPlainGraph::open_latest(&vault_path, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    let csr = physical.read_csr().unwrap().expect("decode empty CSR");
    println!(
        "graph_csr_empty nodes={} edges={}",
        csr.nodes.len(),
        csr.edges.len()
    );
    assert!(csr.nodes.is_empty());
    assert!(csr.edges.is_empty());
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn missing_vault_fails_closed_without_creating_state() {
    let home = temp_home("missing");
    let missing = home.join("vaults").join("does-not-exist");
    let err = run_materialize_graph_csr_with_home(
        &home,
        MaterializeGraphCsrArgs {
            vault: missing.display().to_string(),
            collection: DEFAULT_ASTER_ASSOC_COLLECTION.to_string(),
        },
    )
    .unwrap_err();
    println!("graph_csr_missing_vault_code={}", err.code());
    assert_eq!(err.code(), "CALYX_VAULT_ACCESS_DENIED");
    assert!(!missing.exists(), "failed run must not create vault state");
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn unknown_collection_yields_empty_projection_not_default_data() {
    let home = temp_home("collection");
    let vault_path = graph_vault(&home, "collection", &[(1, 2)]);
    run_materialize_graph_csr_with_home(
        &home,
        MaterializeGraphCsrArgs {
            vault: vault_path.display().to_string(),
            collection: "other-collection".to_string(),
        },
    )
    .expect("materialize other collection");
    let physical = PhysicalPlainGraph::open_latest(&vault_path, "other-collection").unwrap();
    let csr = physical.read_csr().unwrap().expect("decode CSR");
    assert!(csr.nodes.is_empty(), "collections must be isolated");
    // Default collection remains CSR-less: this command only touches the
    // collection it was asked for.
    let default_physical =
        PhysicalPlainGraph::open_latest(&vault_path, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    assert!(default_physical.read_csr_bytes().unwrap().is_none());
    std::fs::remove_dir_all(home).ok();
}
