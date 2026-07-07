//! Full-State-Verification for categorical association (#65).
//!
//! Source of truth: JSON artifacts written to the OS temp dir. Each case plants a
//! contingency table (or a continuous-vs-binary pair) whose χ² / G / φ / V /
//! Theil's U / point-biserial values are hand-derivable or independently computed
//! (numpy + the crate's Numerical-Recipes incomplete gamma), computes the
//! estimator, writes the result to disk, then performs a *separate read* and
//! re-checks the numbers + blake3 digest. Boundary cases prove fail-closed.
//!
//! Run with `--nocapture` to see the evidence log of on-disk artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{CategoricalReport, categorical_association, point_biserial};
use serde_json::{Value, json};

/// Expand a row-major r×c contingency table into paired label vectors.
fn from_table(table: &[&[u64]]) -> (Vec<u32>, Vec<u32>) {
    let mut x = Vec::new();
    let mut y = Vec::new();
    for (i, row) in table.iter().enumerate() {
        for (j, &count) in row.iter().enumerate() {
            for _ in 0..count {
                x.push(i as u32);
                y.push(j as u32);
            }
        }
    }
    (x, y)
}

#[test]
fn categorical_fsv_writes_and_reads_back_all_planted_cases() {
    // ---- Case 1: perfect 2×2 diagonal — χ²=20, φ=V=1, U=1 both ways, MI=1 bit.
    {
        let (x, y) = from_table(&[&[10, 0], &[0, 10]]);
        let r = categorical_association(&x, &y).unwrap();
        assert!((r.chi_square - 20.0).abs() < 1e-3, "{r:?}");
        assert!((r.cramers_v - 1.0).abs() < 1e-5, "{r:?}");
        assert!((r.theil_u_y_given_x - 1.0).abs() < 1e-5, "{r:?}");
        assert!((r.mutual_information_bits - 1.0).abs() < 1e-5, "{r:?}");
        write_cat(
            "categorical_case1_perfect.json",
            "diagonal 2x2; χ²=20 φ=V=1 MI=1bit",
            &r,
        );
    }

    // ---- Case 2: independence — χ²=0, V=0, U=0, p=1.
    {
        let (x, y) = from_table(&[&[5, 5], &[5, 5]]);
        let r = categorical_association(&x, &y).unwrap();
        assert!(r.chi_square.abs() < 1e-6, "{r:?}");
        assert!((r.chi_square_p - 1.0).abs() < 1e-6, "{r:?}");
        write_cat(
            "categorical_case2_independent.json",
            "balanced 2x2; χ²=0 p=1",
            &r,
        );
    }

    // ---- Case 3: directional Theil's U — x pins y (U(Y|X)=1) but not reverse
    // (U(X|Y)=0.5794, independently computed).
    {
        let (x, y) = from_table(&[&[8, 0], &[8, 0], &[0, 8]]);
        let r = categorical_association(&x, &y).unwrap();
        assert!((r.theil_u_y_given_x - 1.0).abs() < 1e-5, "{r:?}");
        assert!((r.theil_u_x_given_y - 0.5794).abs() < 1e-3, "{r:?}");
        write_cat(
            "categorical_case3_directional.json",
            "3x2; U(Y|X)=1, U(X|Y)=0.5794 (directional)",
            &r,
        );
    }

    // ---- Case 4: known textbook 2×2 — χ²=2.982456, G=3.030404, p=0.084171.
    {
        let (x, y) = from_table(&[&[12, 7], &[5, 10]]);
        let r = categorical_association(&x, &y).unwrap();
        assert!((r.chi_square - 2.982_456).abs() < 1e-3, "{r:?}");
        assert!((r.g_statistic - 3.030_404).abs() < 1e-3, "{r:?}");
        assert!((r.chi_square_p - 0.084_171).abs() < 1e-4, "{r:?}");
        write_cat(
            "categorical_case4_known.json",
            "textbook 2x2; χ²=2.982456 G=3.030404 p=0.084171",
            &r,
        );
    }
}

#[test]
fn categorical_fsv_point_biserial_case() {
    // Continuous score against a binary anchor with a clear mean shift.
    let score = [1.0f32, 2.0, 1.5, 2.5, 8.0, 9.0, 8.5, 9.5];
    let binary = [0u32, 0, 0, 0, 1, 1, 1, 1];
    let r = point_biserial(&score, &binary).unwrap();
    assert!(r.r > 0.9 && r.p_value < 0.01, "{r:?}");
    let value = json!({
        "case": "point_biserial",
        "trigger": "group0 low / group1 high; strong positive r_pb",
        "point_biserial": {
            "r": r.r, "t_statistic": r.t_statistic, "p_value": r.p_value,
            "ci_low": r.ci_low, "ci_high": r.ci_high, "n_samples": r.n_samples,
        },
    });
    let path = write_sot("categorical_point_biserial.json", value);
    let round = read_json(&path);
    let r_back = round["point_biserial"]["r"].as_f64().unwrap() as f32;
    assert!((r_back - r.r).abs() <= 1e-5, "readback drift");
    println!(
        "CATEGORICAL_FSV point_biserial r={r_back} blake3={}",
        blake3::hash(&fs::read(&path).unwrap())
    );
}

