use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::partitioned_bench::rrf_plan;

#[test]
fn graph_cf_fused_truth_round_trips_and_rejects_stale_plan() {
    let root = temp_root("fused-truth-db");
    let plan_path = root.join("plan.json");
    write_plan(&plan_path, 1);
    let loaded_plan = rrf_plan::load_from_file(&plan_path).unwrap();
    let cf_root = root.join("truth-cf");
    let rows = vec![vec![0, 2], vec![1, 3]];

    let written = write(
        &rows,
        Context {
            cf_root: &cf_root,
            association_key: "unit_fused_truth",
            plan_path: &plan_path,
            plan_sha256: &loaded_plan.plan_sha256,
            plan: &loaded_plan.plan,
            truth_n: 2,
            k: 2,
            truth_depth: 4,
            corpus_rows: 4,
        },
        true,
    )
    .unwrap();
    assert_eq!(written["mode"], "generated_fused_rrf_aster_cf");
    assert_eq!(written["scale_suitable"], true);

    let loaded = DbFusedTruth::load(Context {
        cf_root: &cf_root,
        association_key: "unit_fused_truth",
        plan_path: &plan_path,
        plan_sha256: &loaded_plan.plan_sha256,
        plan: &loaded_plan.plan,
        truth_n: 2,
        k: 2,
        truth_depth: 4,
        corpus_rows: 4,
    })
    .unwrap();
    assert_eq!(loaded.row_ids(0), &[0, 2]);
    assert!(loaded.scale_suitable());
    assert_eq!(loaded.source()["mode"], "precomputed_fused_rrf_aster_cf");
    assert_eq!(loaded.source()["db_readback"]["readback_matches"], true);

    write_plan(&plan_path, 2);
    let changed = rrf_plan::load_from_file(&plan_path).unwrap();
    let err = DbFusedTruth::load(Context {
        cf_root: &cf_root,
        association_key: "unit_fused_truth",
        plan_path: &plan_path,
        plan_sha256: &changed.plan_sha256,
        plan: &changed.plan,
        truth_n: 2,
        k: 2,
        truth_depth: 4,
        corpus_rows: 4,
    })
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_STALE");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn graph_cf_fused_truth_refuses_duplicate_key() {
    let root = temp_root("fused-truth-db-duplicate");
    let plan_path = root.join("plan.json");
    write_plan(&plan_path, 1);
    let loaded_plan = rrf_plan::load_from_file(&plan_path).unwrap();
    let cf_root = root.join("truth-cf");
    write(
        &[vec![0, 1]],
        Context {
            cf_root: &cf_root,
            association_key: "unit_fused_truth",
            plan_path: &plan_path,
            plan_sha256: &loaded_plan.plan_sha256,
            plan: &loaded_plan.plan,
            truth_n: 1,
            k: 2,
            truth_depth: 4,
            corpus_rows: 4,
        },
        true,
    )
    .unwrap();
    let err = write(
        &[vec![0, 1]],
        Context {
            cf_root: &cf_root,
            association_key: "unit_fused_truth",
            plan_path: &plan_path,
            plan_sha256: &loaded_plan.plan_sha256,
            plan: &loaded_plan.plan,
            truth_n: 1,
            k: 2,
            truth_depth: 4,
            corpus_rows: 4,
        },
        true,
    )
    .unwrap_err();

    assert_eq!(
        err.code(),
        "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_EXISTS"
    );
    let _ = fs::remove_dir_all(root);
}

fn write_plan(path: &Path, offset: u16) {
    let slots = (0..4)
        .map(|idx| {
            format!(
                r#"{{"slot":{idx},"lens_id":"{:032x}","weights_sha256":"{:064x}","signal_kind":"learned_encoder","bits_about":0.1,"vault":"vault-{idx}","queries":"queries-{idx}.fbin","corpus":"corpus-{idx}.fbin"}}"#,
                idx + offset,
                idx + offset
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    fs::write(path, format!(r#"{{"slots":[{slots}]}}"#)).unwrap();
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
