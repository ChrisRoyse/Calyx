use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

#[test]
fn graph_cf_label_anchor_round_trips_chunks() {
    let root = temp_root("i8bin-label-anchor-db");
    let mut labels = Vec::new();
    for idx in 0..17 {
        labels.push(idx % 2 == 0);
    }
    let mut counts = BTreeMap::new();
    counts.insert("0".to_string(), 8);
    counts.insert("1".to_string(), 9);

    let written = write(
        &root,
        "unit_labels",
        "unit",
        1,
        &"11".repeat(32),
        &counts,
        &labels,
        5,
    )
    .unwrap();
    let loaded = read(&root, "unit_labels").unwrap();

    assert!(written.readback_matches);
    assert_eq!(loaded.labels, labels);
    assert_eq!(loaded.db_readback.chunk_count, 4);
    assert_eq!(
        loaded.db_readback.manifest_value_sha256,
        written.manifest_value_sha256
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn graph_cf_label_anchor_refuses_duplicate_key() {
    let root = temp_root("i8bin-label-anchor-db-duplicate");
    let labels = vec![true, false, true, false];
    let counts = BTreeMap::new();

    write(
        &root,
        "unit_labels",
        "unit",
        1,
        &"22".repeat(32),
        &counts,
        &labels,
        4,
    )
    .unwrap();
    let err = write(
        &root,
        "unit_labels",
        "unit",
        1,
        &"22".repeat(32),
        &counts,
        &labels,
        4,
    )
    .unwrap_err();

    assert_eq!(err.code, "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_EXISTS");
    let _ = fs::remove_dir_all(root);
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}
