//! Full-State-Verification for rank correlation (#57).
//!
//! Source of truth: JSON artifacts written to the OS temp dir (and mirrored to
//! `CALYX_FSV_ROOT` when set). Every case plants a synthetic series whose exact
//! Spearman ρ / Kendall τ-b is hand-derivable, computes the estimator, writes
//! the result to disk, then performs a *separate read* of the file and re-checks
//! the numbers + blake3 digest. Boundary cases prove the estimators fail closed.
//!
//! Run with `--nocapture` to see the evidence log of on-disk artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{KendallReport, SpearmanReport, kendall_tau_b, spearman_rho};
use serde_json::{Value, json};

const TOL: f32 = 1e-5;

#[test]
fn rank_correlation_fsv_writes_and_reads_back_all_planted_cases() {
    // ---- Case 1: perfect monotone — ρ = τ = 1, both maximally significant.
    {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        let y = [3.0f32, 9.0, 27.0, 81.0, 243.0, 729.0]; // 3^i: strictly ↑, non-linear
        let s = spearman_rho(&x, &y).unwrap();
        let k = kendall_tau_b(&x, &y).unwrap();
        assert!((s.rho - 1.0).abs() <= TOL, "{s:?}");
        assert!((k.tau_b - 1.0).abs() <= TOL, "{k:?}");
        assert_eq!(k.n_discordant, 0);
        assert_eq!(k.n_concordant, 15); // C(6,2)
        assert!(s.p_value < 1e-6 && k.p_value < 0.05, "sig: {s:?} {k:?}");
        write_and_read_back(
            "rankcorr_case1_perfect_monotone.json",
            "3^i strictly increasing over x=1..6; exact ρ=τ=1",
            &s,
            &k,
        );
    }

    // ---- Case 2: perfect antitone — ρ = τ = -1.
    {
        let x = [10.0f32, 20.0, 30.0, 40.0, 50.0];
        let y = [5.0f32, 4.0, 3.0, 2.0, 1.0];
        let s = spearman_rho(&x, &y).unwrap();
        let k = kendall_tau_b(&x, &y).unwrap();
        assert!((s.rho + 1.0).abs() <= TOL, "{s:?}");
        assert!((k.tau_b + 1.0).abs() <= TOL, "{k:?}");
        assert_eq!(k.n_concordant, 0);
        assert_eq!(k.n_discordant, 10);
        write_and_read_back(
            "rankcorr_case2_perfect_antitone.json",
            "strictly decreasing; exact ρ=τ=-1",
            &s,
            &k,
        );
    }

    // ---- Case 3: tie-corrected known truth (hand-derived in the module docs).
    // x=[1,2,2,4,5], y=[1,2,3,4,5]: ρ=9.5/√95=0.9746794, τ-b=9/√90=0.9486833.
    {
        let x = [1.0f32, 2.0, 2.0, 4.0, 5.0];
        let y = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let s = spearman_rho(&x, &y).unwrap();
        let k = kendall_tau_b(&x, &y).unwrap();
        assert!((s.rho - 0.974_679_4).abs() <= TOL, "{s:?}");
        assert!((k.tau_b - 0.948_683_3).abs() <= TOL, "{k:?}");
        assert_eq!(k.s_statistic, 9);
        write_and_read_back(
            "rankcorr_case3_tie_corrected.json",
            "one x-tie; hand-derived ρ=0.9746794, τ-b=0.9486833",
            &s,
            &k,
        );
    }

    // ---- Case 4: independent series — weak, insignificant (guards false +ve).
    // x=1..8, y a scatter with Spearman ρ = 1-6·88/(8·63) = -0.047619 (no ties).
    {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let y = [5.0f32, 3.0, 6.0, 2.0, 7.0, 4.0, 8.0, 1.0];
        let s = spearman_rho(&x, &y).unwrap();
        let k = kendall_tau_b(&x, &y).unwrap();
        assert!((s.rho - (-0.047_619)).abs() <= 1e-4, "{s:?}");
        assert!(
            s.p_value > 0.5 && k.p_value > 0.5,
            "weak → insig: {s:?} {k:?}"
        );
        write_and_read_back(
            "rankcorr_case4_independent.json",
            "scatter; exact ρ=-0.047619, insignificant",
            &s,
            &k,
        );
    }

    // ---- Case 5: larger planted noisy-monotone (scale/robustness) — ρ,τ high +,
    // strongly significant, CI strictly above zero. Deterministic PRNG jitter.
    {
        let n = 200usize;
        let mut x = Vec::with_capacity(n);
        let mut y = Vec::with_capacity(n);
        for i in 0..n {
            let xi = i as f32;
            // Monotone increasing signal + bounded deterministic jitter that
            // never overturns the global trend at this scale.
            let jitter = ((splitmix(i as u64) as f32) - 0.5) * 1.5;
            x.push(xi);
            y.push(0.05 * xi + jitter);
        }
        let s = spearman_rho(&x, &y).unwrap();
        let k = kendall_tau_b(&x, &y).unwrap();
        assert!(
            s.rho > 0.6 && k.tau_b > 0.45,
            "planted ↑ trend: {s:?} {k:?}"
        );
        assert!(s.p_value < 1e-6 && k.p_value < 1e-6, "sig: {s:?} {k:?}");
        assert!(s.ci_low > 0.0, "95% CI must exclude 0: {s:?}");
        write_and_read_back(
            "rankcorr_case5_noisy_monotone_n200.json",
            "n=200 planted ↑ trend + bounded jitter; ρ,τ strongly +, CI>0",
            &s,
            &k,
        );
    }

    // ---- Case 6: ties in both columns exercise the cross-tie variance term.
    // scipy.stats.kendalltau(method="asymptotic", variant="b"):
    // tau-b=0.7161148740, Var(S)=26.25, z=1.951800146, p=0.050961937.
    {
        let x = [0.0f32, 1.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0];
        let y = [0.0f32, 0.0, 0.0, 2.0, 2.0, 2.0, 2.0, 2.0];
        let s = spearman_rho(&x, &y).unwrap();
        let k = kendall_tau_b(&x, &y).unwrap();
        assert!((s.rho - 0.737_711_13).abs() <= TOL, "{s:?}");
        assert!((k.tau_b - 0.716_114_9).abs() <= TOL, "{k:?}");
        assert!((k.z_statistic - 1.951_800_1).abs() <= TOL, "{k:?}");
        assert!((k.p_value - 0.050_961_94).abs() <= TOL, "{k:?}");
        assert!(
            k.p_value > 0.05,
            "tie correction must avoid false +ve: {k:?}"
        );
        assert_eq!(k.s_statistic, 10);
        write_and_read_back(
            "rankcorr_case6_both_columns_tied.json",
            "both columns tied; scipy asymptotic p=0.050961937",
            &s,
            &k,
        );
    }
}

