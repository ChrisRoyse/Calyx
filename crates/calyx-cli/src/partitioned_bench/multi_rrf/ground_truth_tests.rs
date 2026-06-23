use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::load_plan;
use super::*;

#[test]
fn fused_truth_round_trips_and_rejects_stale_plan_bytes() {
    let root = temp_root("fused-truth");
    let plan_path = root.join("plan.json");
    write_plan(&plan_path, 1);
    let plan = load_plan(&plan_path).unwrap();
    let truth_file = root.join("truth.i32bin");
    let manifest_file = root.join("truth.manifest.json");
    let rows = vec![vec![0, 2], vec![1, 3]];

    let written = write(
        &rows,
        Context {
            truth_file: &truth_file,
            manifest_file: &manifest_file,
            plan_path: &plan_path,
            plan: &plan,
            truth_n: 2,
            k: 2,
            truth_depth: 4,
            corpus_rows: 4,
        },
    )
    .unwrap();

    assert_eq!(written["mode"], "generated_fused_rrf_i32bin");
    assert_eq!(written["scale_suitable"], false);
    let loaded = PrecomputedTruth::load(Context {
        truth_file: &truth_file,
        manifest_file: &manifest_file,
        plan_path: &plan_path,
        plan: &plan,
        truth_n: 2,
        k: 2,
        truth_depth: 4,
        corpus_rows: 4,
    })
    .unwrap();
    assert_eq!(loaded.row_ids(0), &[0, 2]);
    assert!(!loaded.scale_suitable());
    assert_eq!(loaded.source()["mode"], "precomputed_fused_rrf_i32bin");
    assert_eq!(loaded.source()["metric_class"], report::METRIC_CLASS);
    assert_eq!(loaded.source()["valid_real_outcome"], false);
    assert_eq!(loaded.source()["grounded_phase_exit_eligible"], false);

    write_plan(&plan_path, 2);
    let changed_plan = load_plan(&plan_path).unwrap();
    let err = PrecomputedTruth::load(Context {
        truth_file: &truth_file,
        manifest_file: &manifest_file,
        plan_path: &plan_path,
        plan: &changed_plan,
        truth_n: 2,
        k: 2,
        truth_depth: 4,
        corpus_rows: 4,
    })
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_STALE");
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
