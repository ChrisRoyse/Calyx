use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, FixedClock};
use calyx_ledger::{DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore, decode};
use calyx_lodestar::{
    CALYX_PROP_GRAPH_EMPTY, CALYX_PROP_INVALID_INPUT, CALYX_PROP_NO_KERNEL_NODES,
    CALYX_PROP_NOT_CONVERGED, kernel_labels_hash, propagate_labels, propagate_labels_with_ledger,
};
use calyx_paths::AssocGraph;
use proptest::prelude::*;
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

#[test]
fn path_graph_confidence_decays_from_grounded_kernel() {
    let graph = path_graph(5);
    let labels = propagate_labels(&graph, &[(cx(0), 1.0)], 32, 1.0e-6).unwrap();

    println!(
        "PH52_LABEL_PATH {}",
        serde_json::to_string(&labels).unwrap()
    );
    write_readback("ph52-label-path.json", json!({ "labels": labels }));

    assert_eq!(labels[0].node_id, cx(0));
    assert!(!labels[0].provisional);
    assert_eq!(labels[0].hop_distance, 0);
    assert_eq!(labels[0].confidence, 1.0);
    assert!(labels[1].confidence > labels[2].confidence);
    assert!(labels[2].confidence > labels[3].confidence);
    assert!(labels[3].confidence > labels[4].confidence);
    assert!(labels[1..].iter().all(|row| row.provisional));
}

#[test]
fn star_graph_rare_class_carrier_gives_equal_leaf_confidence() {
    let graph = star_graph(5);
    let labels = propagate_labels(&graph, &[(cx(0), 1.0)], 32, 1.0e-6).unwrap();
    let expected = (-0.5_f32).exp();
    let leaves = &labels[1..];

    println!(
        "PH52_LABEL_STAR expected_leaf_confidence={expected:.6} labels={}",
        serde_json::to_string(leaves).unwrap()
    );
    write_readback(
        "ph52-label-star.json",
        json!({ "expected_leaf_confidence": expected, "labels": labels }),
    );

    for leaf in leaves {
        assert_eq!(leaf.hop_distance, 1);
        assert!(leaf.provisional);
        assert!((leaf.confidence - expected).abs() < 0.01);
    }
}

#[test]
fn disconnected_node_is_provisional_zero_confidence() {
    let graph = graph_with_disconnected();
    let labels = propagate_labels(&graph, &[(cx(0), 1.0)], 32, 1.0e-6).unwrap();
    let disconnected = labels.iter().find(|row| row.node_id == cx(9)).unwrap();

    write_readback(
        "ph52-label-disconnected.json",
        json!({ "disconnected": disconnected, "labels": labels }),
    );

    assert_eq!(disconnected.hop_distance, u32::MAX);
    assert_eq!(disconnected.confidence, 0.0);
    assert!(disconnected.provisional);
}

#[test]
fn edge_cases_fail_closed_with_codes() {
    let graph = path_graph(3);
    let slow_graph = path_graph(4);
    let empty_graph = AssocGraph::builder().build();
    let single = single_node_graph();
    let cycle = cycle_graph(4);

    let no_kernel = propagate_labels(&graph, &[], 16, 1.0e-6).unwrap_err();
    let empty = propagate_labels(&empty_graph, &[(cx(0), 1.0)], 16, 1.0e-6).unwrap_err();
    let invalid_label = propagate_labels(&graph, &[(cx(0), 1.5)], 16, 1.0e-6).unwrap_err();
    let duplicate_kernel =
        propagate_labels(&graph, &[(cx(0), 1.0), (cx(0), 1.0)], 16, 1.0e-6).unwrap_err();
    let not_converged =
        propagate_labels(&slow_graph, &[(cx(0), 1.0), (cx(3), 0.0)], 1, 1.0e-9).unwrap_err();
    let single_label = propagate_labels(&single, &[(cx(0), 0.75)], 16, 1.0e-6).unwrap();
    let cycle_labels = propagate_labels(&cycle, &[(cx(0), 1.0)], 16, 1.0e-6).unwrap();

    write_readback(
        "ph52-label-edges.json",
        json!({
            "no_kernel": no_kernel.code(),
            "empty_graph": empty.code(),
            "invalid_label": invalid_label.code(),
            "duplicate_kernel": duplicate_kernel.code(),
            "not_converged": not_converged.code(),
            "single_node": single_label,
            "cycle": cycle_labels,
        }),
    );

    assert_eq!(no_kernel.code(), CALYX_PROP_NO_KERNEL_NODES);
    assert_eq!(empty.code(), CALYX_PROP_GRAPH_EMPTY);
    assert_eq!(invalid_label.code(), CALYX_PROP_INVALID_INPUT);
    assert_eq!(duplicate_kernel.code(), CALYX_PROP_INVALID_INPUT);
    assert_eq!(not_converged.code(), CALYX_PROP_NOT_CONVERGED);
    assert_eq!(single_label.len(), 1);
    assert!(!single_label[0].provisional);
    assert!(cycle_labels.iter().all(|row| row.hop_distance != u32::MAX));
}

