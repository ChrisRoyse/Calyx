use super::*;
use crate::cf::ColumnFamily;
use crate::plain_graph::key::ID_BYTES;
use crate::vault::VaultOptions;
use calyx_core::{CxId, VaultId};
use serde_json::Value;

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; ID_BYTES])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn durable_graph(name: &str) -> (std::path::PathBuf, AsterVault) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "calyx-plain-graph-csr-{name}-{}-{unique}",
        std::process::id()
    ));
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default()).unwrap();
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    graph.put_node(cx(1), b"{}").unwrap();
    graph.put_node(cx(2), b"{}").unwrap();
    graph.put_edge(cx(1), "assoc", cx(2), b"1").unwrap();
    graph.rebuild_csr(vault.latest_seq()).unwrap();
    vault.flush().unwrap();
    (dir, vault)
}

#[test]
fn physical_csr_reader_rejects_tampered_segment_hash() {
    let (dir, vault) = durable_graph("tampered-segment");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    let segment_key = graph.keys.csr_segment_key(0);
    let mut segment = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Graph, &segment_key)
        .unwrap()
        .expect("CSR segment");
    segment[0] ^= 0xff;
    vault
        .write_cf(ColumnFamily::Graph, segment_key, segment)
        .unwrap();
    vault.flush().unwrap();

    let physical = PhysicalPlainGraph::open_latest(&dir, "plain").unwrap();
    let err = physical.read_csr().unwrap_err();
    assert_eq!(err.code, "CALYX_GRAPH_CORRUPT_ROW");
    assert!(err.message.contains("persisted CSR stream hash"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn physical_csr_reader_rejects_manifest_count_mismatch() {
    let (dir, vault) = durable_graph("manifest-mismatch");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    let manifest_key = graph.keys.csr_key();
    let mut manifest: Value = serde_json::from_slice(
        &vault
            .read_cf_at(vault.latest_seq(), ColumnFamily::Graph, &manifest_key)
            .unwrap()
            .expect("CSR manifest"),
    )
    .unwrap();
    manifest["node_count"] = Value::from(999_u64);
    vault
        .write_cf(
            ColumnFamily::Graph,
            manifest_key,
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    vault.flush().unwrap();

    let physical = PhysicalPlainGraph::open_latest(&dir, "plain").unwrap();
    let err = physical.read_csr().unwrap_err();
    assert_eq!(err.code, "CALYX_GRAPH_CORRUPT_ROW");
    assert!(err.message.contains("decode disagrees with manifest"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn physical_csr_reader_rejects_truncated_segment() {
    let (dir, vault) = durable_graph("truncated-segment");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    let segment_key = graph.keys.csr_segment_key(0);
    let mut segment = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Graph, &segment_key)
        .unwrap()
        .expect("CSR segment");
    segment.pop();
    vault
        .write_cf(ColumnFamily::Graph, segment_key, segment)
        .unwrap();
    vault.flush().unwrap();

    let physical = PhysicalPlainGraph::open_latest(&dir, "plain").unwrap();
    let err = physical.read_csr().unwrap_err();
    assert_eq!(err.code, "CALYX_GRAPH_CORRUPT_ROW");
    assert!(err.message.contains("manifest declares"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn physical_csr_reader_rejects_unsupported_manifest_version() {
    let (dir, vault) = durable_graph("unsupported-version");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    let manifest_key = graph.keys.csr_key();
    let mut manifest: Value = serde_json::from_slice(
        &vault
            .read_cf_at(vault.latest_seq(), ColumnFamily::Graph, &manifest_key)
            .unwrap()
            .expect("CSR manifest"),
    )
    .unwrap();
    manifest["csr_manifest_version"] = Value::from(3_u64);
    vault
        .write_cf(
            ColumnFamily::Graph,
            manifest_key,
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    vault.flush().unwrap();

    let physical = PhysicalPlainGraph::open_latest(&dir, "plain").unwrap();
    let err = physical.read_csr().unwrap_err();
    assert_eq!(err.code, "CALYX_GRAPH_CORRUPT_ROW");
    assert!(
        err.message
            .contains("persisted CSR manifest version 3 is not supported")
    );
    let _ = std::fs::remove_dir_all(dir);
}
