//! Full-State-Verification for the HSIC kernel independence test (#55).
//!
//! Source of truth: JSON artifacts written to the OS temp dir. Each case plants a
//! synthetic pair whose HSIC (biased/unbiased) or gamma-null parameters are
//! independently computed (numpy implementation of the same formulas), or whose
//! dependence structure is known by construction, computes the estimator, writes
//! the result to disk, then performs a *separate read* and re-checks the numbers
//! + blake3 digest. Boundary cases prove fail-closed.
//!
//! Run with `--nocapture` to see the evidence log of on-disk artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    HsicConfig, HsicEstimators, HsicPermConfig, HsicReport, hsic, hsic_estimators_with_config,
    hsic_permutation_test, hsic_with_config,
};
use serde_json::{Value, json};

fn fixed(sigma: f64) -> HsicConfig {
    HsicConfig {
        bandwidth_x: Some(sigma),
        bandwidth_y: Some(sigma),
    }
}

/// Deterministic splitmix64 → uniform f64 in [0,1); reproducible, no RNG.
fn splitmix(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64) / ((1_u64 << 53) as f64)
}

#[test]
fn hsic_fsv_writes_and_reads_back_all_planted_cases() {
    // ---- Case 1: RBF σ=1 point-estimate regression (case C).
    // X=Y=[1,2,3,4] → HSIC_b=0.1186381508, HSIC_u=0.1135845698.
    {
        let e =
            hsic_estimators_with_config(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0], fixed(1.0))
                .unwrap();
        assert!((e.hsic_biased - 0.118_638_15).abs() < 1e-6, "{e:?}");
        assert!((e.hsic_unbiased - 0.113_584_57).abs() < 1e-6, "{e:?}");
        write_est(
            "hsic_case1_regression_c.json",
            "RBF σ=1 X=Y=[1..4]; HSIC_b=0.11863815",
            &e,
        );
    }

    // ---- Case 2: reversal invariance (case E == case C).
    {
        let e =
            hsic_estimators_with_config(&[1.0, 2.0, 3.0, 4.0], &[4.0, 3.0, 2.0, 1.0], fixed(1.0))
                .unwrap();
        assert!((e.hsic_biased - 0.118_638_15).abs() < 1e-6, "{e:?}");
        write_est(
            "hsic_case2_reversal.json",
            "Y reversed; HSIC invariant = case C",
            &e,
        );
    }

    // ---- Case 3: gamma-null machinery lock (n=8, y=x, σ=1).
    // T=0.99855307, α=83.98182, β=0.00795453, p tiny.
    {
        let x = [0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let r = hsic_with_config(&x, &x, fixed(1.0)).unwrap();
        assert!((r.test_statistic - 0.998_553).abs() < 1e-4, "{r:?}");
        assert!((r.gamma_shape - 83.981_82).abs() < 1e-1, "{r:?}");
        assert!((r.gamma_scale - 0.007_954_53).abs() < 1e-6, "{r:?}");
        assert!(r.p_value < 1e-3, "{r:?}");
        write_report(
            "hsic_case3_gamma_lock.json",
            "n=8 y=x σ=1; T=0.998553 α=83.98 β=0.0079545",
            &r,
        );
    }

    // ---- Case 4: gamma test discriminates at n=64 (dependent vs independent).
    {
        let n = 64usize;
        let xs: Vec<f32> = (0..n).map(|i| (splitmix(i as u64) * 10.0) as f32).collect();
        let y_dep: Vec<f32> = xs.iter().map(|&v| v * v).collect();
        let y_ind: Vec<f32> = (0..n)
            .map(|i| (splitmix(9000 + i as u64) * 10.0) as f32)
            .collect();
        let dep = hsic(&xs, &y_dep).unwrap();
        let ind = hsic(&xs, &y_ind).unwrap();
        assert!(dep.p_value < 0.01, "dependence rejected: {dep:?}");
        assert!(ind.p_value > 0.05, "independence accepted: {ind:?}");
        write_report(
            "hsic_case4_dep_n64.json",
            "n=64 y=x²; gamma p<0.01 (reject)",
            &dep,
        );
        write_report(
            "hsic_case4_ind_n64.json",
            "n=64 independent; gamma p>0.05 (accept)",
            &ind,
        );
    }

    // ---- Case 5: permutation test agrees (n=40).
    {
        let n = 40usize;
        let xs: Vec<f32> = (0..n).map(|i| (splitmix(i as u64) * 6.0) as f32).collect();
        let y_dep: Vec<f32> = xs.iter().map(|&v| v * v).collect();
        let dep = hsic_permutation_test(&xs, &y_dep, HsicPermConfig::default()).unwrap();
        assert!(dep.p_value < 0.01, "perm dependence rejected: {dep:?}");
        let value = json!({
            "case": "hsic_case5_permutation.json",
            "trigger": "n=40 y=x²; permutation p<0.01",
            "permutation": {
                "hsic_biased": dep.hsic_biased, "p_value": dep.p_value,
                "permutations": dep.permutations, "ge_count": dep.ge_count,
                "seed": dep.seed, "n_samples": dep.n_samples,
            },
        });
        let path = write_sot("hsic_case5_permutation.json", value);
        let round = read_json(&path);
        assert!(
            (round["permutation"]["p_value"].as_f64().unwrap() as f32 - dep.p_value).abs() < 1e-6
        );
        println!(
            "HSIC_FSV perm p={} ge={}/{} blake3={}",
            dep.p_value,
            dep.ge_count,
            dep.permutations,
            blake3::hash(&fs::read(&path).unwrap())
        );
    }
}