#[test]
fn propagation_run_writes_real_ledger_row() {
    let root = fsv_root().join("ph52-label-propagation-ledger");
    reset_dir(&root);
    let ledger_dir = root.join("ledger-cf");
    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap();
    let graph = path_graph(5);
    let kernel_labels = [(cx(0), 1.0)];
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(1_786_000_000),
    )
    .unwrap();

    let receipt =
        propagate_labels_with_ledger(&graph, &kernel_labels, 32, 1.0e-6, 52_448, &mut appender)
            .unwrap();
    drop(appender);
    let after_store = DirectoryLedgerStore::open(&ledger_dir).unwrap();
    let after_rows = after_store.scan().unwrap();
    let decoded = after_rows
        .iter()
        .map(|row| decode(&row.bytes).unwrap())
        .collect::<Vec<_>>();
    let payload: serde_json::Value = serde_json::from_slice(&decoded[0].payload).unwrap();
    let readback = json!({
        "before_rows": before_rows.len(),
        "after_rows": after_rows.len(),
        "ledger_dir": ledger_dir,
        "ledger_ref": receipt.ledger_ref,
        "kernel_hash": receipt.kernel_hash,
        "expected_kernel_hash": hex(&kernel_labels_hash(&kernel_labels)),
        "payload": payload,
        "labels": receipt.labels,
    });
    write_path(&root.join("ph52-label-ledger-readback.json"), &readback);
    write_path(
        &root.join("ph52-label-ledger-decoded.json"),
        &json!(decoded_rows(&decoded)),
    );
    println!("PH52_LABEL_LEDGER_ROOT={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before_rows.len(), 0);
    assert_eq!(after_rows.len(), 1);
    assert_eq!(decoded[0].kind, EntryKind::Kernel);
    assert_eq!(decoded[0].entry_hash, receipt.ledger_ref.hash);
    assert_eq!(payload["graph_version"], json!(52_448));
    assert_eq!(payload["n_propagated"], json!(4));
}

proptest! {
    #[test]
    fn confidence_is_monotone_by_hop_distance(edge_count in 3usize..12) {
        let graph = connected_line_with_shortcuts(edge_count);
        let labels = propagate_labels(&graph, &[(cx(0), 1.0)], 64, 1.0e-6).unwrap();
        for left in &labels {
            for right in &labels {
                if left.hop_distance < right.hop_distance {
                    prop_assert!(left.confidence + 1.0e-6 >= right.confidence);
                }
            }
        }
    }
}

fn path_graph(count: u8) -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for node in 0..count {
        builder.add_node(cx(node), 1.0).unwrap();
    }
    for node in 0..count - 1 {
        add_undirected(&mut builder, node, node + 1);
    }
    builder.build()
}

fn star_graph(leaves: u8) -> AssocGraph {
    let mut builder = AssocGraph::builder();
    builder.add_node(cx(0), 1.0).unwrap();
    for leaf in 1..=leaves {
        builder.add_node(cx(leaf), 1.0).unwrap();
        add_undirected(&mut builder, 0, leaf);
    }
    builder.build()
}

fn graph_with_disconnected() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for node in [0, 1, 2, 9] {
        builder.add_node(cx(node), 1.0).unwrap();
    }
    add_undirected(&mut builder, 0, 1);
    add_undirected(&mut builder, 1, 2);
    builder.build()
}

fn single_node_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    builder.add_node(cx(0), 1.0).unwrap();
    builder.build()
}

fn cycle_graph(count: u8) -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for node in 0..count {
        builder.add_node(cx(node), 1.0).unwrap();
    }
    for node in 0..count {
        add_undirected(&mut builder, node, (node + 1) % count);
    }
    builder.build()
}

fn connected_line_with_shortcuts(count: usize) -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for node in 0..count {
        builder.add_node(cx(node as u8), 1.0).unwrap();
    }
    for node in 0..count - 1 {
        add_undirected(&mut builder, node as u8, node as u8 + 1);
    }
    for node in 0..count.saturating_sub(2) {
        if node % 2 == 0 {
            add_undirected(&mut builder, node as u8, node as u8 + 2);
        }
    }
    builder.build()
}

fn add_undirected(builder: &mut calyx_paths::AssocGraphBuilder, left: u8, right: u8) {
    builder.add_edge(cx(left), cx(right), 1.0).unwrap();
    builder.add_edge(cx(right), cx(left), 1.0).unwrap();
}

fn write_readback(name: &str, value: serde_json::Value) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    write_path(&PathBuf::from(root).join(name), &value);
}

fn write_path(path: &Path, value: &serde_json::Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    println!("PH52_LABEL_READBACK={}", path.display());
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph52-label-propagation"))
}

fn reset_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
}

fn decoded_rows(entries: &[calyx_ledger::LedgerEntry]) -> Vec<serde_json::Value> {
    entries
        .iter()
        .map(|entry| {
            json!({
                "seq": entry.seq,
                "kind": entry.kind.as_str(),
                "prev_hash": hex(&entry.prev_hash),
                "entry_hash": hex(&entry.entry_hash),
                "payload": serde_json::from_slice::<serde_json::Value>(&entry.payload).unwrap(),
            })
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
