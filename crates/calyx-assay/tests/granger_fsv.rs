//! Full-State-Verification for linear Granger causality (#60).
//!
//! Source of truth: JSON artifacts written to the OS temp dir. Each case plants a
//! synthetic series whose Granger F-test is either analytically anchored (locked
//! against an independent numpy `lstsq` OLS on the identical lagged design) or
//! carries a known directional structure, computes the estimator, writes the
//! result to disk, then performs a *separate read* and re-checks the numbers +
//! blake3 digest. Boundary cases prove the estimator fails closed.
//!
//! Run with `--nocapture` to see the evidence log of on-disk artifacts.

// Synthetic series use index-based AR recurrences (y[t] from y[t-1], x[t-1]).
#![allow(clippy::needless_range_loop)]

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    GrangerReport, granger_causality, granger_causality_lags, granger_causality_sweep,
};
use serde_json::{Value, json};

/// Deterministic splitmix64 → uniform f64 in [0,1); reproducible noise, no RNG.
fn splitmix(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64) / ((1_u64 << 53) as f64)
}

#[test]
fn granger_fsv_writes_and_reads_back_all_planted_cases() {
    // ---- Case 1: F-statistic locked to an independent numpy OLS.
    // x,y below at p=1: T=11, RSS_r=96.909059, RSS_u=1.330807, df=(1,8),
    // F=574.5581, p=9.78e-9.
    {
        let x = [
            1.0f32, 3.0, 2.0, 5.0, 4.0, 7.0, 6.0, 9.0, 8.0, 11.0, 10.0, 13.0,
        ];
        let y = [
            2.0f32, 1.0, 5.0, 4.0, 8.0, 6.0, 11.0, 9.0, 14.0, 12.0, 17.0, 15.0,
        ];
        let g = granger_causality_lags(&x, &y, 1).unwrap();
        assert_eq!((g.df_num, g.df_den, g.n_used), (1, 8, 11));
        assert!((g.rss_restricted - 96.909_06).abs() < 1e-2, "{g:?}");
        assert!((g.rss_unrestricted - 1.330_807).abs() < 1e-3, "{g:?}");
        assert!((g.f_statistic - 574.558).abs() / 574.558 < 1e-3, "{g:?}");
        assert!(g.p_value < 1e-6, "{g:?}");
        write_granger(
            "granger_case1_locked_ols.json",
            "F=574.5581 vs numpy lstsq",
            &g,
        );
    }

    // ---- Case 2: directional asymmetry. y_t=0.5 y_{t-1}+1.5 x_{t-1}+noise.
    // X→Y significant; Y→X not — the arrow points the right way.
    {
        let n = 120;
        let mut x = vec![0.0f32; n];
        let mut y = vec![0.0f32; n];
        for t in 0..n {
            x[t] = (splitmix(t as u64) - 0.5) as f32;
        }
        for t in 1..n {
            let noise = (splitmix(1000 + t as u64) - 0.5) * 0.2;
            y[t] = 0.5 * y[t - 1] + 1.5 * x[t - 1] + noise as f32;
        }
        let fwd = granger_causality(&x, &y).unwrap();
        let rev = granger_causality(&y, &x).unwrap();
        assert!(fwd.p_value < 0.001, "X→Y significant: {fwd:?}");
        assert!(rev.p_value > 0.05, "Y→X insignificant: {rev:?}");
        write_granger(
            "granger_case2_forward.json",
            "planted X→Y lag-1; forward is significant",
            &fwd,
        );
        write_granger(
            "granger_case2_reverse.json",
            "same data reversed; Y→X is insignificant (asymmetry)",
            &rev,
        );
    }

    // ---- Case 3: lag sweep recovers the planted lag. y_t=0.4 y_{t-1}+1.5 x_{t-2}.
    {
        let n = 160;
        let mut x = vec![0.0f32; n];
        let mut y = vec![0.0f32; n];
        for t in 0..n {
            x[t] = (splitmix(t as u64) - 0.5) as f32;
        }
        for t in 2..n {
            let noise = (splitmix(2000 + t as u64) - 0.5) * 0.15;
            y[t] = 0.4 * y[t - 1] + 1.5 * x[t - 2] + noise as f32;
        }
        let best = granger_causality_sweep(&x, &y).unwrap();
        assert_eq!(best.lags, 2, "sweep recovers planted lag 2: {best:?}");
        assert!(best.p_value < 0.001, "{best:?}");
        write_granger(
            "granger_case3_sweep_lag2.json",
            "sweep picks planted lag-2",
            &best,
        );
    }

    // ---- Case 4: independent series → insignificant (guards false positives).
    {
        let n = 100;
        let mut x = vec![0.0f32; n];
        let mut y = vec![0.0f32; n];
        for t in 0..n {
            x[t] = (splitmix(t as u64) - 0.5) as f32;
            y[t] = (splitmix(7777 + t as u64) - 0.5) as f32;
        }
        let g = granger_causality(&x, &y).unwrap();
        assert!(g.p_value > 0.05, "independent → insignificant: {g:?}");
        write_granger(
            "granger_case4_independent.json",
            "independent noise; insignificant",
            &g,
        );
    }
}

