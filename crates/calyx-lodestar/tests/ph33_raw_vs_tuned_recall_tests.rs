#[path = "support/recall_tuning.rs"]
mod recall_tuning;

use std::fs;
use std::path::PathBuf;

use calyx_core::CxId;
use calyx_lodestar::{CALYX_KERNEL_RECALL_BELOW_GATE, RecallReport};
use recall_tuning::{RecallPassMode, tuning_report};
use serde::Serialize;
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn recall(ratio: f32) -> RecallReport {
    RecallReport {
        kernel_only: ratio,
        full: 1.0,
        ratio,
        warning: (ratio < 0.95)
            .then(|| format!("{CALYX_KERNEL_RECALL_BELOW_GATE}: ratio={ratio:.6} min=0.950000")),
        ..RecallReport::default()
    }
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-raw-vs-tuned-recall"));
    base.join(case)
}

fn write_readback(path: PathBuf, value: &impl Serialize) {
    fs::create_dir_all(path.parent().expect("readback parent")).expect("create readback dir");
    fs::write(&path, serde_json::to_vec_pretty(value).expect("json")).expect("write readback");
    println!("RAW_VS_TUNED_READBACK={}", path.display());
}

#[test]
fn raw_below_tuned_pass_is_explicit_in_json() {
    let raw = recall(0.40);
    let tuned = recall(1.0);
    let report = tuning_report(
        Some(&raw),
        &tuned,
        &[cx(1), cx(2)],
        &[cx(1), cx(2), cx(3)],
        0.95,
    );
    let value = serde_json::to_value(&report).expect("json value");

    assert_eq!(report.pass_mode, RecallPassMode::Tuned);
    assert_eq!(value["pass_mode"], "tuned");
    assert_eq!(value["acceptance_metric"], "tuned_recall.ratio");
    assert_eq!(value["raw_passed"], false);
    assert_eq!(value["tuned_passed"], true);
    assert!(value["raw_recall"]["ratio"].as_f64().unwrap() < 0.95);
    assert!(value["tuned_recall"]["ratio"].as_f64().unwrap() >= 0.95);
    assert_eq!(value["added_member_count"], 1);
    assert_eq!(value["added_member_ids"].as_array().unwrap().len(), 1);

    write_readback(
        fsv_root("raw-below-tuned-pass").join("raw-vs-tuned-report.json"),
        &json!({ "scenario": "raw_below_tuned_pass", "report": report }),
    );
}

#[test]
fn raw_pass_remains_distinct_from_tuned_repair() {
    let raw = recall(0.96);
    let report = tuning_report(Some(&raw), &raw, &[cx(7), cx(8)], &[cx(7), cx(8)], 0.95);
    let value = serde_json::to_value(&report).expect("json value");

    assert_eq!(report.pass_mode, RecallPassMode::Raw);
    assert_eq!(value["pass_mode"], "raw");
    assert_eq!(value["raw_passed"], true);
    assert_eq!(value["tuned_passed"], true);
    assert_eq!(value["added_member_count"], 0);
    assert!(value["added_member_hash"].as_str().unwrap().len() == 32);

    write_readback(
        fsv_root("raw-pass").join("raw-vs-tuned-report.json"),
        &json!({ "scenario": "raw_pass", "report": report }),
    );
}
