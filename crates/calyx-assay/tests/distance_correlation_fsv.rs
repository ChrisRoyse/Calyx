//! Full-State-Verification for distance correlation (#54).
//!
//! Source of truth: JSON artifacts under `%TEMP%/calyx_dcor_fsv/`. Every case
//! plants a synthetic pair whose exact dCor is known (0 for an orthogonal grid,
//! 1 for a linear map) or whose qualitative truth is known (a symmetric parabola
//! is invisible to Pearson but caught by dCor). Each result is written to disk
//! and re-read independently, with a blake3 digest. Boundary cases prove
//! fail-closed behaviour. Run with `--nocapture` to see the evidence log.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{DcorPermConfig, distance_correlation, distance_correlation_test};
use serde_json::{Value, json};

fn pearson(x: &[f32], y: &[f32]) -> f64 {
    let n = x.len() as f64;
    let mx = x.iter().map(|&v| v as f64).sum::<f64>() / n;
    let my = y.iter().map(|&v| v as f64).sum::<f64>() / n;
    let (mut cov, mut vx, mut vy) = (0.0, 0.0, 0.0);
    for (&a, &b) in x.iter().zip(y) {
        let (da, db) = (a as f64 - mx, b as f64 - my);
        cov += da * db;
        vx += da * da;
        vy += db * db;
    }
    cov / (vx.sqrt() * vy.sqrt())
}

#[test]
fn distance_correlation_fsv_writes_and_reads_back_all_cases() {
    // Case 1: orthogonal 2×2 grid → dCov² = 0, dCor = 0 exactly.
    {
        let x = [0.0f32, 0.0, 1.0, 1.0];
        let y = [0.0f32, 1.0, 0.0, 1.0];
        let r = distance_correlation(&x, &y).unwrap();
        assert!(r.dcor.abs() <= 1e-6 && r.dcov2.abs() <= 1e-7, "{r:?}");
        write_sot(
            "dcor_case1_orthogonal_grid.json",
            json!({
                "trigger": "row vs col of a 2×2 grid; independent → dCor=0 exact",
                "dcor": r.dcor, "dcov2": r.dcov2,
                "dvar_x": r.dvar_x, "dvar_y": r.dvar_y, "n": r.n_samples,
            }),
        );
    }

    // Case 2: exact linear map → dCor = 1.
    {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        let y = [3.0f32, 6.0, 9.0, 12.0, 15.0, 18.0];
        let r = distance_correlation(&x, &y).unwrap();
        assert!((r.dcor - 1.0).abs() <= 1e-6, "{r:?}");
        write_sot(
            "dcor_case2_linear.json",
            json!({
                "trigger": "y = 3x; linear map → dCor=1 exact",
                "dcor": r.dcor, "dcov2": r.dcov2, "n": r.n_samples,
            }),
        );
    }

    // Case 3: symmetric parabola (n=50) — Pearson=0 exactly, dCor caught &
    // permutation-significant. This is the headline property of dCor.
    {
        let mut x = Vec::new();
        let mut y = Vec::new();
        for k in 1..=25i32 {
            for s in [-1i32, 1] {
                let xi = (s * k) as f32;
                x.push(xi);
                y.push(xi * xi);
            }
        }
        let pear = pearson(&x, &y);
        let t = distance_correlation_test(&x, &y, DcorPermConfig::default()).unwrap();
        assert!(pear.abs() < 1e-6, "Pearson must be ~0, got {pear}");
        assert!(t.dcor > 0.4 && t.p_value < 0.01, "{t:?}");
        let path = write_sot(
            "dcor_case3_parabola_pearson_blind.json",
            json!({
                "trigger": "y = x², x = ±1..±25 (n=50); Pearson≈0 but dCor sees it",
                "pearson": pear,
                "dcor": t.dcor, "dcov2": t.dcov2, "p_value": t.p_value,
                "ge_count": t.ge_count, "permutations": t.permutations,
                "seed": t.seed, "n": t.n_samples,
            }),
        );
        // Independent readback: prove the file records Pearson≈0 yet dCor>0.4.
        let round = read_json(&path);
        assert!(round["pearson"].as_f64().unwrap().abs() < 1e-6);
        assert!(round["dcor"].as_f64().unwrap() > 0.4);
        assert!(round["p_value"].as_f64().unwrap() < 0.01);
        println!(
            "DCOR_FSV parabola on-disk pearson={} dcor={} p={}",
            round["pearson"], round["dcor"], round["p_value"]
        );
    }

    // Case 4: independent scatter → weak, insignificant (guards false +ve).
    {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let y = [5.0f32, 3.0, 6.0, 2.0, 7.0, 4.0, 8.0, 1.0];
        let t = distance_correlation_test(&x, &y, DcorPermConfig::default()).unwrap();
        assert!(t.p_value > 0.1, "independent → insignificant: {t:?}");
        write_sot(
            "dcor_case4_independent.json",
            json!({
                "trigger": "scatter; weak dCor, insignificant permutation p",
                "dcor": t.dcor, "p_value": t.p_value, "n": t.n_samples,
            }),
        );
    }

    // Case 5: seeded reproducibility — same seed ⇒ identical p on disk.
    {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        let y = [1.0f32, 8.0, 27.0, 64.0, 125.0, 216.0];
        let cfg = DcorPermConfig {
            permutations: 500,
            seed: 777,
        };
        let a = distance_correlation_test(&x, &y, cfg).unwrap();
        let b = distance_correlation_test(&x, &y, cfg).unwrap();
        assert_eq!(a.p_value, b.p_value);
        assert_eq!(a.ge_count, b.ge_count);
        write_sot(
            "dcor_case5_seeded_reproducible.json",
            json!({
                "trigger": "same seed=777, P=500 twice → identical p & ge_count",
                "p_value_run1": a.p_value, "p_value_run2": b.p_value,
                "ge_count": a.ge_count, "seed": a.seed,
            }),
        );
    }
}

