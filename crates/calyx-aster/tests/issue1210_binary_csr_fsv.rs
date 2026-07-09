use std::fs;

use calyx_aster::plain_graph::{PhysicalPlainGraph, PlainGraph, PlainGraphCsr, PlainGraphCsrEdge};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId};
use serde_json::json;
use sha2::{Digest, Sha256};

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn node(index: usize) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&(index as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

#[test]
fn binary_csr_physical_readback_is_smaller_than_json() {
    let dir = temp_dir("binary-csr");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"issue1210", VaultOptions::default()).unwrap();
    let graph = PlainGraph::new(&vault, "issue1210").unwrap();
    let projection = repeated_type_projection(96, 20_000);
    let json_len = serde_json::to_vec(&projection).unwrap().len();
    let commit = graph.write_csr_projection(projection.clone()).unwrap();
    vault.flush().unwrap();
    drop(graph);
    drop(vault);

    let physical = PhysicalPlainGraph::open_latest(&dir, "issue1210").unwrap();
    let raw_csr = physical
        .read_csr_bytes()
        .unwrap()
        .expect("persisted binary CSR bytes");
    let decoded = physical.read_csr().unwrap().expect("decoded CSR");
    assert_eq!(decoded, projection);
    assert_eq!(&raw_csr[..8], b"CALYXCSR");
    assert!(
        raw_csr.len() * 2 < json_len,
        "binary CSR bytes={} JSON bytes={json_len}",
        raw_csr.len()
    );

    let edge_cases = edge_case_readbacks();
    assert!(edge_cases["empty_roundtrip"].as_bool().unwrap());
    assert!(edge_cases["self_loop_roundtrip"].as_bool().unwrap());
    assert_eq!(edge_cases["large_type_table_count"], json!(70_000));

    maybe_write_fsv(json!({
        "source_of_truth": "physical Aster Graph CF reopened through PhysicalPlainGraph read_csr_bytes/read_csr",
        "vault_dir": dir,
        "write_seq": commit.seq,
        "csr_source_snapshot": decoded.source_snapshot,
        "binary_magic": String::from_utf8_lossy(&raw_csr[..8]),
        "binary_csr_sha256": sha256_hex(&raw_csr),
        "binary_csr_bytes": raw_csr.len(),
        "json_baseline_bytes": json_len,
        "binary_less_than_half_json": raw_csr.len() * 2 < json_len,
        "node_count": decoded.nodes.len(),
        "edge_count": decoded.edges.len(),
        "association_edge_count": decoded.association_edge_count,
        "edge_cases": edge_cases,
    }));

    if calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_none() {
        let _ = fs::remove_dir_all(dir);
    }
}

fn edge_case_readbacks() -> serde_json::Value {
    let empty = PlainGraphCsr {
        collection: "issue1210".to_string(),
        source_snapshot: 1,
        nodes: Vec::new(),
        offsets: vec![0],
        edges: Vec::new(),
        association_edge_count: 0,
    };
    let self_loop = PlainGraphCsr {
        collection: "issue1210".to_string(),
        source_snapshot: 2,
        nodes: vec![cx(1)],
        offsets: vec![0, 1],
        edges: vec![PlainGraphCsrEdge {
            dst: cx(1),
            edge_type: "self".to_string(),
            weight: 1.0,
        }],
        association_edge_count: 1,
    };
    let large_type_count = 70_000;
    let large_type_table = PlainGraphCsr {
        collection: "issue1210".to_string(),
        source_snapshot: 3,
        nodes: vec![cx(1)],
        offsets: vec![0, large_type_count],
        edges: (0..large_type_count)
            .map(|index| PlainGraphCsrEdge {
                dst: cx(1),
                edge_type: format!("t{index:05}"),
                weight: 1.0,
            })
            .collect(),
        association_edge_count: large_type_count,
    };
    json!({
        "empty_roundtrip": roundtrip_projection(empty),
        "self_loop_roundtrip": roundtrip_projection(self_loop),
        "large_type_table_roundtrip": roundtrip_projection(large_type_table),
        "large_type_table_count": large_type_count,
    })
}

fn roundtrip_projection(projection: PlainGraphCsr) -> bool {
    let vault = AsterVault::new(vault_id(), b"issue1210-edge-case");
    let graph = PlainGraph::new(&vault, &projection.collection).unwrap();
    let commit = graph.write_csr_projection(projection.clone()).unwrap();
    graph.read_csr(commit.seq).unwrap() == Some(projection)
}

fn repeated_type_projection(node_count: usize, edge_count: usize) -> PlainGraphCsr {
    let nodes = (0..node_count).map(node).collect::<Vec<_>>();
    let mut offsets = Vec::with_capacity(node_count + 1);
    let mut edges = Vec::with_capacity(edge_count);
    for src_index in 0..node_count {
        offsets.push(edges.len());
        while edges.len() < edge_count && edges.len() < (src_index + 1) * (edge_count / node_count)
        {
            let edge_index = edges.len();
            edges.push(PlainGraphCsrEdge {
                dst: nodes[(src_index + edge_index + 1) % node_count],
                edge_type: if edge_index % 3 == 0 {
                    "associated_with".to_string()
                } else if edge_index % 3 == 1 {
                    "targets".to_string()
                } else {
                    "literature_supports".to_string()
                },
                weight: 1.0,
            });
        }
    }
    offsets.push(edges.len());
    PlainGraphCsr {
        collection: "issue1210".to_string(),
        source_snapshot: 12_100,
        nodes,
        offsets,
        association_edge_count: edges.len(),
        edges,
    }
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx-issue1210-{label}-{}-{unique}",
        std::process::id()
    ))
}

fn maybe_write_fsv(readback: serde_json::Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).unwrap();
    let path = root.join("issue1210_binary_csr_readback.json");
    let bytes = serde_json::to_vec_pretty(&readback).unwrap();
    fs::write(&path, &bytes).unwrap();
    let stored = fs::read(&path).unwrap();
    assert_eq!(stored, bytes);
    println!("ISSUE1210_BINARY_CSR_READBACK={}", path.display());
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
