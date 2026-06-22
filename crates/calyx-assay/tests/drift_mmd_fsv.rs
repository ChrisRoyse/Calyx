//! PH70 / issue #609 FSV: MMD drift and change-point over the drift pair.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    DEFAULT_MMD_ALPHA, MmdConfig, gaussian_mmd, gaussian_mmd_with_config, mmd_change_point,
};
use serde_json::json;

const FSV_SEED: u64 = 609;
const PERMUTATIONS: usize = 99;

fn config() -> MmdConfig {
    MmdConfig {
        bandwidth: None,
        permutations: PERMUTATIONS,
        seed: FSV_SEED,
        alpha: DEFAULT_MMD_ALPHA,
    }
}

fn cluster(cx: f64, cy: f64, n: usize, phase: f64) -> Vec<Vec<f64>> {
    (0..n)
        .map(|i| {
            let t = i as f64 + phase;
            vec![cx + 0.05 * t.sin(), cy + 0.05 * (t * 0.7).cos()]
        })
        .collect()
}

#[test]
fn planted_shift_detected_and_control_not_detected() {
    let a = cluster(0.0, 0.0, 32, 0.0);
    let b = cluster(2.0, 2.0, 32, 1.0);
    let control = cluster(0.0, 0.0, 32, 8.0);
    let shift = gaussian_mmd_with_config(&a, &b, &config()).unwrap();
    let no_shift = gaussian_mmd_with_config(&a, &control, &config()).unwrap();
    assert!(shift.significant, "shift report: {shift:?}");
    assert!(!no_shift.significant, "control report: {no_shift:?}");
    assert!(shift.mmd2 > no_shift.mmd2 * 5.0);
}

#[test]
fn change_point_finds_planted_boundary() {
    let a = cluster(0.0, 0.0, 32, 0.0);
    let b = cluster(2.0, 2.0, 32, 1.0);
    let mut stream = a.clone();
    stream.extend(b);
    let report = mmd_change_point(&stream, 12, &config()).unwrap();
    assert!(report.report.significant, "change-point report: {report:?}");
    assert!(
        report.split_index.abs_diff(a.len()) <= 2,
        "split {} should be near boundary {}",
        report.split_index,
        a.len()
    );
}