#[test]
fn granger_fsv_edge_cases_fail_closed() {
    let mut edges = Vec::new();

    // Edge A: zero lags → insufficient samples.
    let a = granger_causality_lags(&[1.0, 2.0, 3.0, 4.0, 5.0], &[1.0, 2.0, 3.0, 4.0, 5.0], 0)
        .unwrap_err();
    assert_eq!(a.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "zero_lags", "before": {"lags": 0}, "after": {"code": a.code}}));

    // Edge B: below minimum samples (p=1 needs n≥5) → insufficient samples.
    let b = granger_causality_lags(&[1.0, 2.0, 3.0, 4.0], &[2.0, 1.0, 4.0, 3.0], 1).unwrap_err();
    assert_eq!(b.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(
        json!({"case": "below_min", "before": {"n": 4, "min": 5}, "after": {"code": b.code}}),
    );

    // Edge C: constant regressor → rank-deficient design → degenerate.
    let x: Vec<f32> = (0..20).map(|i| (i % 3) as f32).collect();
    let yc = vec![4.0f32; 20];
    let c = granger_causality(&x, &yc).unwrap_err();
    assert_eq!(c.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    edges.push(json!({"case": "constant_regressor", "before": {"y": "const 4"}, "after": {"code": c.code}}));

    // Edge D: non-finite input → insufficient samples (never a silent NaN).
    let mut xn = vec![1.0f32; 20];
    xn[5] = f32::NAN;
    let yn: Vec<f32> = (0..20).map(|i| i as f32).collect();
    let d = granger_causality(&xn, &yn).unwrap_err();
    assert_eq!(d.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(json!({"case": "non_finite", "before": {"x[5]": "NaN"}, "after": {"code": d.code}}));

    // Edge E: length mismatch → insufficient samples.
    let e = granger_causality(&[1.0, 2.0, 3.0, 4.0, 5.0], &[1.0, 2.0, 3.0]).unwrap_err();
    assert_eq!(e.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    edges.push(
        json!({"case": "length_mismatch", "before": {"x": 5, "y": 3}, "after": {"code": e.code}}),
    );

    let sot = json!({"suite": "granger_edge_cases", "edges": edges});
    let path = write_sot("granger_edge_cases.json", sot);
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
    println!("GRANGER_FSV_EDGES on-disk codes = {codes:?}");
}

// ----- FSV helpers -----------------------------------------------------------

fn write_granger(file: &str, trigger: &str, g: &GrangerReport) {
    let value = json!({
        "case": file, "trigger": trigger,
        "granger": {
            "f_statistic": g.f_statistic, "p_value": g.p_value, "lags": g.lags,
            "df_num": g.df_num, "df_den": g.df_den,
            "rss_restricted": g.rss_restricted, "rss_unrestricted": g.rss_unrestricted,
            "n_used": g.n_used,
        },
    });
    let path = write_sot(file, value);
    let round = read_json(&path);
    let f_back = round["granger"]["f_statistic"].as_f64().unwrap() as f32;
    assert!(
        (f_back - g.f_statistic).abs() <= 1e-3 * g.f_statistic.abs().max(1.0),
        "readback drift {file}"
    );
    println!(
        "GRANGER_FSV {} F={f_back} lag={} p={} blake3={}",
        path.display(),
        g.lags,
        g.p_value,
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
        .join("calyx_granger_fsv")
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