#[test]
fn rank_correlation_fsv_edge_cases_fail_closed() {
    // Boundary & edge audit: each records before/after (the estimator returns a
    // structured error code, so the "after" state is the code, not a value).
    let mut edges = Vec::new();

    // Edge A: constant column → correlation undefined.
    let a = spearman_rho(&[7.0, 7.0, 7.0, 7.0], &[1.0, 2.0, 3.0, 4.0]).unwrap_err();
    let a2 = kendall_tau_b(&[7.0, 7.0, 7.0, 7.0], &[1.0, 2.0, 3.0, 4.0]).unwrap_err();
    assert_eq!(a.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    assert_eq!(a2.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    edges.push(json!({
        "case": "constant_column",
        "before": {"x": [7.0,7.0,7.0,7.0], "y": [1,2,3,4]},
        "after": {"spearman_code": a.code, "kendall_code": a2.code},
    }));

    // Edge B: length mismatch → insufficient samples.
    let b = kendall_tau_b(&[1.0, 2.0, 3.0], &[1.0, 2.0]).unwrap_err();
    assert_eq!(b.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({
        "case": "length_mismatch",
        "before": {"x_len": 3, "y_len": 2},
        "after": {"code": b.code},
    }));

    // Edge C: below minimum (n<3) → insufficient samples.
    let c = spearman_rho(&[1.0, 2.0], &[9.0, 4.0]).unwrap_err();
    assert_eq!(c.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({
        "case": "below_min_samples",
        "before": {"n": 2, "min": 3},
        "after": {"code": c.code},
    }));

    // Edge D: non-finite input → insufficient samples (never a silent NaN).
    let d = spearman_rho(&[1.0, f32::INFINITY, 3.0], &[1.0, 2.0, 3.0]).unwrap_err();
    assert_eq!(d.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({
        "case": "non_finite",
        "before": {"x": ["1", "inf", "3"]},
        "after": {"code": d.code},
    }));

    let sot = json!({"suite": "rank_correlation_edge_cases", "edges": edges});
    let path = write_sot("rankcorr_edge_cases.json", sot);
    // Full-state readback: re-open the file and confirm all four codes reside.
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
    println!("RANKCORR_FSV_EDGES on-disk codes = {codes:?}");
}

// ----- FSV helpers (source-of-truth write + independent readback) ------------

fn write_and_read_back(file: &str, trigger: &str, s: &SpearmanReport, k: &KendallReport) {
    let value = json!({
        "case": file,
        "trigger": trigger,
        "spearman": {
            "rho": s.rho, "t_statistic": s.t_statistic, "p_value": s.p_value,
            "ci_low": s.ci_low, "ci_high": s.ci_high, "n_samples": s.n_samples,
        },
        "kendall": {
            "tau_b": k.tau_b, "s_statistic": k.s_statistic, "z_statistic": k.z_statistic,
            "p_value": k.p_value, "n_concordant": k.n_concordant,
            "n_discordant": k.n_discordant, "n_samples": k.n_samples,
        },
    });
    let path = write_sot(file, value);

    // Independent read of the source of truth, then re-validate the residing data.
    let round = read_json(&path);
    let rho_back = round["spearman"]["rho"].as_f64().unwrap() as f32;
    let tau_back = round["kendall"]["tau_b"].as_f64().unwrap() as f32;
    assert!(
        (rho_back - s.rho).abs() <= TOL,
        "readback ρ drift in {file}"
    );
    assert!(
        (tau_back - k.tau_b).abs() <= TOL,
        "readback τ drift in {file}"
    );
    println!(
        "RANKCORR_FSV {} ρ={rho_back} τ-b={tau_back} blake3={}",
        path.display(),
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
    // Verify the write landed byte-for-byte.
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
        .join("calyx_rankcorr_fsv")
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

/// Deterministic splitmix64 → uniform f64 in [0,1); reproducible jitter, no RNG.
fn splitmix(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64) / ((1_u64 << 53) as f64)
}