#[test]
fn distance_correlation_fsv_edge_cases_fail_closed() {
    let mut edges = Vec::new();

    let a = distance_correlation(&[5.0, 5.0, 5.0, 5.0], &[1.0, 2.0, 3.0, 4.0]).unwrap_err();
    assert_eq!(a.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    edges.push(json!({"case": "constant_column", "after": a.code}));

    let b = distance_correlation(&[1.0, 2.0, 3.0], &[1.0, 2.0]).unwrap_err();
    assert_eq!(b.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "length_mismatch", "after": b.code}));

    let c = distance_correlation(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]).unwrap_err();
    assert_eq!(c.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES"); // n<4
    edges.push(json!({"case": "below_min_samples", "after": c.code}));

    let d =
        distance_correlation(&[1.0, f32::INFINITY, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0]).unwrap_err();
    assert_eq!(d.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "non_finite", "after": d.code}));

    let cfg = DcorPermConfig {
        permutations: 0,
        seed: 1,
    };
    let e =
        distance_correlation_test(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0], cfg).unwrap_err();
    assert_eq!(e.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "zero_permutations", "after": e.code}));

    let path = write_sot("dcor_edge_cases.json", json!({"edges": edges}));
    let round = read_json(&path);
    let codes: Vec<String> = round["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["after"].as_str().unwrap().to_string())
        .collect();
    assert!(codes.iter().any(|c| c == "CALYX_ASSAY_DEGENERATE_INPUT"));
    assert_eq!(
        codes
            .iter()
            .filter(|c| c.as_str() == "CALYX_ASSAY_INSUFFICIENT_SAMPLES")
            .count(),
        4
    );
    println!("DCOR_FSV_EDGES on-disk codes = {codes:?}");
}

// ----- FSV helpers -----------------------------------------------------------

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
    println!(
        "DCOR_FSV {} blake3={}",
        path.display(),
        blake3::hash(&bytes)
    );
    path
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn tmp_file(file_name: &str) -> PathBuf {
    std::env::temp_dir().join("calyx_dcor_fsv").join(file_name)
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
