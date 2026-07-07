//! Full-State-Verification for #62 signed cross-correlation profiles.
//!
//! Source of truth: one JSON report under CALYX_ISSUE062_FSV_ROOT, then a
//! separate readback that re-checks the peak lags and edge-case codes.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{CCF_LAG_CONVENTION, cross_correlation_profile};
use serde_json::{Value, json};

#[test]
fn issue062_cross_correlation_fsv_writes_and_reads_back_profile() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let report_path = root.join("issue062_cross_correlation_fsv_report.json");
    let before = file_state(&report_path);

    let proof_claim = "CCF computes a full signed lag profile, selects the \
        known peak lag, and fails closed on invalid or degenerate inputs.";
    let max_lag = 8usize;
    let planted_lag = 3isize;
    let (x, y) = planted_x_leads_y(96, planted_lag as usize);

    let forward = cross_correlation_profile(&x, &y, max_lag).unwrap();
    assert_eq!(forward.peak_lag, planted_lag, "{forward:?}");
    assert!(
        forward.peak_correlation > 0.99,
        "planted lag must be near-perfect: {forward:?}"
    );
    assert_eq!(forward.points.len(), 2 * max_lag + 1);

    let swapped = cross_correlation_profile(&y, &x, max_lag).unwrap();
    assert_eq!(swapped.peak_lag, -planted_lag, "{swapped:?}");
    assert!(
        swapped.peak_correlation > 0.99,
        "swapped pair must keep near-perfect magnitude: {swapped:?}"
    );

    let edges = edge_readbacks();
    assert!(edges.len() >= 3);

    let report = json!({
        "schema": "poly.issue062.cross_correlation_fsv.v1",
        "proof_claim": proof_claim,
        "source_of_truth": {
            "path": report_path.to_string_lossy(),
            "before": before,
        },
        "external_references_checked": [
            "https://www.statsmodels.org/stable/generated/statsmodels.tsa.stattools.ccf.html",
            "https://stat.ethz.ch/R-manual/R-devel/library/stats/html/acf.html"
        ],
        "minimum_sufficient_corpus": {
            "paired_samples": x.len(),
            "planted_lag": planted_lag,
            "max_lag": max_lag,
            "why_smaller_insufficient": "Needs a profile wide enough to include off-peak lags and a swapped readback to prove signed lag semantics.",
            "why_larger_wasteful": "Larger series exercise the same shifted-pair Pearson, peak selection, write, readback, and fail-closed paths without adding proof."
        },
        "lag_convention": CCF_LAG_CONVENTION,
        "forward_x_y": forward,
        "swapped_y_x": swapped,
        "edge_cases": edges,
    });
    let bytes = serde_json::to_vec_pretty(&report).unwrap();
    fs::write(&report_path, &bytes).unwrap();
    assert_eq!(fs::read(&report_path).unwrap(), bytes);

    let readback = read_json(&report_path);
    assert_eq!(readback["forward_x_y"]["peak_lag"], json!(3));
    assert_eq!(readback["swapped_y_x"]["peak_lag"], json!(-3));
    assert_eq!(
        readback["forward_x_y"]["points"].as_array().unwrap().len(),
        17
    );
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
        "ISSUE062_FSV path={} blake3={} forward_peak={} swapped_peak={} edges={}",
        report_path.display(),
        digest,
        readback["forward_x_y"]["peak_lag"],
        readback["swapped_y_x"]["peak_lag"],
        readback["edge_cases"].as_array().unwrap().len()
    );
}

fn edge_readbacks() -> Vec<Value> {
    let mismatch = cross_correlation_profile(&[1.0, 2.0, 3.0], &[1.0, 2.0], 0).unwrap_err();
    assert_eq!(mismatch.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let too_wide =
        cross_correlation_profile(&[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 1.0, 4.0, 3.0, 6.0], 3)
            .unwrap_err();
    assert_eq!(too_wide.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let non_finite =
        cross_correlation_profile(&[1.0, f32::NAN, 3.0], &[1.0, 2.0, 3.0], 0).unwrap_err();
    assert_eq!(non_finite.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let constant =
        cross_correlation_profile(&[4.0, 4.0, 4.0, 4.0], &[1.0, 2.0, 3.0, 4.0], 1).unwrap_err();
    assert_eq!(constant.code, "CALYX_ASSAY_DEGENERATE_INPUT");

    vec![
        json!({
            "case": "length_mismatch",
            "before": {"x_len": 3, "y_len": 2, "max_lag": 0},
            "after": {"code": mismatch.code},
        }),
        json!({
            "case": "max_lag_too_wide",
            "before": {"n": 5, "max_lag": 3, "boundary_pairs": 2},
            "after": {"code": too_wide.code},
        }),
        json!({
            "case": "non_finite",
            "before": {"x": ["1", "NaN", "3"]},
            "after": {"code": non_finite.code},
        }),
        json!({
            "case": "constant_series",
            "before": {"x": [4, 4, 4, 4]},
            "after": {"code": constant.code},
        }),
    ]
}

fn planted_x_leads_y(n: usize, lag: usize) -> (Vec<f32>, Vec<f32>) {
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    for t in 0..n {
        x.push((splitmix(t as u64 * 17 + 11) - 0.5) as f32);
    }
    for t in 0..n {
        let value = if t >= lag {
            x[t - lag] + 0.001 * (splitmix(10_000 + t as u64) - 0.5) as f32
        } else {
            (splitmix(20_000 + t as u64) - 0.5) as f32
        };
        y.push(value);
    }
    (x, y)
}

fn splitmix(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64) / ((1_u64 << 53) as f64)
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE062_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx_issue062_cross_correlation_fsv"))
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
