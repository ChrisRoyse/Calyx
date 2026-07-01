use std::fs;

use calyx_core::SlotId;

use super::*;

#[test]
fn rebuild_writes_manifest_flat_dense_filter_sidecar_and_searches() {
    let root = scratch("happy");
    let docs = docs([
        (1, vec![1.0, 0.0]),
        (2, vec![0.0, 1.0]),
        (3, vec![0.8, 0.2]),
    ]);

    let summary = rebuild_from_docs(&root, &docs, 7).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let hits = indexes
        .search(SlotId::new(0), &dense(vec![1.0, 0.0]), 2)
        .expect("search");
    let filter_entry = indexes.manifest.filter.as_ref().expect("filter entry");
    let filter_path = root.join(&filter_entry.index_rel);
    let filter_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&filter_path).unwrap()).unwrap();
    let slot_entry = indexes.require_entry(SlotId::new(0)).unwrap();
    let flat_path = root.join(slot_entry.require_index_rel(SlotId::new(0)).unwrap());
    let flat_bytes = fs::read(&flat_path).unwrap();

    assert_eq!(summary.slots, 1);
    assert_eq!(summary.total_rows, 3);
    assert!(summary.manifest_path.is_file());
    assert_eq!(hits[0].cx_id, cx(1));
    assert_eq!(slot_entry.kind, "flat_dense");
    assert!(flat_path.is_file());
    assert_eq!(
        sha256_hex(&flat_bytes),
        slot_entry.require_sha256(SlotId::new(0)).unwrap()
    );
    assert_eq!(filter_json["format"], "calyx-search-filter-index-v1");
    assert_eq!(filter_json["rows"].as_array().unwrap().len(), 3);
    assert!(root.join("idx/search/manifest.json").is_file());
    assert!(root.join("idx/search").read_dir().unwrap().count() >= 3);
    maybe_write_fsv_json(
        "flat-dense-happy-path.json",
        &serde_json::json!({
            "source_of_truth": "persisted search manifest plus flat dense sidecar bytes",
            "root": root.display().to_string(),
            "manifest": serde_json::to_value(&indexes.manifest).unwrap(),
            "flat_path": flat_path.display().to_string(),
            "flat_exists": flat_path.is_file(),
            "flat_bytes": flat_bytes.len(),
            "flat_sha256": sha256_hex(&flat_bytes),
            "manifest_sha256": slot_entry.require_sha256(SlotId::new(0)).unwrap(),
            "hits": hits.iter().map(|hit| hit.cx_id.to_string()).collect::<Vec<_>>(),
        }),
    );
    cleanup(root);
}

#[test]
fn flat_dense_corrupt_sidecar_hash_fails_closed() {
    let root = scratch("flat-corrupt");
    rebuild_from_docs(&root, &docs([(1, vec![1.0, 0.0]), (2, vec![0.0, 1.0])]), 33)
        .expect("rebuild flat dense");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes.require_entry(SlotId::new(0)).unwrap();
    assert_eq!(entry.kind, "flat_dense");
    let path = root.join(entry.require_index_rel(SlotId::new(0)).unwrap());
    let before = fs::read(&path).unwrap();
    fs::write(&path, b"corrupt-flat-dense-sidecar").unwrap();

    let err = indexes
        .search(SlotId::new(0), &dense(vec![1.0, 0.0]), 1)
        .unwrap_err();
    let after = fs::read(&path).unwrap();

    let proof = serde_json::json!({
        "source_of_truth": "flat dense sidecar file bytes after deliberate corruption",
        "path": path.display().to_string(),
        "path_exists": path.is_file(),
        "before_bytes": before.len(),
        "after_bytes": after.len(),
        "before_sha256": sha256_hex(&before),
        "manifest_sha256": entry.require_sha256(SlotId::new(0)).unwrap(),
        "actual_after_sha256": sha256_hex(&after),
        "error_code": err.code(),
        "error_message": err.message(),
    });
    println!("FLAT_DENSE_CORRUPT_HASH_FSV {proof}");
    maybe_write_fsv_json("flat-dense-corrupt-hash-fail-closed.json", &proof);
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("sha256"));
    cleanup(root);
}

#[test]
fn flat_dense_query_dim_mismatch_fails_closed() {
    let root = scratch("flat-dim");
    rebuild_from_docs(&root, &docs([(1, vec![1.0, 0.0])]), 34).expect("rebuild flat dense");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes.require_entry(SlotId::new(0)).unwrap();

    let err = indexes
        .search(SlotId::new(0), &dense(vec![1.0, 0.0, 0.0]), 1)
        .unwrap_err();

    assert_eq!(entry.kind, "flat_dense");
    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("index dim 2 != query dim 3"));
    maybe_write_fsv_json(
        "flat-dense-dim-mismatch-fail-closed.json",
        &serde_json::json!({
            "source_of_truth": "flat dense manifest dim and query vector dim",
            "root": root.display().to_string(),
            "entry_kind": entry.kind,
            "entry_dim": entry.require_dim(SlotId::new(0)).unwrap(),
            "query_dim": 3,
            "error_code": err.code(),
            "error_message": err.message(),
        }),
    );
    cleanup(root);
}

#[test]
fn dense_index_planner_switches_after_flat_threshold() {
    assert!(super::super::dense::should_use_flat_dense_index(32_768));
    assert!(!super::super::dense::should_use_flat_dense_index(32_769));
}

fn maybe_write_fsv_json(name: &str, value: &serde_json::Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).expect("create FSV root");
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(value).expect("serialize FSV"),
    )
    .expect("write FSV");
}

fn cleanup(root: std::path::PathBuf) {
    if calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_none() {
        fs::remove_dir_all(root).ok();
    }
}
