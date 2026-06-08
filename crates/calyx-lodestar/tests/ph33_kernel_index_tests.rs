use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_core::CxId;
use calyx_lodestar::{
    FsKernelStore, GroundednessReport, Kernel, LodestarError, RecallReport, build_kernel_index,
    kernel_search, load_kernel_index, write_kernel_index,
};
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn kernel(members: Vec<CxId>) -> Kernel {
    Kernel {
        kernel_id: cx(99),
        panel_version: 1,
        anchor_kind: Some("synthetic_anchor".to_string()),
        corpus_shard_hash: [7; 32],
        members: members.clone(),
        kernel_graph: members,
        groundedness: GroundednessReport {
            reached_anchor: 1.0,
            unanchored_members: Vec::new(),
        },
        recall: RecallReport::default(),
        built_at_millis: 1,
        estimator_provenance: "test".to_string(),
        warnings: Vec::new(),
    }
}

fn embeddings() -> BTreeMap<CxId, Vec<f32>> {
    BTreeMap::from([
        (cx(1), vec![1.0, 0.0, 0.0]),
        (cx(2), vec![0.0, 1.0, 0.0]),
        (cx(3), vec![0.0, 0.0, 1.0]),
        (cx(4), vec![0.7, 0.7, 0.0]),
        (cx(5), vec![-1.0, 0.0, 0.0]),
    ])
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph33-t01"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("readback write");
    println!("PH33_T01_READBACK={}", path.display());
}

#[test]
fn kernel_index_builds_searches_and_handles_top_k_edges() {
    let ids = vec![cx(1), cx(2), cx(3), cx(4), cx(5)];
    let index = build_kernel_index(&kernel(ids), &embeddings()).unwrap();
    let top3 = kernel_search(&index, &[1.0, 0.0, 0.0], 3).unwrap();
    let repeat = kernel_search(&index, &[1.0, 0.0, 0.0], 3).unwrap();
    let all = kernel_search(&index, &[1.0, 0.0, 0.0], 99).unwrap();

    println!("KERNEL_INDEX_SEARCH top3={top3:?} all_len={}", all.len());
    write_readback(
        "search",
        "kernel-index-search.json",
        json!({
            "top3": top3,
            "repeat": repeat,
            "all_len": all.len(),
            "rows": index.rows(),
        }),
    );

    assert_eq!(top3, repeat);
    assert_eq!(top3[0].0, cx(1));
    assert!((top3[0].1 - 1.0).abs() <= 1e-6);
    assert_eq!(all.len(), 5);
}

#[test]
fn kernel_index_write_load_round_trip_reads_same_results() {
    let ids = vec![cx(1), cx(2), cx(3), cx(4), cx(5)];
    let index = build_kernel_index(&kernel(ids), &embeddings()).unwrap();
    let store = FsKernelStore::new(fsv_root("roundtrip"));
    let before = kernel_search(&index, &[0.72, 0.68, 0.0], 4).unwrap();

    write_kernel_index(&index, &store).unwrap();
    let path = store.index_file_path(index.kernel_id);
    let bytes = fs::read(&path).expect("index bytes");
    let loaded = load_kernel_index(index.kernel_id, &store).unwrap();
    let after = kernel_search(&loaded, &[0.72, 0.68, 0.0], 4).unwrap();

    println!(
        "KERNEL_INDEX_ROUNDTRIP path={} bytes={} before={before:?} after={after:?}",
        path.display(),
        bytes.len()
    );
    write_readback(
        "roundtrip",
        "kernel-index-roundtrip.json",
        json!({
            "path": path,
            "bytes": bytes.len(),
            "before": before,
            "after": after,
        }),
    );

    assert!(path.exists());
    assert_eq!(before.len(), after.len());
    for (left, right) in before.iter().zip(&after) {
        assert_eq!(left.0, right.0);
        assert!((left.1 - right.1).abs() <= 1e-6);
    }
}

#[test]
fn kernel_index_fail_closed_edges_report_catalog_codes() {
    let ids = vec![cx(1), cx(2), cx(3)];
    let index = build_kernel_index(&kernel(ids), &embeddings()).unwrap();
    let dim_err = kernel_search(&index, &[1.0, 0.0], 1).unwrap_err();
    let missing_store = FsKernelStore::new(fsv_root("missing"));
    let missing = load_kernel_index(cx(200), &missing_store).unwrap_err();
    let empty = build_kernel_index(&kernel(Vec::new()), &embeddings()).unwrap_err();

    println!(
        "KERNEL_INDEX_ERRORS dim={} missing={} empty={}",
        dim_err.code(),
        missing.code(),
        empty.code()
    );
    write_readback(
        "edges",
        "kernel-index-errors.json",
        json!({
            "dim": dim_err.code(),
            "missing": missing.code(),
            "empty": empty.code(),
        }),
    );

    assert_eq!(dim_err.code(), "CALYX_KERNEL_DIM_MISMATCH");
    assert_eq!(missing.code(), "CALYX_KERNEL_INDEX_NOT_FOUND");
    assert!(matches!(empty, LodestarError::KernelEmptyResult));
}
