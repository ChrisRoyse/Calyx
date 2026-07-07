//! Full-State-Verification for #67 temporal cross-K / co-intensity.
//!
//! Source of truth: one JSON report under CALYX_ISSUE067_FSV_ROOT, then a
//! separate readback that re-checks clustered/inhibited verdicts and edge codes.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{CoIntensityVerdict, temporal_cross_k};
use serde_json::{Value, json};

#[test]
fn issue067_point_process_cointensity_fsv_writes_and_reads_back() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let report_path = root.join("issue067_point_process_cointensity_fsv_report.json");
    let before = file_state(&report_path);

    let radii = [1.0, 2.0, 4.0];
    let (cluster_a, cluster_b) = clustered_events();
    let clustered = temporal_cross_k(&cluster_a, &cluster_b, &radii, 0.0, 120.0).unwrap();
    assert_eq!(clustered.points[0].verdict, CoIntensityVerdict::Clustered);
    assert_eq!(clustered.strongest_cluster_radius, 1.0);
    assert!(
        clustered.strongest_cluster_pair_correlation > 5.0,
        "{clustered:?}"
    );

    let (inhibit_a, inhibit_b) = inhibited_events();
    let inhibited = temporal_cross_k(&inhibit_a, &inhibit_b, &radii, 0.0, 120.0).unwrap();
    assert!(
        inhibited
            .points
            .iter()
            .all(|point| point.verdict == CoIntensityVerdict::Inhibited),
        "{inhibited:?}"
    );
    assert_eq!(inhibited.strongest_inhibition_pair_correlation, 0.0);

    let edges = edge_readbacks();

    let report = json!({
        "schema": "poly.issue067.point_process_cointensity_fsv.v1",
        "proof_claim": "Temporal cross-K and ring pair-correlation detect clustered and inhibited cross-type event streams, then fail closed on invalid inputs.",
        "scope": "1D event-time point process; no spatial edge correction.",
        "source_of_truth": {
            "path": report_path.to_string_lossy(),
            "before": before,
        },
        "external_references_checked": [
            "https://www.rdocumentation.org/packages/spatstat.core/versions/2.3-1/topics/Kcross",
            "https://search.r-project.org/CRAN/refmans/spatstat.explore/html/Kcross.html"
        ],
        "minimum_sufficient_corpus": {
            "clustered_events_per_type": cluster_a.len(),
            "inhibited_events_per_type": inhibit_a.len(),
            "radii": radii,
            "edge_cases": edges.len(),
            "why_smaller_insufficient": "Needs both clustered and inhibited known-truth streams plus fail-closed boundaries.",
            "why_larger_wasteful": "Larger streams exercise the same pair-count, K, ring pair-correlation, write, readback, and edge paths without adding proof."
        },
        "clustered": clustered,
        "inhibited": inhibited,
        "edge_cases": edges,
    });
    let bytes = serde_json::to_vec_pretty(&report).unwrap();
    fs::write(&report_path, &bytes).unwrap();
    assert_eq!(fs::read(&report_path).unwrap(), bytes);

    let readback = read_json(&report_path);
    assert_eq!(
        readback["clustered"]["strongest_cluster_radius"],
        json!(1.0)
    );
    assert!(
        readback["clustered"]["strongest_cluster_pair_correlation"]
            .as_f64()
            .unwrap()
            > 5.0
    );
    assert_eq!(
        readback["inhibited"]["strongest_inhibition_pair_correlation"],
        json!(0.0)
    );
    assert!(
        readback["edge_cases"]
            .as_array()
            .unwrap()
            .iter()
            .all(|edge| edge["after"]["code"] == "CALYX_ASSAY_INSUFFICIENT_SAMPLES")
    );

    let digest = blake3::hash(&fs::read(&report_path).unwrap());
    println!(
        "ISSUE067_FSV path={} blake3={} cluster_g={} inhibited_g={} edges={}",
        report_path.display(),
        digest,
        readback["clustered"]["strongest_cluster_pair_correlation"],
        readback["inhibited"]["strongest_inhibition_pair_correlation"],
        readback["edge_cases"].as_array().unwrap().len()
    );
}

fn edge_readbacks() -> Vec<Value> {
    let empty = temporal_cross_k(&[], &[1.0, 2.0], &[1.0], 0.0, 10.0).unwrap_err();
    assert_eq!(empty.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let unsorted =
        temporal_cross_k(&[1.0, 3.0, 2.0], &[1.5, 2.5, 3.5], &[1.0], 0.0, 10.0).unwrap_err();
    assert_eq!(unsorted.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let bad_radius =
        temporal_cross_k(&[1.0, 2.0], &[1.5, 2.5], &[1.0, 1.0], 0.0, 10.0).unwrap_err();
    assert_eq!(bad_radius.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let bad_window = temporal_cross_k(&[1.0, 2.0], &[1.5, 2.5], &[1.0], 10.0, 0.0).unwrap_err();
    assert_eq!(bad_window.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    vec![
        json!({
            "case": "empty_a",
            "before": {"a_len": 0},
            "after": {"code": empty.code},
        }),
        json!({
            "case": "unsorted_a",
            "before": {"a": [1.0, 3.0, 2.0]},
            "after": {"code": unsorted.code},
        }),
        json!({
            "case": "duplicate_radius",
            "before": {"radii": [1.0, 1.0]},
            "after": {"code": bad_radius.code},
        }),
        json!({
            "case": "bad_window",
            "before": {"start": 10.0, "end": 0.0},
            "after": {"code": bad_window.code},
        }),
    ]
}

fn clustered_events() -> (Vec<f64>, Vec<f64>) {
    let a: Vec<f64> = (1..=10).map(|i| i as f64 * 10.0).collect();
    let b = a.iter().map(|time| time + 0.5).collect();
    (a, b)
}

fn inhibited_events() -> (Vec<f64>, Vec<f64>) {
    let a: Vec<f64> = (1..=10).map(|i| i as f64 * 10.0).collect();
    let b = a.iter().map(|time| time + 5.0).collect();
    (a, b)
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE067_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx_issue067_point_process_fsv"))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn file_state(path: &Path) -> Value {
    match fs::read(path) {
        Ok(bytes) => json!({
            "exists": true,
            "len": bytes.len(),
            "blake3": blake3::hash(&bytes).to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({"exists": false}),
        Err(e) => json!({"exists": false, "read_error": e.to_string()}),
    }
}
