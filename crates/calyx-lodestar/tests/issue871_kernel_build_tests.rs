use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::CxId;
use calyx_lodestar::{
    FsKernelStore, InMemoryAnnIndex, InMemoryCorpus, KernelGraphParams, KernelParams, RecallReport,
    RecallTestParams, build_kernel_index, build_kernel_pipeline, kernel_health, kernel_recall_gate,
    read_kernel_artifact, write_kernel_artifact, write_kernel_index,
};
use calyx_paths::AssocGraph;
use serde_json::json;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

#[test]
fn kernel_build_persists_grounded_recall_health_readback() {
    let dir = test_dir("happy");
    let graph = cycle_graph();
    let anchors = [cx(1)];
    let params = kernel_params();
    let mut kernel = build_kernel_pipeline(&graph, &anchors, &params).unwrap();
    let member = kernel.members[0];
    let embeddings = BTreeMap::from([(member, vec![1.0, 0.0])]);
    let index = build_kernel_index(&kernel, &embeddings).unwrap();
    let recall_params = recall_params();
    let full = InMemoryAnnIndex::new(vec![calyx_lodestar::RecallQuery {
        cx_id: member,
        vector: vec![1.0, 0.0],
    }])
    .unwrap();
    let corpus = InMemoryCorpus::new(
        "issue871-synthetic-pass",
        vec![calyx_lodestar::RecallQuery {
            cx_id: member,
            vector: vec![1.0, 0.0],
        }],
    );
    let measured = kernel_recall_gate(&index, &full, &corpus, &recall_params).unwrap();
    let approx_factor = kernel.recall.approx_factor;
    let tau_star_estimate = kernel.recall.tau_star_estimate;
    let tau_star_exact = kernel.recall.tau_star_exact;
    kernel.recall = RecallReport {
        approx_factor,
        tau_star_estimate,
        tau_star_exact,
        ..measured
    };

    let store = FsKernelStore::new(&dir);
    write_kernel_index(&index, &store).unwrap();
    write_kernel_artifact(&kernel, &store).unwrap();
    let readback = read_kernel_artifact(kernel.kernel_id, &store).unwrap();
    let health = kernel_health(kernel.kernel_id, &store).unwrap();
    let kernel_bytes = fs::metadata(store.kernel_file_path(kernel.kernel_id))
        .unwrap()
        .len();
    let index_bytes = fs::metadata(store.index_file_path(kernel.kernel_id))
        .unwrap()
        .len();

    write_readback(
        "happy",
        "issue871_kernel_build_readback.json",
        json!({
            "source_graph": {
                "node_count": graph.node_count(),
                "edge_count": graph.edge_count(),
            },
            "kernel_file_bytes": kernel_bytes,
            "index_file_bytes": index_bytes,
            "readback_kernel_id": readback.kernel_id,
            "member_count": readback.members.len(),
            "kernel_graph_count": readback.kernel_graph.len(),
            "groundedness_fraction": readback.groundedness.reached_anchor,
            "recall_ratio": readback.recall.ratio,
            "tau_star_estimate": readback.recall.tau_star_estimate,
            "tau_star_exact": readback.recall.tau_star_exact,
            "health": health,
        }),
    );

    assert_eq!(readback.kernel_id, kernel.kernel_id);
    assert_eq!(readback.members.len(), 1);
    assert_eq!(readback.kernel_graph.len(), 3);
    assert_eq!(readback.groundedness.reached_anchor, 1.0);
    assert_eq!(readback.recall.ratio, 1.0);
    assert_eq!(readback.recall.tau_star_estimate, 1);
    assert!(readback.recall.tau_star_exact);
    assert_eq!(format!("{:?}", health.recall.pass_mode), "Passed");
    assert_eq!(health.grounded_fraction, 1.0);
    cleanup(dir);
}

#[test]
fn kernel_recall_gate_records_below_gate_failure() {
    let kernel = build_kernel_pipeline(&cycle_graph(), &[cx(1)], &kernel_params()).unwrap();
    let member = kernel.members[0];
    let other = cx(9);
    let index = build_kernel_index(&kernel, &BTreeMap::from([(member, vec![1.0, 0.0])])).unwrap();
    let full = InMemoryAnnIndex::new(vec![
        calyx_lodestar::RecallQuery {
            cx_id: other,
            vector: vec![0.0, 1.0],
        },
        calyx_lodestar::RecallQuery {
            cx_id: member,
            vector: vec![1.0, 0.0],
        },
    ])
    .unwrap();
    let corpus = InMemoryCorpus::new(
        "issue871-synthetic-fail",
        vec![calyx_lodestar::RecallQuery {
            cx_id: other,
            vector: vec![0.0, 1.0],
        }],
    );
    let err = kernel_recall_gate(&index, &full, &corpus, &recall_params()).unwrap_err();

    write_readback(
        "edges",
        "issue871_kernel_recall_fail.json",
        json!({
            "error_code": err.code(),
            "kernel_member": member,
            "full_top_expected": other,
        }),
    );

    assert_eq!(err.code(), "CALYX_KERNEL_RECALL_BELOW_GATE");
}

#[test]
fn kernel_build_fails_closed_on_missing_embedding_and_empty_corpus() {
    let kernel = build_kernel_pipeline(&cycle_graph(), &[cx(1)], &kernel_params()).unwrap();
    let missing = build_kernel_index(&kernel, &BTreeMap::new()).unwrap_err();
    let member = kernel.members[0];
    let index = build_kernel_index(&kernel, &BTreeMap::from([(member, vec![1.0, 0.0])])).unwrap();
    let full = InMemoryAnnIndex::new(vec![calyx_lodestar::RecallQuery {
        cx_id: member,
        vector: vec![1.0, 0.0],
    }])
    .unwrap();
    let empty = InMemoryCorpus::new("issue871-empty", Vec::new());
    let empty_err = kernel_recall_gate(&index, &full, &empty, &recall_params()).unwrap_err();

    write_readback(
        "edges",
        "issue871_kernel_build_errors.json",
        json!({
            "missing_embedding": missing.code(),
            "empty_corpus": empty_err.code(),
        }),
    );

    assert_eq!(missing.code(), "CALYX_KERNEL_EMBEDDING_MISSING");
    assert_eq!(empty_err.code(), "CALYX_RECALL_EMPTY_CORPUS");
}

fn cycle_graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for id in [cx(1), cx(2), cx(3)] {
        builder.add_node(id, 1.0).unwrap();
    }
    builder.add_edge(cx(1), cx(2), 0.9).unwrap();
    builder.add_edge(cx(2), cx(3), 0.9).unwrap();
    builder.add_edge(cx(3), cx(1), 0.9).unwrap();
    builder.build()
}

fn kernel_params() -> KernelParams {
    KernelParams {
        panel_version: 871,
        anchor_kind: Some("issue871-synthetic".to_string()),
        built_at_millis: 1_785_500_000_000,
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 3,
            degree_weight: 0.40,
            betweenness_weight: 0.40,
            groundedness_weight: 0.20,
        },
        ..KernelParams::default()
    }
}

fn recall_params() -> RecallTestParams {
    RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 1,
        rng_seed: 871,
        min_recall_ratio: 0.95,
    }
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue871-kernel-build"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("readback write");
    println!("ISSUE871_KERNEL_BUILD_READBACK={}", path.display());
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-lodestar-issue871-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