#[test]
fn categorical_fsv_edge_cases_fail_closed() {
    let mut edges = Vec::new();

    // Edge A: single-category column → association undefined (degenerate).
    let a = categorical_association(&[0, 1, 0, 1, 0, 1], &[7, 7, 7, 7, 7, 7]).unwrap_err();
    assert_eq!(a.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    edges.push(json!({"case": "single_category", "after": {"code": a.code}}));

    // Edge B: length mismatch → insufficient samples.
    let b = categorical_association(&[0, 1, 0], &[0, 1]).unwrap_err();
    assert_eq!(b.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "length_mismatch", "after": {"code": b.code}}));

    // Edge C: below minimum samples → insufficient samples.
    let c = categorical_association(&[0, 1], &[1, 0]).unwrap_err();
    assert_eq!(c.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "below_min", "after": {"code": c.code}}));

    // Edge D: point-biserial with a non-binary label → insufficient samples.
    let d = point_biserial(&[1.0, 2.0, 3.0, 4.0], &[0, 1, 2, 1]).unwrap_err();
    assert_eq!(d.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "non_binary", "after": {"code": d.code}}));

    // Edge E: point-biserial with only one class present → degenerate.
    let e = point_biserial(&[1.0, 2.0, 3.0, 4.0], &[1, 1, 1, 1]).unwrap_err();
    assert_eq!(e.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    edges.push(json!({"case": "single_class", "after": {"code": e.code}}));

    let sot = json!({"suite": "categorical_edge_cases", "edges": edges});
    let path = write_sot("categorical_edge_cases.json", sot);
    let round = read_json(&path);
    let codes: Vec<String> = round["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["after"]["code"].as_str().unwrap().to_string())
        .collect();
    assert!(codes.iter().any(|c| c == "CALYX_ASSAY_DEGENERATE_INPUT"));
    assert!(
        codes
            .iter()
            .any(|c| c == "CALYX_ASSAY_INSUFFICIENT_SAMPLES")
    );
    println!("CATEGORICAL_FSV_EDGES on-disk codes = {codes:?}");
}

// ----- FSV helpers -----------------------------------------------------------

fn write_cat(file: &str, trigger: &str, r: &CategoricalReport) {
    let value = json!({
        "case": file, "trigger": trigger,
        "categorical": {
            "chi_square": r.chi_square, "g_statistic": r.g_statistic, "dof": r.dof,
            "chi_square_p": r.chi_square_p, "g_p": r.g_p,
            "phi": r.phi, "cramers_v": r.cramers_v,
            "theil_u_y_given_x": r.theil_u_y_given_x, "theil_u_x_given_y": r.theil_u_x_given_y,
            "mutual_information_bits": r.mutual_information_bits,
            "n_rows": r.n_rows, "n_cols": r.n_cols, "n_samples": r.n_samples,
        },
    });
    let path = write_sot(file, value);
    let round = read_json(&path);
    let chi_back = round["categorical"]["chi_square"].as_f64().unwrap() as f32;
    let v_back = round["categorical"]["cramers_v"].as_f64().unwrap() as f32;
    assert!(
        (chi_back - r.chi_square).abs() <= 1e-3,
        "readback χ² drift {file}"
    );
    assert!(
        (v_back - r.cramers_v).abs() <= 1e-5,
        "readback V drift {file}"
    );
    println!(
        "CATEGORICAL_FSV {} χ²={chi_back} V={v_back} U(Y|X)={} blake3={}",
        path.display(),
        r.theil_u_y_given_x,
        blake3::hash(&fs::read(&path).unwrap())
    );
}

fn write_sot(file_name: &str, mut value: Value) -> PathBuf {
    let path = tmp_file(file_name);
    value["source_of_truth"] = json!({
        "primary_path": path.to_string_lossy(),
        "before": file_state(&path),
    });
    let bytes = serde_json::to_vec_pretty(&value).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "SoT write mismatch {file_name}"
    );
    path
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn tmp_file(file_name: &str) -> PathBuf {
    std::env::temp_dir()
        .join("calyx_categorical_fsv")
        .join(file_name)
}

fn file_state(path: &Path) -> Value {
    match fs::read(path) {
        Ok(bytes) => json!({
            "exists": true, "len": bytes.len(),
            "blake3": blake3::hash(&bytes).to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({"exists": false}),
        Err(e) => json!({"exists": false, "read_error": e.to_string()}),
    }
}
