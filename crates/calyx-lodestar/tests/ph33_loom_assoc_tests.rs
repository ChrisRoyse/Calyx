use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CxId, SlotId};
use calyx_lodestar::{
    LoomDirectionalConfidence, LoomSlotNode, build_assoc_graph_from_loom, loom_assoc_graph_input,
};
use calyx_loom::LoomStore;
use serde_json::json;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph33-loom-assoc"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("readback write");
    println!("PH33_LOOM_ASSOC_READBACK={}", path.display());
}

#[test]
fn loom_assoc_xterm_cf_builds_lodestar_assoc_graph_with_provenance() {
    let dir = test_dir("happy");
    let xterm_cx = cx(1);
    let src_cx = cx(10);
    let dst_cx = cx(11);
    let src_slot = slot(1);
    let dst_slot = slot(2);
    let mut router = CfRouter::open(&dir, 1024).unwrap();
    let mut store = LoomStore::new(8);
    let slots = BTreeMap::from([(src_slot, vec![1.0, 0.0]), (dst_slot, vec![1.0, 0.0])]);

    store.weave(xterm_cx, &slots).unwrap();
    let persisted = store.persist_xterms_to_aster(&mut router).unwrap();
    drop(router);

    let router = CfRouter::open(&dir, 1024).unwrap();
    let cf_rows: Vec<_> = router
        .iter_cf(ColumnFamily::XTerm)
        .unwrap()
        .into_iter()
        .map(|entry| {
            json!({
                "key_hex": hex(&entry.key),
                "value_len": entry.value.len(),
                "value_json": serde_json::from_slice::<serde_json::Value>(&entry.value).unwrap(),
            })
        })
        .collect();
    let loaded = LoomStore::load_xterms_from_aster(&router, 8).unwrap();
    let bindings = [
        LoomSlotNode {
            xterm_cx,
            slot: src_slot,
            node: src_cx,
        },
        LoomSlotNode {
            xterm_cx,
            slot: dst_slot,
            node: dst_cx,
        },
    ];
    let confidences = [
        LoomDirectionalConfidence {
            xterm_cx,
            src_slot,
            dst_slot,
            confidence: 0.8,
        },
        LoomDirectionalConfidence {
            xterm_cx,
            src_slot: dst_slot,
            dst_slot: src_slot,
            confidence: 0.25,
        },
    ];
    let (graph, provenance) =
        build_assoc_graph_from_loom(&loaded, &bindings, &confidences).unwrap();
    let src_idx = graph.require_node_index(src_cx).unwrap();
    let dst_idx = graph.require_node_index(dst_cx).unwrap();
    let src_to_dst = graph
        .out_edges_by_index(src_idx)
        .iter()
        .find(|edge| edge.dst == dst_idx)
        .copied()
        .unwrap();
    let dst_to_src = graph
        .out_edges_by_index(dst_idx)
        .iter()
        .find(|edge| edge.dst == src_idx)
        .copied()
        .unwrap();

    write_readback(
        "happy",
        "loom-assoc-graph.json",
        json!({
            "persisted_xterms": persisted,
            "cf_rows": cf_rows,
            "graph": {
                "node_count": graph.node_count(),
                "edge_count": graph.edge_count(),
                "src_to_dst_weight": src_to_dst.weight,
                "dst_to_src_weight": dst_to_src.weight,
            },
            "provenance": provenance,
        }),
    );

    assert_eq!(persisted, 1);
    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.edge_count(), 2);
    assert!((src_to_dst.weight - 0.8).abs() <= f32::EPSILON);
    assert!((dst_to_src.weight - 0.25).abs() <= f32::EPSILON);
    assert_eq!(provenance.len(), 2);
    cleanup(dir);
}

#[test]
fn loom_assoc_adapter_fails_closed_on_missing_mapping_and_direction() {
    let mut store = LoomStore::new(8);
    let xterm_cx = cx(2);
    let src_slot = slot(1);
    let dst_slot = slot(2);
    let slots = BTreeMap::from([(src_slot, vec![1.0, 0.0]), (dst_slot, vec![1.0, 0.0])]);
    store.weave(xterm_cx, &slots).unwrap();
    let one_binding = [LoomSlotNode {
        xterm_cx,
        slot: src_slot,
        node: cx(20),
    }];
    let confidences = [LoomDirectionalConfidence {
        xterm_cx,
        src_slot,
        dst_slot,
        confidence: 1.0,
    }];
    let missing_mapping = loom_assoc_graph_input(&store, &one_binding, &confidences).unwrap_err();
    let missing_direction = loom_assoc_graph_input(&store, &[], &[]).unwrap_err();

    write_readback(
        "edges",
        "loom-assoc-errors.json",
        json!({
            "missing_mapping": missing_mapping.code(),
            "missing_directional_confidence": missing_direction.code(),
        }),
    );

    assert_eq!(
        missing_mapping.code(),
        "CALYX_KERNEL_LOOM_SLOT_MAPPING_MISSING"
    );
    assert_eq!(
        missing_direction.code(),
        "CALYX_KERNEL_LOOM_DIRECTIONAL_CONFIDENCE_MISSING"
    );
}

#[test]
fn loom_assoc_adapter_fails_closed_on_orphan_direction_and_invalid_confidence() {
    let store = LoomStore::new(8);
    let xterm_cx = cx(3);
    let src_slot = slot(1);
    let dst_slot = slot(2);
    let bindings = [
        LoomSlotNode {
            xterm_cx,
            slot: src_slot,
            node: cx(30),
        },
        LoomSlotNode {
            xterm_cx,
            slot: dst_slot,
            node: cx(31),
        },
    ];
    let orphan_direction = [LoomDirectionalConfidence {
        xterm_cx,
        src_slot,
        dst_slot,
        confidence: 1.0,
    }];
    let invalid_confidence = [LoomDirectionalConfidence {
        xterm_cx,
        src_slot,
        dst_slot,
        confidence: 1.1,
    }];
    let missing_agreement =
        loom_assoc_graph_input(&store, &bindings, &orphan_direction).unwrap_err();
    let invalid = loom_assoc_graph_input(&store, &bindings, &invalid_confidence).unwrap_err();

    write_readback(
        "edges",
        "loom-assoc-orphan-invalid.json",
        json!({
            "missing_agreement": missing_agreement.code(),
            "invalid_confidence": invalid.code(),
        }),
    );

    assert_eq!(
        missing_agreement.code(),
        "CALYX_KERNEL_LOOM_AGREEMENT_MISSING"
    );
    assert_eq!(invalid.code(), "CALYX_KERNEL_LOOM_AGREEMENT_INVALID");
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-lodestar-loom-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