#[test]
fn hsic_fsv_edge_cases_fail_closed() {
    let mut edges = Vec::new();

    // Edge A: constant column → undefined bandwidth → degenerate.
    let a = hsic_estimators_with_config(
        &[5.0, 5.0, 5.0, 5.0, 5.0],
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        HsicConfig::default(),
    )
    .unwrap_err();
    assert_eq!(a.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    edges.push(json!({"case": "constant_column", "after": {"code": a.code}}));

    // Edge B: below min samples (n<4) → insufficient samples.
    let b = hsic_estimators_with_config(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0], HsicConfig::default())
        .unwrap_err();
    assert_eq!(b.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "below_min", "after": {"code": b.code}}));

    // Edge C: gamma test below n=6 → insufficient samples.
    let c = hsic(&[1.0, 2.0, 3.0, 4.0, 5.0], &[1.0, 4.0, 9.0, 16.0, 25.0]).unwrap_err();
    assert_eq!(c.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "gamma_below_six", "after": {"code": c.code}}));

    // Edge D: non-finite input → insufficient samples (never a silent NaN).
    let d = hsic_estimators_with_config(
        &[1.0, f32::NAN, 3.0, 4.0],
        &[1.0, 2.0, 3.0, 4.0],
        HsicConfig::default(),
    )
    .unwrap_err();
    assert_eq!(d.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "non_finite", "after": {"code": d.code}}));

    // Edge E: length mismatch → insufficient samples.
    let e = hsic_estimators_with_config(
        &[1.0, 2.0, 3.0, 4.0],
        &[1.0, 2.0, 3.0],
        HsicConfig::default(),
    )
    .unwrap_err();
    assert_eq!(e.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "length_mismatch", "after": {"code": e.code}}));

    let sot = json!({"suite": "hsic_edge_cases", "edges": edges});
    let path = write_sot("hsic_edge_cases.json", sot);
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
    println!("HSIC_FSV_EDGES on-disk codes = {codes:?}");
}

// ----- FSV helpers -----------------------------------------------------------

fn write_est(file: &str, trigger: &str, e: &HsicEstimators) {
    let value = json!({
        "case": file, "trigger": trigger,
        "hsic": {
            "hsic_biased": e.hsic_biased, "hsic_unbiased": e.hsic_unbiased,
            "bandwidth_x": e.bandwidth_x, "bandwidth_y": e.bandwidth_y, "n_samples": e.n_samples,
        },
    });
    let path = write_sot(file, value);
    let round = read_json(&path);
    let hb = round["hsic"]["hsic_biased"].as_f64().unwrap() as f32;
    assert!(
        (hb - e.hsic_biased).abs() <= 1e-6,
        "readback HSIC_b drift {file}"
    );
    println!(
        "HSIC_FSV {} HSIC_b={hb} HSIC_u={} blake3={}",
        path.display(),
        e.hsic_unbiased,
        blake3::hash(&fs::read(&path).unwrap())
    );
}

fn write_report(file: &str, trigger: &str, r: &HsicReport) {
    let value = json!({
        "case": file, "trigger": trigger,
        "hsic": {
            "hsic_biased": r.hsic_biased, "hsic_unbiased": r.hsic_unbiased,
            "test_statistic": r.test_statistic, "p_value": r.p_value,
            "gamma_shape": r.gamma_shape, "gamma_scale": r.gamma_scale,
            "bandwidth_x": r.bandwidth_x, "bandwidth_y": r.bandwidth_y, "n_samples": r.n_samples,
        },
    });
    let path = write_sot(file, value);
    let round = read_json(&path);
    let p = round["hsic"]["p_value"].as_f64().unwrap() as f32;
    assert!((p - r.p_value).abs() <= 1e-6, "readback p drift {file}");
    println!(
        "HSIC_FSV {} T={} p={p} blake3={}",
        path.display(),
        r.test_statistic,
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
    std::env::temp_dir().join("calyx_hsic_fsv").join(file_name)
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
