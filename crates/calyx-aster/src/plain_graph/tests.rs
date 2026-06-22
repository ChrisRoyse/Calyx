use super::*;
use crate::plain_graph::key::ID_BYTES;
use crate::vault::VaultOptions;
use calyx_core::{CxId, VaultId};
use serde_json::json;

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; ID_BYTES])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

#[test]
fn edge_and_reverse_index_share_one_sequence() {
    let vault = AsterVault::new(vault_id(), b"salt");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    graph.put_node(cx(1), br#"{"n":"a"}"#).unwrap();
    graph.put_node(cx(2), br#"{"n":"b"}"#).unwrap();
    let commit = graph.put_edge(cx(1), "knows", cx(2), b"ab").unwrap();
    let rows = vault.scan_cf_at(commit.seq, ColumnFamily::Graph).unwrap();
    assert!(
        rows.iter()
            .any(|(key, value)| key == &commit.edge_key && value == b"ab")
    );
    assert!(
        rows.iter()
            .any(|(key, value)| key == &commit.reverse_key && value == &commit.edge_key)
    );
    assert_eq!(
        graph
            .out_neighbors(commit.seq, cx(1), Some("knows"), 4)
            .unwrap()[0]
            .dst,
        cx(2)
    );
    assert_eq!(
        graph
            .in_neighbors(commit.seq, cx(2), Some("knows"), 4)
            .unwrap()[0]
            .src,
        cx(1)
    );
}

#[test]
fn traverse_handles_chain_cycle_unknown_type_and_limits() {
    let vault = AsterVault::new(vault_id(), b"salt");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    for id in [cx(1), cx(2), cx(3), cx(4)] {
        graph.put_node(id, b"{}").unwrap();
    }
    graph.put_edge(cx(1), "knows", cx(2), b"ab").unwrap();
    graph.put_edge(cx(2), "knows", cx(3), b"bc").unwrap();
    graph.put_edge(cx(3), "knows", cx(4), b"cd").unwrap();
    graph.put_edge(cx(3), "knows", cx(1), b"cycle").unwrap();
    let opts = TraverseOptions {
        edge_type: Some("knows"),
        direction: PlainGraphDirection::Out,
        max_hops: 2,
        cost_cap: 16,
    };
    assert_eq!(
        graph.traverse(vault.latest_seq(), cx(1), opts).unwrap(),
        vec![cx(2), cx(3)]
    );
    let mut unknown = opts;
    unknown.edge_type = Some("blocks");
    assert!(
        graph
            .traverse(vault.latest_seq(), cx(1), unknown)
            .unwrap()
            .is_empty()
    );
    let err = graph
        .traverse(
            vault.latest_seq(),
            cx(1),
            TraverseOptions {
                max_hops: MAX_TRAVERSE_HOPS + 1,
                ..opts
            },
        )
        .unwrap_err();
    assert_eq!(err.code, "CALYX_GRAPH_TRAVERSE_LIMIT");
}

#[test]
fn csr_projection_is_rebuildable_and_persisted() {
    let vault = AsterVault::new(vault_id(), b"salt");
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    graph.put_node(cx(1), b"{}").unwrap();
    graph.put_node(cx(2), b"{}").unwrap();
    graph.put_node(cx(3), b"{}").unwrap();
    graph.put_edge(cx(1), "knows", cx(2), b"ab").unwrap();
    graph.put_edge(cx(1), "likes", cx(3), b"ac").unwrap();
    let commit = graph.rebuild_csr(vault.latest_seq()).unwrap();
    assert_eq!(commit.projection.nodes, vec![cx(1), cx(2), cx(3)]);
    assert_eq!(commit.projection.offsets, vec![0, 2, 2, 2]);
    assert_eq!(graph.read_csr(commit.seq).unwrap(), Some(commit.projection));
}

#[test]
fn failed_wal_append_leaves_neither_edge_row() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("calyx-plain-graph-{}-{unique}", std::process::id()));
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default()).unwrap();
    let graph = PlainGraph::new(&vault, "plain").unwrap();
    graph.put_node(cx(1), b"{}").unwrap();
    graph.put_node(cx(2), b"{}").unwrap();
    vault.fail_next_wal_append_for_test();
    let err = graph.put_edge(cx(1), "knows", cx(2), b"ab").unwrap_err();
    assert_eq!(err.code, "CALYX_DISK_PRESSURE");
    assert!(
        graph
            .get_edge(vault.latest_seq(), cx(1), "knows", cx(2))
            .unwrap()
            .is_none()
    );
    let reverse = graph.edge_in_key(cx(2), "knows", cx(1)).unwrap();
    assert!(
        vault
            .read_cf_at(vault.latest_seq(), ColumnFamily::Graph, &reverse)
            .unwrap()
            .is_none()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[ignore = "manual FSV writes plain graph evidence bytes"]
fn issue638_plain_graph_fsv() {
    let root = std::env::var_os("CALYX_ISSUE638_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-issue638-fsv"));
    let vault_dir = root.join("vault.calyx");
    let report_path = root.join("issue638-plain-graph-fsv.json");
    let _ = std::fs::remove_dir_all(&vault_dir);
    let _ = std::fs::remove_file(&report_path);
    std::fs::create_dir_all(&root).unwrap();
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue638-salt",
        VaultOptions::default(),
    )
    .unwrap();
    let graph = PlainGraph::new(&vault, "issue638_plain").unwrap();
    let empty = PlainGraph::new(&vault, "issue638_empty").unwrap();
    let before = raw_graph_rows(&vault);

    for (id, props) in [
        (cx(1), br#"{"name":"a"}"#.as_slice()),
        (cx(2), br#"{"name":"b"}"#.as_slice()),
        (cx(3), br#"{"name":"c"}"#.as_slice()),
        (cx(4), br#"{"name":"d"}"#.as_slice()),
    ] {
        graph.put_node(id, props).unwrap();
    }
    let ab = graph.put_edge(cx(1), "knows", cx(2), b"ab").unwrap();
    let bc = graph.put_edge(cx(2), "knows", cx(3), b"bc").unwrap();
    graph.put_edge(cx(3), "knows", cx(4), b"cd").unwrap();
    graph.put_edge(cx(3), "knows", cx(1), b"cycle").unwrap();
    graph.put_edge(cx(4), "likes", cx(2), b"db").unwrap();
    let snapshot = vault.latest_seq();
    let expected_two_hop = vec![cx(2), cx(3)];
    let actual_two_hop = graph
        .traverse(
            snapshot,
            cx(1),
            TraverseOptions {
                edge_type: Some("knows"),
                direction: PlainGraphDirection::Out,
                max_hops: 2,
                cost_cap: 32,
            },
        )
        .unwrap();
    assert_eq!(actual_two_hop, expected_two_hop);
    let csr_commit = graph.rebuild_csr(snapshot).unwrap();
    let csr_readback = graph.read_csr(csr_commit.seq).unwrap().unwrap();
    assert_eq!(csr_readback, csr_commit.projection);

    let unknown_before = raw_graph_rows(&vault).len();
    let unknown = graph
        .traverse(
            vault.latest_seq(),
            cx(1),
            TraverseOptions {
                edge_type: Some("blocks"),
                direction: PlainGraphDirection::Out,
                max_hops: 2,
                cost_cap: 32,
            },
        )
        .unwrap();
    let unknown_after = raw_graph_rows(&vault).len();
    let max_hop_code = graph
        .traverse(
            vault.latest_seq(),
            cx(1),
            TraverseOptions {
                max_hops: MAX_TRAVERSE_HOPS + 1,
                ..TraverseOptions::default()
            },
        )
        .unwrap_err()
        .code;
    let empty_csr = empty.csr_projection(vault.latest_seq()).unwrap();
    let empty_traverse_code = empty
        .traverse(vault.latest_seq(), cx(9), TraverseOptions::default())
        .unwrap_err()
        .code;

    let da_forward = graph.edge_out_key(cx(4), "knows", cx(1)).unwrap();
    let da_reverse = graph.edge_in_key(cx(1), "knows", cx(4)).unwrap();
    let crash_before = raw_graph_rows(&vault).len();
    vault.fail_next_wal_append_for_test();
    let crash_code = graph
        .put_edge(cx(4), "knows", cx(1), b"da")
        .unwrap_err()
        .code;
    let crash_forward_after = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Graph, &da_forward)
        .unwrap();
    let crash_reverse_after = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Graph, &da_reverse)
        .unwrap();
    let crash_after = raw_graph_rows(&vault).len();
    assert!(crash_forward_after.is_none());
    assert!(crash_reverse_after.is_none());

    vault.flush().unwrap();
    let reopened = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue638-salt",
        VaultOptions::default(),
    )
    .unwrap();
    let report = json!({
        "issue": 638,
        "source_of_truth": {
            "vault_dir": vault_dir,
            "cf": "graph",
            "sst_dir": root.join("vault.calyx/cf/graph"),
            "wal_dir": root.join("vault.calyx/wal")
        },
        "before_graph_rows": before,
        "edge_atomicity": {
            "forward_seq": ab.seq,
            "next_edge_seq": bc.seq,
            "forward_key_hex": hex(&ab.edge_key),
            "forward_value_hex": hex(&graph.get_edge(ab.seq, cx(1), "knows", cx(2)).unwrap().unwrap()),
            "reverse_key_hex": hex(&ab.reverse_key),
            "reverse_value_hex": hex(&vault.read_cf_at(ab.seq, ColumnFamily::Graph, &ab.reverse_key).unwrap().unwrap())
        },
        "traverse_2hop": {
            "start": cx(1),
            "edge_type": "knows",
            "max_hops": 2,
            "expected": expected_two_hop,
            "actual": actual_two_hop
        },
        "edge_cases": {
            "unknown_etype": {
                "before_rows": unknown_before,
                "actual": unknown,
                "after_rows": unknown_after
            },
            "max_hop": {
                "code": max_hop_code
            },
            "empty_graph": {
                "csr_offsets": empty_csr.offsets,
                "node_count": empty_csr.nodes.len(),
                "traverse_code": empty_traverse_code
            },
            "crash_mid_write": {
                "before_rows": crash_before,
                "code": crash_code,
                "forward_after": crash_forward_after,
                "reverse_after": crash_reverse_after,
                "after_rows": crash_after
            }
        },
        "csr": {
            "write_seq": csr_commit.seq,
            "key_hex": hex(&csr_commit.key),
            "projection": csr_readback
        },
        "after_reopen_rows": raw_graph_rows(&reopened),
        "physical_files": physical_files(&root)
    });
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("ISSUE638_FSV_ROOT {}", root.display());
    println!("ISSUE638_FSV_REPORT {}", report_path.display());
}

fn raw_graph_rows<C: calyx_core::Clock>(vault: &AsterVault<C>) -> Vec<serde_json::Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Graph)
        .unwrap()
        .into_iter()
        .map(|(key, value)| json!({"key_hex": hex(&key), "value_hex": hex(&value)}))
        .collect()
}

fn physical_files(root: &std::path::Path) -> Vec<serde_json::Value> {
    let mut files = Vec::new();
    collect_files(&root.join("vault.calyx/cf/graph"), &mut files);
    collect_files(&root.join("vault.calyx/wal"), &mut files);
    files
}

fn collect_files(dir: &std::path::Path, files: &mut Vec<serde_json::Value>) {
    if !dir.exists() {
        return;
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            let bytes = std::fs::read(&path).unwrap();
            files.push(json!({
                "path": path,
                "bytes": bytes.len(),
                "blake3": blake3::hash(&bytes).to_hex().to_string()
            }));
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
