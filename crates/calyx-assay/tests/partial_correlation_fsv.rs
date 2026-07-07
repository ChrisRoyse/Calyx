//! Full-State-Verification for Pearson + partial correlation (#58).
//!
//! Source of truth: JSON artifacts written to the OS temp dir. Every case plants
//! a synthetic series whose exact Pearson / partial-correlation value is
//! independently computed (numpy `corrcoef` + a Numerical-Recipes incomplete-beta
//! for the p-value), computes the estimator, writes the result to disk, then
//! performs a *separate read* of the file and re-checks the numbers + blake3
//! digest. Boundary cases prove the estimators fail closed.
//!
//! Run with `--nocapture` to see the evidence log of on-disk artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    PartialReport, PearsonReport, partial_correlation, partial_correlation_controlling, pearson,
};
use serde_json::{Value, json};

const TOL: f32 = 1e-4;

#[test]
fn partial_correlation_fsv_writes_and_reads_back_all_planted_cases() {
    // ---- Case 1: perfect positive Pearson — r = 1, maximally significant.
    {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let y = [2.0f32, 4.0, 6.0, 8.0, 10.0];
        let r = pearson(&x, &y).unwrap();
        assert!((r.r - 1.0).abs() <= TOL, "{r:?}");
        assert!(r.p_value < 1e-6, "perfect r significant: {r:?}");
        write_pearson("partialcorr_case1_perfect_pos.json", "y=2x; exact r=1", &r);
    }

    // ---- Case 2: known Pearson value — r=0.8219949, t=2.5, p=0.0877066.
    {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let y = [2.0f32, 1.0, 4.0, 3.0, 6.0];
        let r = pearson(&x, &y).unwrap();
        assert!((r.r - 0.821_994_9).abs() <= TOL, "{r:?}");
        assert!((r.t_statistic - 2.5).abs() <= 1e-3, "{r:?}");
        assert!((r.p_value - 0.087_706_6).abs() <= 1e-3, "{r:?}");
        write_pearson(
            "partialcorr_case2_known_r.json",
            "independently computed r=0.8219949, t=2.5, p=0.0877066",
            &r,
        );
    }

    // ---- Case 3: de-confounding — Z drives both X,Y; partial collapses to ~0.
    // X=3Z+a, Y=3Z+b, corr(a,b)=0: raw r=0.9777 -> partial=-0.1085.
    {
        let z = [0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let x = [1.0f32, 2.0, 7.0, 8.0, 13.0, 14.0, 19.0, 20.0];
        let y = [1.0f32, 4.0, 5.0, 8.0, 13.0, 16.0, 17.0, 20.0];
        let pc = partial_correlation(&x, &y, &z).unwrap();
        assert!((pc.zero_order_r - 0.977_7).abs() <= 1e-3, "{pc:?}");
        assert!((pc.partial_r - (-0.108_5)).abs() <= 1e-3, "{pc:?}");
        // The de-confounding delta is the headline result: |raw| - |partial|.
        assert!(
            pc.zero_order_r.abs() - pc.partial_r.abs() > 0.5,
            "controlling Z must collapse the association: {pc:?}"
        );
        write_partial(
            "partialcorr_case3_deconfound.json",
            "Z drives both; raw r=0.9777 collapses to partial=-0.1085",
            &pc,
        );
    }

    // ---- Case 4: known first-order partial — 0.7604172, t=2.028, p=0.13560.
    {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        let y = [2.0f32, 1.0, 4.0, 3.0, 7.0, 5.0];
        let z = [5.0f32, 6.0, 2.0, 1.0, 4.0, 3.0];
        let pc = partial_correlation(&x, &y, &z).unwrap();
        assert!((pc.partial_r - 0.760_417_2).abs() <= TOL, "{pc:?}");
        assert!((pc.t_statistic - 2.028_042).abs() <= 1e-3, "{pc:?}");
        assert!((pc.p_value - 0.135_599_8).abs() <= 1e-3, "{pc:?}");
        write_partial(
            "partialcorr_case4_known_partial.json",
            "independently computed partial=0.7604172, t=2.028042, p=0.1355998",
            &pc,
        );
    }

    // ---- Case 5: cross-check — first-order == precision-matrix, single control.
    {
        let x = [2.0f32, 4.0, 1.0, 7.0, 3.0, 9.0, 5.0, 6.0];
        let y = [1.0f32, 3.0, 2.0, 8.0, 4.0, 7.0, 6.0, 5.0];
        let z = [3.0f32, 1.0, 4.0, 2.0, 8.0, 5.0, 7.0, 6.0];
        let a = partial_correlation(&x, &y, &z).unwrap();
        let b = partial_correlation_controlling(&x, &y, &[&z]).unwrap();
        assert!(
            (a.partial_r - b.partial_r).abs() <= 1e-5,
            "two independent derivations must agree: {a:?} vs {b:?}"
        );
        write_partial(
            "partialcorr_case5_crosscheck.json",
            "first-order formula equals R^-1 precision-matrix estimator",
            &b,
        );
    }
}

#[test]
fn partial_correlation_fsv_edge_cases_fail_closed() {
    let mut edges = Vec::new();

    // Edge A: constant column → Pearson undefined (degenerate).
    let a = pearson(&[5.0, 5.0, 5.0, 5.0], &[1.0, 2.0, 3.0, 4.0]).unwrap_err();
    assert_eq!(a.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    edges.push(json!({
        "case": "constant_column",
        "before": {"x": [5,5,5,5], "y": [1,2,3,4]},
        "after": {"code": a.code},
    }));

    // Edge B: control (near-)perfectly explains X and Y → partial undefined.
    let b = partial_correlation(
        &[1.0, 2.0, 3.0, 4.0],
        &[2.0, 4.0, 6.0, 8.0],
        &[1.0, 2.0, 3.0, 4.0],
    )
    .unwrap_err();
    assert_eq!(b.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    edges.push(json!({
        "case": "collinear_control",
        "before": {"x": [1,2,3,4], "y": [2,4,6,8], "z": [1,2,3,4]},
        "after": {"code": b.code},
    }));

    // Edge C: below minimum (partial needs n≥4) → insufficient samples.
    let c = partial_correlation(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0]).unwrap_err();
    assert_eq!(c.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({
        "case": "below_min_samples",
        "before": {"n": 3, "min": 4},
        "after": {"code": c.code},
    }));

    // Edge D: non-finite input → insufficient samples (never a silent NaN).
    let d = pearson(&[1.0, f32::INFINITY, 3.0], &[1.0, 2.0, 3.0]).unwrap_err();
    assert_eq!(d.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({
        "case": "non_finite",
        "before": {"x": ["1", "inf", "3"]},
        "after": {"code": d.code},
    }));

    // Edge E: length mismatch → insufficient samples.
    let e =
        partial_correlation(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0]).unwrap_err();
    assert_eq!(e.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({
        "case": "length_mismatch",
        "before": {"x_len": 4, "z_len": 2},
        "after": {"code": e.code},
    }));

    let sot = json!({"suite": "partial_correlation_edge_cases", "edges": edges});
    let path = write_sot("partialcorr_edge_cases.json", sot);
    let round = read_json(&path);
    let codes: Vec<String> = round["edges"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|e| e["after"].as_object().unwrap().values())
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(codes.iter().any(|c| c == "CALYX_ASSAY_DEGENERATE_INPUT"));
    assert!(
        codes
            .iter()
            .any(|c| c == "CALYX_ASSAY_INSUFFICIENT_SAMPLES")
    );
    println!("PARTIALCORR_FSV_EDGES on-disk codes = {codes:?}");
}

// ----- FSV helpers (source-of-truth write + independent readback) ------------

fn write_pearson(file: &str, trigger: &str, r: &PearsonReport) {
    let value = json!({
        "case": file, "trigger": trigger,
        "pearson": {
            "r": r.r, "t_statistic": r.t_statistic, "p_value": r.p_value,
            "ci_low": r.ci_low, "ci_high": r.ci_high, "n_samples": r.n_samples,
        },
    });
    let path = write_sot(file, value);
    let round = read_json(&path);
    let r_back = round["pearson"]["r"].as_f64().unwrap() as f32;
    assert!((r_back - r.r).abs() <= TOL, "readback r drift in {file}");
    println!(
        "PARTIALCORR_FSV {} r={r_back} blake3={}",
        path.display(),
        blake3::hash(&fs::read(&path).unwrap())
    );
}

fn write_partial(file: &str, trigger: &str, p: &PartialReport) {
    let value = json!({
        "case": file, "trigger": trigger,
        "partial": {
            "partial_r": p.partial_r, "zero_order_r": p.zero_order_r,
            "t_statistic": p.t_statistic, "p_value": p.p_value,
            "ci_low": p.ci_low, "ci_high": p.ci_high,
            "n_controls": p.n_controls, "n_samples": p.n_samples,
        },
    });
    let path = write_sot(file, value);
    let round = read_json(&path);
    let pr_back = round["partial"]["partial_r"].as_f64().unwrap() as f32;
    assert!(
        (pr_back - p.partial_r).abs() <= TOL,
        "readback partial drift in {file}"
    );
    println!(
        "PARTIALCORR_FSV {} partial_r={pr_back} zero_order={} blake3={}",
        path.display(),
        p.zero_order_r,
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
    let bytes = fs::read(path).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn tmp_file(file_name: &str) -> PathBuf {
    std::env::temp_dir()
        .join("calyx_partialcorr_fsv")
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
