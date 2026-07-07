//! Full-State-Verification for #61 variable-lag transfer entropy + Granger.
//!
//! Source of truth: a JSON report written under CALYX_ISSUE061_FSV_ROOT, then
//! separately read back and re-checked. The proof corpus is intentionally small:
//! one planted lag-2 transfer-entropy stream, one planted lag-2 Granger series,
//! and explicit fail-closed edges.

#![allow(clippy::needless_range_loop)]

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    CALYX_TE_INSUFFICIENT_SAMPLES, DEFAULT_GRANGER_LAG_SWEEP, DEFAULT_TE_LAGS, Direction,
    TransferEntropyConfig, granger_causality, granger_causality_sweep_lags,
    max_transfer_entropy_lag, transfer_entropy, transfer_entropy_sweep_with_config,
    transfer_entropy_with_config,
};
use calyx_core::FixedClock;
use serde_json::{Value, json};

type TestStream = Vec<(u64, f32)>;

#[test]
fn issue061_variable_lag_fsv_writes_and_reads_back_known_truth() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let report_path = root.join("issue061_variable_lag_fsv_report.json");
    let before = file_state(&report_path);

    let proof_claim = "TE and Granger lag sweeps recover the planted lag-2 driver \
        and fail closed on invalid or underpowered inputs.";
    let lags = [1usize, 2, 4, 8];

    let te_config = TransferEntropyConfig {
        bootstrap_resamples: 20,
        ..TransferEntropyConfig::default()
    };
    let (a, b) = planted_te_a_to_b(140, 2);
    let te_lag2 = transfer_entropy_with_config(&a, &b, 2, &clock(), &te_config).unwrap();
    let te_sweep = transfer_entropy_sweep_with_config(&a, &b, &lags, &clock(), &te_config);
    let te_best = max_transfer_entropy_lag(&te_sweep);
    assert_eq!(te_best, Some(2), "TE sweep must find lag 2: {te_sweep:?}");
    assert_eq!(te_lag2.dominant_direction, Direction::AToB);
    assert!(!te_lag2.provisional);
    assert!(
        te_lag2.t_a_to_b > te_lag2.t_b_to_a + 0.1,
        "TE direction margin must be decisive: {te_lag2:?}"
    );

    let (x, y) = planted_granger_x_to_y_lag2(160);
    let granger_best = granger_causality_sweep_lags(&x, &y, &lags).unwrap();
    assert_eq!(
        granger_best.lags, 2,
        "Granger sweep must find lag 2: {granger_best:?}"
    );
    assert!(
        granger_best.p_value < 0.001,
        "planted Granger lag must be significant: {granger_best:?}"
    );

    let edges = edge_readbacks();
    assert!(edges.len() >= 3);

    let report = json!({
        "schema": "poly.issue061.variable_lag_fsv.v1",
        "proof_claim": proof_claim,
        "source_of_truth": {
            "path": report_path.to_string_lossy(),
            "before": before,
        },
        "minimum_sufficient_corpus": {
            "te_stream_samples_per_side": a.len(),
            "granger_samples_per_series": x.len(),
            "candidate_lags": lags,
            "why_smaller_insufficient": "Needs both TE and Granger planted lag recovery plus explicit boundary edges.",
            "why_larger_wasteful": "Larger streams exercise the same sweep, estimator, write, readback, and fail-closed paths without adding proof."
        },
        "defaults_checked": {
            "te_default_lags": DEFAULT_TE_LAGS,
            "granger_default_lags": DEFAULT_GRANGER_LAG_SWEEP,
        },
        "transfer_entropy": {
            "known_truth": "A drives B at lag 2",
            "selected_lag": te_best,
            "lag2": {
                "t_a_to_b": te_lag2.t_a_to_b,
                "t_b_to_a": te_lag2.t_b_to_a,
                "dominant_direction": format!("{:?}", te_lag2.dominant_direction),
                "difference_ci_95": te_lag2.difference_ci_95,
                "n_samples": te_lag2.n_samples,
                "provisional": te_lag2.provisional,
                "error_code": te_lag2.error_code,
            },
            "sweep": te_sweep,
        },
        "granger": {
            "known_truth": "X drives Y at lag 2",
            "selected_lag": granger_best.lags,
            "f_statistic": granger_best.f_statistic,
            "p_value": granger_best.p_value,
            "df_num": granger_best.df_num,
            "df_den": granger_best.df_den,
            "n_used": granger_best.n_used,
            "rss_restricted": granger_best.rss_restricted,
            "rss_unrestricted": granger_best.rss_unrestricted,
        },
        "edge_cases": edges,
    });
    let bytes = serde_json::to_vec_pretty(&report).unwrap();
    fs::write(&report_path, &bytes).unwrap();
    assert_eq!(fs::read(&report_path).unwrap(), bytes);

    let readback = read_json(&report_path);
    assert_eq!(readback["transfer_entropy"]["selected_lag"], json!(2));
    assert_eq!(
        readback["transfer_entropy"]["lag2"]["dominant_direction"],
        "AToB"
    );
    assert_eq!(readback["granger"]["selected_lag"], json!(2));
    assert!(
        readback["edge_cases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["after"]["code"] == "CALYX_ASSAY_DEGENERATE_INPUT")
    );
    assert!(
        readback["edge_cases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["after"]["code"] == "CALYX_ASSAY_INSUFFICIENT_SAMPLES")
    );

    let digest = blake3::hash(&fs::read(&report_path).unwrap());
    println!(
        "ISSUE061_FSV path={} blake3={} te_lag={:?} granger_lag={} edges={}",
        report_path.display(),
        digest,
        te_best,
        granger_best.lags,
        readback["edge_cases"].as_array().unwrap().len()
    );
}

fn edge_readbacks() -> Vec<Value> {
    let short = vec![(0, 0.1), (1, 0.2)];
    let underpowered_te = transfer_entropy(&short, &short, 1, &clock()).unwrap();
    assert!(underpowered_te.provisional);
    assert_eq!(
        underpowered_te.error_code.as_deref(),
        Some(CALYX_TE_INSUFFICIENT_SAMPLES)
    );

    let duplicate = vec![(1, 0.1), (1, 0.2)];
    let duplicate_error = transfer_entropy(&duplicate, &duplicate, 1, &clock()).unwrap_err();
    assert_eq!(duplicate_error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let empty_lag_error =
        granger_causality_sweep_lags(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0], &[]).unwrap_err();
    assert_eq!(empty_lag_error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let constant_y = vec![4.0f32; 20];
    let varying_x: Vec<f32> = (0..20).map(|i| (i % 3) as f32).collect();
    let degenerate = granger_causality(&varying_x, &constant_y).unwrap_err();
    assert_eq!(degenerate.code, "CALYX_ASSAY_DEGENERATE_INPUT");

    vec![
        json!({
            "case": "underpowered_te",
            "before": {"samples_per_stream": short.len(), "lag": 1},
            "after": {
                "provisional": underpowered_te.provisional,
                "code": underpowered_te.error_code,
                "n_samples": underpowered_te.n_samples,
            },
        }),
        json!({
            "case": "duplicate_te_timestamp",
            "before": {"timestamps": [1, 1]},
            "after": {"code": duplicate_error.code},
        }),
        json!({
            "case": "empty_granger_lag_set",
            "before": {"lags": []},
            "after": {"code": empty_lag_error.code},
        }),
        json!({
            "case": "constant_granger_target",
            "before": {"y": "constant"},
            "after": {"code": degenerate.code},
        }),
    ]
}

fn planted_te_a_to_b(n: usize, lag: usize) -> (TestStream, TestStream) {
    let a = simple_stream(n, 7);
    let mut b = Vec::with_capacity(n);
    for t in 0..n {
        let value = if t >= lag {
            let driver = a[t - lag].1;
            driver + 0.01 * (noise(t as u64, 41) - 0.5)
        } else {
            noise(t as u64, 73)
        };
        b.push((t as u64, value));
    }
    (a, b)
}

fn planted_granger_x_to_y_lag2(n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut x = vec![0.0f32; n];
    let mut y = vec![0.0f32; n];
    for t in 0..n {
        x[t] = (splitmix(t as u64) - 0.5) as f32;
    }
    for t in 2..n {
        let eps = (splitmix(2000 + t as u64) - 0.5) * 0.15;
        y[t] = 0.4 * y[t - 1] + 1.5 * x[t - 2] + eps as f32;
    }
    (x, y)
}

fn simple_stream(n: usize, salt: u64) -> TestStream {
    (0..n)
        .map(|t| (t as u64, 0.2 + 0.6 * noise(t as u64, salt)))
        .collect()
}

fn noise(t: u64, salt: u64) -> f32 {
    splitmix(t ^ salt.rotate_left(17)) as f32
}

fn splitmix(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64) / ((1_u64 << 53) as f64)
}

fn clock() -> FixedClock {
    FixedClock::new(1_786_100_061)
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE061_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx_issue061_variable_lag_fsv"))
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
