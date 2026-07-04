use super::*;
use crate::plain_graph::key::ID_BYTES;
use calyx_core::{CxId, VaultId};

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; ID_BYTES])
}

fn node(index: usize) -> CxId {
    let mut bytes = [0_u8; ID_BYTES];
    bytes[..8].copy_from_slice(&(index as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

#[test]
fn binary_csr_roundtrips_and_reduces_repeated_edge_type_bytes() {
    let vault = AsterVault::new(vault_id(), b"salt");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    let projection = repeated_type_projection(32, 256);
    let json_len = serde_json::to_vec(&projection).unwrap().len();

    let commit = graph.write_csr_projection(projection.clone()).unwrap();
    let stream = graph
        .read_csr(commit.seq)
        .unwrap()
        .expect("binary CSR decoded");
    let bytes = csr_store::load_csr_bytes(&graph.keys, |key| {
        vault.read_cf_at(commit.seq, ColumnFamily::Graph, key)
    })
    .unwrap()
    .expect("CSR stream bytes");

    assert_eq!(stream, projection);
    assert!(
        bytes.len() * 2 < json_len,
        "binary CSR bytes={} JSON bytes={}",
        bytes.len(),
        json_len
    );
}

#[test]
fn binary_csr_handles_empty_graph_and_single_self_loop() {
    let vault = AsterVault::new(vault_id(), b"salt");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    let empty = PlainGraphCsr {
        collection: "plain".to_string(),
        source_snapshot: 7,
        nodes: Vec::new(),
        offsets: vec![0],
        edges: Vec::new(),
        association_edge_count: 0,
    };
    let empty_commit = graph.write_csr_projection(empty.clone()).unwrap();
    assert_eq!(graph.read_csr(empty_commit.seq).unwrap(), Some(empty));

    let self_loop = PlainGraphCsr {
        collection: "plain".to_string(),
        source_snapshot: 8,
        nodes: vec![cx(1)],
        offsets: vec![0, 1],
        edges: vec![PlainGraphCsrEdge {
            dst: cx(1),
            edge_type: "self".to_string(),
            weight: 1.0,
        }],
        association_edge_count: 1,
    };
    let loop_commit = graph.write_csr_projection(self_loop.clone()).unwrap();
    assert_eq!(graph.read_csr(loop_commit.seq).unwrap(), Some(self_loop));
}

#[test]
fn binary_csr_uses_u32_edge_type_dictionary_indexes() {
    const TYPE_COUNT: usize = 70_000;
    let vault = AsterVault::new(vault_id(), b"salt");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    let projection = PlainGraphCsr {
        collection: "plain".to_string(),
        source_snapshot: 9,
        nodes: vec![cx(1)],
        offsets: vec![0, TYPE_COUNT],
        edges: (0..TYPE_COUNT)
            .map(|index| PlainGraphCsrEdge {
                dst: cx(1),
                edge_type: format!("t{index:05}"),
                weight: 1.0,
            })
            .collect(),
        association_edge_count: TYPE_COUNT,
    };
    let commit = graph.write_csr_projection(projection.clone()).unwrap();
    let readback = graph.read_csr(commit.seq).unwrap().expect("binary CSR");
    assert_eq!(readback, projection);
}

fn repeated_type_projection(node_count: usize, edge_count: usize) -> PlainGraphCsr {
    let nodes = (0..node_count).map(node).collect::<Vec<_>>();
    let mut offsets = Vec::with_capacity(node_count + 1);
    let mut edges = Vec::with_capacity(edge_count);
    for src_index in 0..node_count {
        offsets.push(edges.len());
        let remaining = edge_count.saturating_sub(edges.len());
        let per_node = remaining.min(edge_count / node_count + 1);
        for edge_index in 0..per_node {
            edges.push(PlainGraphCsrEdge {
                dst: nodes[(src_index + edge_index + 1) % node_count],
                edge_type: if edge_index % 2 == 0 {
                    "associated_with".to_string()
                } else {
                    "targets".to_string()
                },
                weight: 1.0,
            });
            if edges.len() == edge_count {
                break;
            }
        }
    }
    while offsets.len() < node_count + 1 {
        offsets.push(edges.len());
    }
    PlainGraphCsr {
        collection: "plain".to_string(),
        source_snapshot: 6,
        nodes,
        offsets,
        association_edge_count: edges.len(),
        edges,
    }
}
