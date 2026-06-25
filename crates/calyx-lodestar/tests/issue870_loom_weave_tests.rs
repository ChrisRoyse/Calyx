use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CxId, SlotId};
use calyx_lodestar::{
    LoomDirectionalConfidence, LoomSlotNode, LoomWeaveReportParams, build_assoc_graph_from_loom,
    loom_weave_report,
};
use calyx_loom::LoomStore;
use calyx_paths::AssocGraph;
use serde_json::json;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

#[test]
fn loom_weave_report_records_cf_graph_counts_and_grounding() {
    let dir = test_dir("happy");
    let xterm_cx = cx(1);
    let clinical = cx(10);
    let target = cx(11);
    let clinical_slot = slot(1);
    let target_slot = slot(2);
    let mut router = CfRouter::open(&dir, 1024).unwrap();
    let mut store = LoomStore::new(8);
    let slots = BTreeMap::from([
        (clinical_slot, vec![1.0, 0.0]),
        (target_slot, vec![1.0, 0.0]),
    ]);

    store.weave(xterm_cx, &slots).unwrap();
    let persisted_xterms = store.persist_xterms_to_aster(&mut router).unwrap();
    drop(router);

    let router = CfRouter::open(&dir, 1024).unwrap();
    let cf_row_count = router.iter_cf(ColumnFamily::XTerm).unwrap().len();
    let loaded = LoomStore::load_xterms_from_aster(&router, 8).unwrap();
    let bindings = [
        LoomSlotNode {
            xterm_cx,
            slot: clinical_slot,
            node: clinical,
        },
        LoomSlotNode {
            xterm_cx,
            slot: target_slot,
            node: target,
        },
    ];
    let confidences = [
        LoomDirectionalConfidence {
            xterm_cx,
            src_slot: clinical_slot,
            dst_slot: target_slot,
            confidence: 0.8,
        },
        LoomDirectionalConfidence {
            xterm_cx,
            src_slot: target_slot,
            dst_slot: clinical_slot,
            confidence: 0.25,
        },
    ];
    let (graph, provenance) =
        build_assoc_graph_from_loom(&loaded, &bindings, &confidences).unwrap();
    let params = LoomWeaveReportParams {
        max_groundedness_distance: 2,
        min_groundedness_fraction: 0.01,
        max_top_edges: 4,
    };
    let report = loom_weave_report(&graph, &provenance, &[target], &params).unwrap();

    write_readback(
        "happy",
        "issue870_loom_weave_readback.json",
        json!({
            "persisted_xterms": persisted_xterms,
            "cf_row_count": cf_row_count,
            "report": report,
        }),
    );

    assert_eq!(persisted_xterms, 1);
    assert_eq!(cf_row_count, 1);
    assert_eq!(report.node_count, 2);
    assert_eq!(report.edge_count, 2);
    assert_eq!(report.provenance_count, 2);
    assert_eq!(report.unique_xterm_count, 1);
    assert_eq!(report.grounded_node_count, 2);
    assert_eq!(report.groundedness_fraction, 1.0);
    assert!(report.gate_passed);
    assert_eq!(report.top_edges.len(), 2);
    assert_eq!(report.top_edges[0].src, clinical);
    assert_eq!(report.top_edges[0].dst, target);
    assert!((report.top_edges[0].edge_weight - 0.8).abs() <= f32::EPSILON);
    cleanup(dir);
}

#[test]
fn loom_weave_report_records_ungrounded_gate_failure() {
    let (graph, provenance) = synthetic_graph();
    let params = LoomWeaveReportParams {
        max_groundedness_distance: 2,
        min_groundedness_fraction: 0.01,
        max_top_edges: 2,
    };
    let report = loom_weave_report(&graph, &provenance, &[], &params).unwrap();

    write_readback(
        "edges",
        "issue870_loom_weave_ungrounded.json",
        json!({
            "node_count": report.node_count,
            "edge_count": report.edge_count,
            "grounded_node_count": report.grounded_node_count,
            "groundedness_fraction": report.groundedness_fraction,
            "gate_passed": report.gate_passed,
        }),
    );

    assert_eq!(report.node_count, 2);
    assert_eq!(report.edge_count, 1);
    assert_eq!(report.grounded_node_count, 0);
    assert_eq!(report.groundedness_fraction, 0.0);
    assert!(!report.gate_passed);
}

#[test]
fn loom_weave_report_fails_closed_on_empty_graph_and_invalid_params() {
    let (graph, provenance) = synthetic_graph();
    let empty_graph = AssocGraph::builder().build();
    let empty_err =
        loom_weave_report(&empty_graph, &provenance, &[cx(20)], &Default::default()).unwrap_err();
    let mut bad_fraction = LoomWeaveReportParams::default();
    bad_fraction.min_groundedness_fraction = 1.1;
    let fraction_err =
        loom_weave_report(&graph, &provenance, &[cx(20)], &bad_fraction).unwrap_err();
    let mut bad_top_edges = LoomWeaveReportParams::default();
    bad_top_edges.max_top_edges = 0;
    let top_edges_err =
        loom_weave_report(&graph, &provenance, &[cx(20)], &bad_top_edges).unwrap_err();

    write_readback(
        "edges",
        "issue870_loom_weave_errors.json",
        json!({
            "empty_graph": empty_err.code(),
            "bad_fraction": fraction_err.code(),
            "bad_top_edges": top_edges_err.code(),
        }),
    );

    assert_eq!(empty_err.code(), "CALYX_KERNEL_EMPTY_GRAPH");
    assert_eq!(fraction_err.code(), "CALYX_KERNEL_INVALID_PARAMS");
    assert_eq!(top_edges_err.code(), "CALYX_KERNEL_INVALID_PARAMS");
}

fn synthetic_graph() -> (AssocGraph, Vec<calyx_lodestar::LoomAssocEdgeProvenance>) {
    let mut builder = AssocGraph::builder();
    let left = cx(20);
    let right = cx(21);
    builder.add_node(left, 1.0).unwrap();
    builder.add_node(right, 1.0).unwrap();
    builder.add_edge(left, right, 0.5).unwrap();
    (
        builder.build(),
        vec![calyx_lodestar::LoomAssocEdgeProvenance {
            xterm_cx: cx(2),
            src_slot: slot(1),
            dst_slot: slot(2),
            src_cx: left,
            dst_cx: right,
            raw_agreement: 1.0,
            agreement: 1.0,
            directional_confidence: 0.5,
            edge_weight: 0.5,
        }],
    )
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue870-loom-weave"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("readback write");
    println!("ISSUE870_LOOM_WEAVE_READBACK={}", path.display());
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-lodestar-issue870-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
