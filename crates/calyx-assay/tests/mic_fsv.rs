//! Full-State-Verification for the Maximal Information Coefficient (#56).
//!
//! Source of truth: JSON artifacts written to the OS temp dir. Each case plants a
//! synthetic pair whose MIC is analytically anchored (noiseless functional
//! relationships → MIC = 1; independence → small) or whose ordering is known by
//! construction, computes the estimator, writes the result to disk, then performs
//! a *separate read* and re-checks the numbers + blake3 digest. Boundary cases
//! prove fail-closed.
//!
//! Run with `--nocapture` to see the evidence log of on-disk artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{MicReport, mic};
use serde_json::{Value, json};

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
fn mic_fsv_writes_and_reads_back_all_planted_cases() {
    // ---- Case 1: noiseless bijection y=x → MIC = 1 (analytic).
    {
        let n = 40;
        let x: Vec<f32> = (0..n).map(|i| i as f32).collect();
        let y = x.clone();
        let r = mic(&x, &y).unwrap();
        assert!((r.mic - 1.0).abs() < 1e-6, "{r:?}");
        write_mic(
            "mic_case1_bijection.json",
            "y=x noiseless bijection; MIC=1",
            &r,
        );
    }

    // ---- Case 2: noiseless parabola y=x² → MIC ≈ 1 (non-monotone; Pearson-blind).
    {
        let x: Vec<f32> = (-20..=20).map(|i| i as f32).collect();
        let y: Vec<f32> = x.iter().map(|&v| v * v).collect();
        let r = mic(&x, &y).unwrap();
        assert!(r.mic > 0.99, "{r:?}");
        write_mic(
            "mic_case2_parabola.json",
            "y=x² noiseless; MIC≈1 (Pearson≈0)",
            &r,
        );
    }

    // ---- Case 3: independent scatter → small MIC.
    {
        let n = 200usize;
        let x: Vec<f32> = (0..n).map(|i| splitmix(i as u64) as f32).collect();
        let y: Vec<f32> = (0..n).map(|i| splitmix(5000 + i as u64) as f32).collect();
        let r = mic(&x, &y).unwrap();
        assert!(r.mic < 0.5, "independent MIC small: {r:?}");
        write_mic(
            "mic_case3_independent.json",
            "independent scatter; small MIC",
            &r,
        );
    }

    // ---- Case 4: noisy monotone → high MIC, and strictly above independence.
    {
        let n = 200usize;
        let x: Vec<f32> = (0..n).map(|i| splitmix(i as u64) as f32).collect();
        let y_dep: Vec<f32> = x
            .iter()
            .enumerate()
            .map(|(i, &v)| v + 0.05 * (splitmix(9000 + i as u64) as f32 - 0.5))
            .collect();
        let y_ind: Vec<f32> = (0..n).map(|i| splitmix(5000 + i as u64) as f32).collect();
        let dep = mic(&x, &y_dep).unwrap();
        let ind = mic(&x, &y_ind).unwrap();
        assert!(dep.mic > ind.mic + 0.2, "dep>ind: {dep:?} {ind:?}");
        assert!(dep.mic > 0.7, "{dep:?}");
        write_mic(
            "mic_case4_noisy_monotone.json",
            "noisy monotone; MIC>0.7 >> independent",
            &dep,
        );
    }
}

#[test]
fn mic_fsv_edge_cases_fail_closed() {
    let mut edges = Vec::new();

    // Edge A: constant column → no grid → degenerate.
    let a = mic(&[5.0, 5.0, 5.0, 5.0, 5.0], &[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap_err();
    assert_eq!(a.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    edges.push(json!({"case": "constant_column", "after": {"code": a.code}}));

    // Edge B: below min samples (n<4) → insufficient samples.
    let b = mic(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]).unwrap_err();
    assert_eq!(b.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "below_min", "after": {"code": b.code}}));

    // Edge C: non-finite input → insufficient samples (never a silent NaN).
    let c = mic(&[1.0, f32::NAN, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0]).unwrap_err();
    assert_eq!(c.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "non_finite", "after": {"code": c.code}}));

    // Edge D: length mismatch → insufficient samples.
    let d = mic(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0]).unwrap_err();
    assert_eq!(d.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "length_mismatch", "after": {"code": d.code}}));

    let sot = json!({"suite": "mic_edge_cases", "edges": edges});
    let path = write_sot("mic_edge_cases.json", sot);
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
    println!("MIC_FSV_EDGES on-disk codes = {codes:?}");
}

// ----- FSV helpers -----------------------------------------------------------

fn write_mic(file: &str, trigger: &str, r: &MicReport) {
    let value = json!({
        "case": file, "trigger": trigger,
        "mic": {
            "mic": r.mic, "best_nx": r.best_nx, "best_ny": r.best_ny,
            "b_budget": r.b_budget, "n_samples": r.n_samples,
        },
    });
    let path = write_sot(file, value);
    let round = read_json(&path);
    let m = round["mic"]["mic"].as_f64().unwrap() as f32;
    assert!((m - r.mic).abs() <= 1e-6, "readback MIC drift {file}");
    println!(
        "MIC_FSV {} MIC={m} grid={}x{} B={} blake3={}",
        path.display(),
        r.best_nx,
        r.best_ny,
        r.b_budget,
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
    std::env::temp_dir().join("calyx_mic_fsv").join(file_name)
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
