//! Full-State-Verification for #59 empirical copula tail/co-movement measures.
//!
//! Source of truth: one JSON report under CALYX_ISSUE059_FSV_ROOT, then a
//! separate readback that re-checks the expected dependence/tail values.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    DEFAULT_TAIL_Q, empirical_copula_tail_dependence, empirical_copula_tail_dependence_with_q,
};
use serde_json::{Value, json};

#[test]
fn issue059_copula_tail_fsv_writes_and_reads_back() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let report_path = root.join("issue059_copula_tail_fsv_report.json");
    let before = file_state(&report_path);

    let (mono_x, mono_y) = comonotonic_pair(100);
    let comonotonic = empirical_copula_tail_dependence(&mono_x, &mono_y).unwrap();
    assert!(comonotonic.blomqvist_beta > 0.99, "{comonotonic:?}");
    assert!(comonotonic.gini_gamma > 0.95, "{comonotonic:?}");
    assert_eq!(comonotonic.lower_tail_lambda, 1.0);
    assert_eq!(comonotonic.upper_tail_lambda, 1.0);

    let (anti_x, anti_y) = countermonotonic_pair(100);
    let countermonotonic = empirical_copula_tail_dependence(&anti_x, &anti_y).unwrap();
    assert!(
        countermonotonic.blomqvist_beta < -0.99,
        "{countermonotonic:?}"
    );
    assert!(countermonotonic.gini_gamma < -0.90, "{countermonotonic:?}");
    assert_eq!(countermonotonic.lower_tail_lambda, 0.0);
    assert_eq!(countermonotonic.upper_tail_lambda, 0.0);

    let (weak_x, weak_y) = weak_scatter_pair(100);
    let weak = empirical_copula_tail_dependence(&weak_x, &weak_y).unwrap();
    assert!(weak.blomqvist_beta.abs() < 0.30, "{weak:?}");
    assert!(weak.gini_gamma.abs() < 0.35, "{weak:?}");
    assert!(weak.lower_tail_lambda < 0.40, "{weak:?}");
    assert!(weak.upper_tail_lambda < 0.40, "{weak:?}");

    let edges = edge_readbacks();

    let report = json!({
        "schema": "poly.issue059.copula_tail_fsv.v1",
        "proof_claim": "Empirical rank copula measures detect positive tail co-movement, negative/countermonotonic co-tail absence, weak scatter, and fail closed on invalid inputs.",
        "scope": "Empirical/rank copula summaries for continuous paired samples; no parametric copula fitting.",
        "source_of_truth": {
            "path": report_path.to_string_lossy(),
            "before": before,
        },
        "external_references_checked": [
            "https://search.r-project.org/CRAN/refmans/copBasic/help/blomCOP.html",
            "https://search.r-project.org/CRAN/refmans/copBasic/help/giniCOP.html",
            "https://rdrr.io/cran/copula/man/empCopula.html"
        ],
        "minimum_sufficient_corpus": {
            "samples_per_case": 100,
            "tail_q": DEFAULT_TAIL_Q,
            "edge_cases": edges.len(),
            "why_smaller_insufficient": "Needs positive, countermonotonic, weak baseline, and fail-closed boundaries.",
            "why_larger_wasteful": "Larger samples exercise the same rank transform, empirical copula, tail-count, write, readback, and edge paths without adding proof."
        },
        "comonotonic": comonotonic,
        "countermonotonic": countermonotonic,
        "weak_scatter": weak,
        "edge_cases": edges,
    });
    let bytes = serde_json::to_vec_pretty(&report).unwrap();
    fs::write(&report_path, &bytes).unwrap();
    assert_eq!(fs::read(&report_path).unwrap(), bytes);

    let readback = read_json(&report_path);
    assert_eq!(readback["comonotonic"]["lower_tail_lambda"], json!(1.0));
    assert_eq!(readback["comonotonic"]["upper_tail_lambda"], json!(1.0));
    assert_eq!(
        readback["countermonotonic"]["lower_tail_lambda"],
        json!(0.0)
    );
    assert_eq!(
        readback["countermonotonic"]["upper_tail_lambda"],
        json!(0.0)
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
        "ISSUE059_FSV path={} blake3={} beta_pos={} beta_neg={} weak_beta={} edges={}",
        report_path.display(),
        digest,
        readback["comonotonic"]["blomqvist_beta"],
        readback["countermonotonic"]["blomqvist_beta"],
        readback["weak_scatter"]["blomqvist_beta"],
        readback["edge_cases"].as_array().unwrap().len()
    );
}

fn edge_readbacks() -> Vec<Value> {
    let mismatch = empirical_copula_tail_dependence(&[1.0; 20], &[1.0; 19]).unwrap_err();
    assert_eq!(mismatch.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let short = empirical_copula_tail_dependence(&series(19), &series(19)).unwrap_err();
    assert_eq!(short.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let non_finite = {
        let mut x = series(20);
        x[3] = f64::NAN;
        empirical_copula_tail_dependence(&x, &series(20)).unwrap_err()
    };
    assert_eq!(non_finite.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let ties = empirical_copula_tail_dependence(&[1.0; 20], &series(20)).unwrap_err();
    assert_eq!(ties.code, "CALYX_ASSAY_DEGENERATE_INPUT");

    let bad_q =
        empirical_copula_tail_dependence_with_q(&series(20), &reverse_series(20), 0.5).unwrap_err();
    assert_eq!(bad_q.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    vec![
        json!({
            "case": "length_mismatch",
            "before": {"x_len": 20, "y_len": 19},
            "after": {"code": mismatch.code},
        }),
        json!({
            "case": "below_min_samples",
            "before": {"n": 19},
            "after": {"code": short.code},
        }),
        json!({
            "case": "non_finite",
            "before": {"x[3]": "NaN"},
            "after": {"code": non_finite.code},
        }),
        json!({
            "case": "tied_margin",
            "before": {"x": "constant"},
            "after": {"code": ties.code},
        }),
        json!({
            "case": "bad_tail_q",
            "before": {"tail_q": 0.5},
            "after": {"code": bad_q.code},
        }),
    ]
}

fn comonotonic_pair(n: usize) -> (Vec<f64>, Vec<f64>) {
    let x = series(n);
    (x.clone(), x)
}

fn countermonotonic_pair(n: usize) -> (Vec<f64>, Vec<f64>) {
    (series(n), reverse_series(n))
}

fn weak_scatter_pair(n: usize) -> (Vec<f64>, Vec<f64>) {
    let x = series(n);
    let y = (0..n).map(|i| ((i * 37 + 11) % n) as f64 + 0.25).collect();
    (x, y)
}

fn series(n: usize) -> Vec<f64> {
    (0..n).map(|i| i as f64 + 1.0).collect()
}

fn reverse_series(n: usize) -> Vec<f64> {
    (0..n).rev().map(|i| i as f64 + 1.0).collect()
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE059_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx_issue059_copula_tail_fsv"))
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
