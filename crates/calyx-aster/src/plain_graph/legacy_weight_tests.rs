use super::*;
use crate::plain_graph::key::ID_BYTES;
use crate::vault::AsterVault;
use calyx_core::{CxId, VaultId};

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; ID_BYTES])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

#[test]
fn legacy_unweighted_edges_require_explicit_upgrade_policy_for_csr() {
    let vault = AsterVault::new(vault_id(), b"salt");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    for id in [cx(1), cx(2), cx(3)] {
        graph.put_node(id, b"{}").unwrap();
    }
    graph
        .put_edge(cx(1), "knows", cx(2), br#"{"support":1}"#)
        .unwrap();
    graph
        .put_edge(cx(2), "knows", cx(3), b"legacy-bytes")
        .unwrap();

    let strict = graph.csr_projection(vault.latest_seq()).unwrap_err();
    assert_eq!(strict.code, "CALYX_GRAPH_CORRUPT_ROW");
    assert!(
        strict
            .message
            .contains("graph edge value must carry a positive numeric evidence weight")
    );

    let (commit, stats) = graph
        .rebuild_csr_with_legacy_unit_weights(vault.latest_seq())
        .unwrap();
    assert_eq!(stats.explicit_weight_edges, 0);
    assert_eq!(stats.legacy_unit_weight_edges, 2);
    assert_eq!(commit.projection.edges.len(), 2);
    assert!(
        commit
            .projection
            .edges
            .iter()
            .all(|edge| (edge.weight - 1.0).abs() < f32::EPSILON)
    );

    graph
        .put_edge(cx(3), "knows", cx(1), br#"{"weight":0}"#)
        .unwrap();
    let invalid = graph
        .rebuild_csr_with_legacy_unit_weights(vault.latest_seq())
        .unwrap_err();
    assert_eq!(invalid.code, "CALYX_GRAPH_CORRUPT_ROW");
    assert!(
        invalid
            .message
            .contains("graph edge evidence weight must be finite and > 0")
    );
}
