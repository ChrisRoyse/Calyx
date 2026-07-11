//! Manual full-state verification for issue #1313.

use std::{fs, path::PathBuf};

use calyx_assay::{PcSeries, pc_stable_gaussian};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use serde_json::json;

mod issue1313_support;

use issue1313_support::{
    asymmetric_separator_fixture, canonical_edges, discover, has_edge, name_set, separating_set,
    values,
};

const ASSAY_KEY: &[u8] = b"issue1313/pc-stable/both-endpoints/v1";

#[test]
#[ignore = "manual FSV persists and reopens deterministic PC-stable Assay CF bytes"]
fn issue1313_pc_stable_manual_fsv() {
    let root = fsv_root().join("issue1313-pc-stable");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let cf_root = root.join("aster");
    let report_path = root.join("issue1313-pc-stable-readback.json");

    let data = asymmetric_separator_fixture(256);
    let forward = discover(&data, &["i", "a", "w", "j"], 2);
    let reversed = discover(&data, &["j", "a", "w", "i"], 2);
    let forward_edges = canonical_edges(&forward);
    let reversed_edges = canonical_edges(&reversed);
    assert_eq!(forward_edges, reversed_edges);
    assert!(!forward_edges.is_empty());
    assert!(has_edge(&forward, "i", "a"));
    assert!(has_edge(&reversed, "i", "a"));
    assert!(!has_edge(&forward, "i", "j"));
    assert!(!has_edge(&reversed, "i", "j"));
    assert_eq!(
        separating_set(&forward, "i", "j"),
        Some(name_set(&["a", "w"]))
    );
    assert_eq!(
        separating_set(&reversed, "i", "j"),
        Some(name_set(&["a", "w"]))
    );

    let edges = edge_readbacks(&data);
    let persisted = json!({
        "schema_version": "calyx.issue1313.pc_stable.v1",
        "source_of_truth": "order-invariant PC-stable reports persisted in Aster Assay CF",
        "fixture": {
            "samples": 256,
            "structural_equations": ["a=i+w+0.3*e_a", "j=a+w+e_j"],
            "required_separator": ["a", "w"],
            "alpha": 0.01,
            "max_conditioning": 2,
        },
        "happy_path": {
            "forward": forward,
            "reversed": reversed,
            "canonical_edges": forward_edges,
            "order_invariant_canonical_skeleton": true,
        },
        "edges": edges,
    });
    let persisted_bytes = serde_json::to_vec(&persisted).unwrap();
    let persisted_hash = blake3::hash(&persisted_bytes).to_string();

    let mut router = CfRouter::open(&cf_root, 1_048_576).unwrap();
    let before = router.get(ColumnFamily::Assay, ASSAY_KEY).unwrap();
    let before_rows = router.iter_cf(ColumnFamily::Assay).unwrap();
    assert!(before.is_none());
    assert!(before_rows.is_empty());

    router
        .put(ColumnFamily::Assay, ASSAY_KEY, &persisted_bytes)
        .unwrap();
    let action_read = router
        .get(ColumnFamily::Assay, ASSAY_KEY)
        .unwrap()
        .expect("issue1313 Assay row immediately after put");
    assert_eq!(action_read, persisted_bytes);
    router.flush_cf(ColumnFamily::Assay).unwrap();
    drop(router);

    let reopened = CfRouter::open(&cf_root, 1_048_576).unwrap();
    let reopened_bytes = reopened
        .get(ColumnFamily::Assay, ASSAY_KEY)
        .unwrap()
        .expect("persisted issue1313 Assay row after reopen");
    let raw_rows = reopened.iter_cf(ColumnFamily::Assay).unwrap();
    assert_eq!(reopened_bytes, persisted_bytes);
    assert_eq!(raw_rows.len(), 1);
    let reopened_json: serde_json::Value = serde_json::from_slice(&reopened_bytes).unwrap();
    assert_eq!(reopened_json, persisted);

    let report = json!({
        "source_of_truth": {
            "column_family": "assay",
            "key_utf8": String::from_utf8_lossy(ASSAY_KEY),
            "cf_root": cf_root,
        },
        "before": {
            "key_present": false,
            "raw_rows": before_rows.len(),
        },
        "action": {
            "put_readback_bytes": action_read.len(),
            "put_readback_hash": blake3::hash(&action_read).to_string(),
            "flushed_then_closed": true,
        },
        "after": {
            "reopened_raw_rows": raw_rows.len(),
            "reopened_bytes": reopened_bytes.len(),
            "reopened_hash": blake3::hash(&reopened_bytes).to_string(),
            "expected_hash": persisted_hash,
            "byte_for_byte_match": reopened_bytes == persisted_bytes,
            "decoded_value": reopened_json,
        },
    });
    fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let report_readback: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report_readback["after"]["byte_for_byte_match"], true);
    println!("ISSUE1313_PC_STABLE_READBACK={}", report_path.display());
}

fn edge_readbacks(data: &issue1313_support::Fixture) -> serde_json::Value {
    let depth_one_forward = discover(data, &["i", "a", "w", "j"], 1);
    let depth_one_reversed = discover(data, &["j", "a", "w", "i"], 1);
    assert!(has_edge(&depth_one_forward, "i", "j"));
    assert!(has_edge(&depth_one_reversed, "i", "j"));

    let duplicate_names = [
        PcSeries {
            name: "i",
            values: values(data, "i"),
        },
        PcSeries {
            name: "i",
            values: values(data, "a"),
        },
    ];
    let duplicate_code = pc_stable_gaussian(&duplicate_names, 0.01, 0)
        .unwrap_err()
        .code;

    let ordinary = [
        PcSeries {
            name: "i",
            values: values(data, "i"),
        },
        PcSeries {
            name: "a",
            values: values(data, "a"),
        },
        PcSeries {
            name: "w",
            values: values(data, "w"),
        },
        PcSeries {
            name: "j",
            values: values(data, "j"),
        },
    ];
    let excessive_depth_code = pc_stable_gaussian(&ordinary, 0.01, 3).unwrap_err().code;

    let short_j = &values(data, "j")[..255];
    let mismatched = [
        PcSeries {
            name: "i",
            values: values(data, "i"),
        },
        PcSeries {
            name: "j",
            values: short_j,
        },
    ];
    let mismatch_code = pc_stable_gaussian(&mismatched, 0.01, 0).unwrap_err().code;

    assert_eq!(duplicate_code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert_eq!(excessive_depth_code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert_eq!(mismatch_code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    json!([
        {
            "case": "conditioning_depth_below_required_separator",
            "before": {"max_conditioning": 1},
            "after": {"forward_i_j_retained": true, "reversed_i_j_retained": true},
        },
        {"case": "duplicate_names", "code": duplicate_code},
        {"case": "max_conditioning_too_deep", "code": excessive_depth_code},
        {"case": "sample_length_mismatch", "code": mismatch_code},
    ])
}

fn fsv_root() -> PathBuf {
    calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        std::env::temp_dir().join("calyx-issue1313-pc-stable-fsv")
    })
}