#[test]
fn mmd_reports_are_deterministic() {
    let a = cluster(0.0, 0.0, 16, 0.0);
    let b = cluster(1.0, 1.0, 16, 1.0);
    let first = gaussian_mmd_with_config(&a, &b, &config()).unwrap();
    let second = gaussian_mmd_with_config(&a, &b, &config()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn fail_closed_error_codes_are_exact() {
    let a = cluster(0.0, 0.0, 8, 0.0);
    let b = cluster(1.0, 1.0, 8, 1.0);
    assert_eq!(
        gaussian_mmd(&[], &[]).unwrap_err().code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
    assert_eq!(
        gaussian_mmd(&a[..3], &b[..3]).unwrap_err().code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
    let mut mismatched = b.clone();
    mismatched[0].push(3.0);
    assert_eq!(
        gaussian_mmd(&a, &mismatched).unwrap_err().code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
    let mut non_finite = b.clone();
    non_finite[1][0] = f64::NAN;
    assert_eq!(
        gaussian_mmd(&a, &non_finite).unwrap_err().code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
    let flat = vec![vec![1.0, 1.0]; 8];
    assert_eq!(
        gaussian_mmd(&flat, &flat).unwrap_err().code,
        "CALYX_ASSAY_LOW_SIGNAL"
    );
}

#[test]
#[ignore = "manual FSV reads drift-pair bytes and writes PH70 source-of-truth evidence"]
fn drift_pair_manual_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let dataset_dir = drift_pair_dir(&root);
    if !dataset_dir.exists() {
        write_fixture_dataset(&dataset_dir);
    }
    let month_a = load_features(&dataset_dir.join("month_a.tsv")).unwrap();
    let month_b = load_features(&dataset_dir.join("month_b.tsv")).unwrap();
    let control = load_features(&dataset_dir.join("month_a_control.tsv")).unwrap();
    let cfg = config();
    let shift = gaussian_mmd_with_config(&month_a, &month_b, &cfg).unwrap();
    let no_shift = gaussian_mmd_with_config(&month_a, &control, &cfg).unwrap();
    let mut stream = month_a.clone();
    stream.extend(month_b.clone());
    let change_point = mmd_change_point(&stream, month_a.len() / 4, &cfg).unwrap();
    let boundary_error = change_point.split_index.abs_diff(month_a.len());
    assert!(
        shift.significant,
        "month A vs B must detect drift: {shift:?}"
    );
    assert!(
        !no_shift.significant,
        "month A vs A-control must not flag drift: {no_shift:?}"
    );
    assert!(
        change_point.report.significant && boundary_error <= month_a.len() / 4,
        "change point must land near A/B boundary: {change_point:?}"
    );
    write_json(
        &root.join("ph70_drift_mmd.json"),
        &json!({
            "dataset_dir": dataset_dir,
            "trigger": "compare month_a vs month_b drift pair and month_a vs month_a_control",
            "expected": {
                "month_a_vs_month_b": "shift_detected",
                "month_a_vs_month_a_control": "no_shift",
                "change_point_boundary": month_a.len(),
            },
            "month_a_rows": month_a.len(),
            "month_b_rows": month_b.len(),
            "month_a_control_rows": control.len(),
            "mmd_alpha": cfg.alpha,
            "permutations": cfg.permutations,
            "shift_report": shift,
            "control_report": no_shift,
            "change_point": {
                "split_index": change_point.split_index,
                "boundary_error": boundary_error,
                "report": change_point.report,
            },
        }),
    );
    write_json(&root.join("ph70_drift_mmd_edges.json"), &edge_cases());
    println!(
        "FSV evidence written under {} — read back ph70_drift_mmd*.json",
        root.display()
    );
}

fn edge_cases() -> serde_json::Value {
    let a = cluster(0.0, 0.0, 8, 0.0);
    let b = cluster(1.0, 1.0, 8, 1.0);
    let control = cluster(0.0, 0.0, 8, 5.0);
    json!([
        edge_case(
            "empty_input",
            json!({"a": 0, "b": 0}),
            gaussian_mmd(&[], &[])
        ),
        edge_case(
            "below_min_samples",
            json!({"a": 3, "b": 3}),
            gaussian_mmd(&a[..3], &b[..3])
        ),
        edge_case("dimension_mismatch", json!({"b_row_0_extra_dim": true}), {
            let mut bad = b.clone();
            bad[0].push(9.0);
            gaussian_mmd(&a, &bad)
        }),
        edge_case("nan_value", json!({"b[1][0]": "NaN"}), {
            let mut bad = b.clone();
            bad[1][0] = f64::NAN;
            gaussian_mmd(&a, &bad)
        }),
        edge_case("zero_signal", json!({"all_rows": [1.0, 1.0]}), {
            let flat = vec![vec![1.0, 1.0]; 8];
            gaussian_mmd(&flat, &flat)
        }),
        edge_case(
            "no_shift_control",
            json!({"same_distribution": true}),
            gaussian_mmd_with_config(&a, &control, &config())
        ),
    ])
}

fn edge_case(
    name: &str,
    state_before: serde_json::Value,
    outcome: Result<calyx_assay::MmdReport, calyx_core::CalyxError>,
) -> serde_json::Value {
    let state_after = match outcome {
        Ok(report) => {
            json!({"ok": {"mmd2": report.mmd2, "p_value": report.p_value, "significant": report.significant}})
        }
        Err(error) => json!({"error_code": error.code, "message": error.message}),
    };
    json!({"case": name, "state_before": state_before, "state_after": state_after})
}

fn load_features(path: &Path) -> Result<Vec<Vec<f64>>, String> {
    let text =
        fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 9 {
            return Err(format!(
                "{} line {} has {} fields",
                path.display(),
                index + 1,
                fields.len()
            ));
        }
        rows.push(
            fields[2..8]
                .iter()
                .map(|value| value.parse::<f64>().map_err(|error| error.to_string()))
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    Ok(rows)
}

fn write_fixture_dataset(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    write_tsv(
        &dir.join("month_a.tsv"),
        &cluster(0.0, 0.0, 32, 0.0),
        "world",
    );
    write_tsv(
        &dir.join("month_a_control.tsv"),
        &cluster(0.0, 0.0, 32, 6.0),
        "world",
    );
    write_tsv(
        &dir.join("month_b.tsv"),
        &cluster(2.0, 2.0, 32, 1.0),
        "business",
    );
    fs::write(
        dir.join("manifest.json"),
        "{\"dataset\":\"synthetic_fallback_drift_pair\",\"seed\":609}\n",
    )
    .unwrap();
}

fn write_tsv(path: &Path, rows: &[Vec<f64>], class_name: &str) {
    let mut text =
        "id\tsource_class\ttoken_count\tmean_token_len\tworld_hits\tsports_hits\tbusiness_hits\tscitech_hits\ttext_sha256\n".to_string();
    for (index, row) in rows.iter().enumerate() {
        text.push_str(&format!(
            "{index}\t{class_name}\t{:.8}\t{:.8}\t{:.8}\t{:.8}\t{:.8}\t{:.8}\tfixture-{index}\n",
            row[0],
            row[1],
            row[0],
            row[1],
            row[0] + row[1],
            (row[0] - row[1]).abs()
        ));
    }
    fs::write(path, text).unwrap();
}

fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-drift-mmd-fsv"))
}

fn drift_pair_dir(root: &Path) -> PathBuf {
    std::env::var("CALYX_DRIFT_PAIR_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("drift_pair_fixture"))
}
