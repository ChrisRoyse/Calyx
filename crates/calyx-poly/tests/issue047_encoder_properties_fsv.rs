//! Issue #47 - encoder property FSV.
//!
//! Source of truth: a persisted encoder property report read back from disk.

use std::fs;

use calyx_poly::encode::{QuantileEncoder, RffEncoder, one_hot};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const RFF_SEED: u64 = 47;
const RFF_DIM: usize = 4096;
const RFF_SIGMA: f64 = 0.20;
const RFF_TOLERANCE: f64 = 0.03;
const RFF_POINTS: [f64; 4] = [-0.30, -0.10, 0.10, 0.30];
const QUANTILE_VALUES: [f64; 4] = [0.0, 5.0, 50.0, 500.0];

#[test]
fn issue047_encoder_property_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE047_FSV_ROOT", "poly-issue047-encoders");
    reset_dir(&root);

    let report = json!({
        "issue": 47,
        "proof_claim": "RFF cosine approximates the true RBF kernel within a fixed bound, encoder output is deterministic, quantile fill is monotone, and edge inputs produce explicit absent/zero vectors.",
        "minimum_sufficient_corpus": {
            "rff_scalar_points": RFF_POINTS.len(),
            "rff_pairwise_kernel_checks": RFF_POINTS.len() * RFF_POINTS.len(),
            "quantile_scalar_values": QUANTILE_VALUES.len(),
            "edge_probes": 3,
            "why_this_is_sufficient": "Four scalar points are the smallest grid used here that proves identity, near, medium, far, negative, positive, and symmetric RFF/RBF relationships; four quantile values prove low, in-bin, cross-bin, and saturated monotone fill; three edge probes cover non-finite RFF, non-finite quantile, and out-of-range categorical encoding.",
            "why_smaller_is_insufficient": "Fewer RFF points would drop either the far-pair error bound or sign-symmetry coverage, fewer quantile values would miss a fill regime, and fewer edges would leave one documented absent-vector path unproven.",
            "why_larger_is_wasteful": "More scalar points or larger market datasets would repeat the same deterministic encoder math; #47 is a bounded encoder property claim, not a corpus-scale claim."
        },
        "happy_path": {
            "rff_kernel_matrix": rff_kernel_matrix_report(),
            "quantile_monotonicity": quantile_monotonicity_report()
        },
        "edge_cases": {
            "rff_nonfinite_zero_vector": edge_rff_nonfinite_zero_vector(),
            "quantile_nonfinite_zero_vector": edge_quantile_nonfinite_zero_vector(),
            "one_hot_out_of_range_zero_vector": edge_one_hot_out_of_range_zero_vector()
        }
    });

    let report_path = root.join("encoder_property_report.json");
    write_json(&report_path, &report);
    let report_bytes = fs::read(&report_path).expect("read encoder property report");
    assert_eq!(
        report_bytes,
        serde_json::to_vec_pretty(&report).expect("encode encoder property report")
    );
    let readback: Value =
        serde_json::from_slice(&report_bytes).expect("decode encoder property report");

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback_summary = json!({
        "issue": 47,
        "source_of_truth": report_path.display().to_string(),
        "report_read_back_matches_written": true,
        "report": readback,
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback_summary);
    write_blake3sums(&root);
    println!(
        "ISSUE047_ENCODER_PROPERTIES_READBACK={}",
        readback_path.display()
    );
}

fn rff_kernel_matrix_report() -> Value {
    let encoder = RffEncoder::new(RFF_SEED, RFF_DIM, RFF_SIGMA);
    let vectors: Vec<_> = RFF_POINTS
        .iter()
        .map(|point| encoder.encode(*point))
        .collect();
    let repeat = RffEncoder::new(RFF_SEED, RFF_DIM, RFF_SIGMA);
    let mut rows = Vec::new();
    let mut max_abs_error = 0.0_f64;

    for (i, x) in RFF_POINTS.iter().enumerate() {
        assert_eq!(vectors[i], repeat.encode(*x), "RFF determinism for {x}");
        for (j, y) in RFF_POINTS.iter().enumerate() {
            let observed = cosine(&vectors[i], &vectors[j]);
            let expected = true_rbf(*x, *y, RFF_SIGMA);
            let abs_error = (observed - expected).abs();
            max_abs_error = max_abs_error.max(abs_error);
            assert!(
                abs_error <= RFF_TOLERANCE,
                "RFF pair ({x},{y}) observed={observed} expected={expected} error={abs_error}"
            );
            rows.push(json!({
                "x": x,
                "y": y,
                "observed_cosine": observed,
                "expected_rbf": expected,
                "abs_error": abs_error
            }));
        }
    }

    json!({
        "seed": RFF_SEED,
        "dim": RFF_DIM,
        "sigma": RFF_SIGMA,
        "tolerance": RFF_TOLERANCE,
        "max_abs_error": max_abs_error,
        "points": RFF_POINTS,
        "vector_hashes": RFF_POINTS.iter().zip(vectors.iter()).map(|(point, vector)| json!({
            "point": point,
            "blake3": vector_hash(vector),
            "len": vector.len()
        })).collect::<Vec<_>>(),
        "pairs": rows
    })
}

fn quantile_monotonicity_report() -> Value {
    let encoder = QuantileEncoder::new(vec![0.0, 10.0, 100.0, 1000.0]);
    let mut last_fill = f64::NEG_INFINITY;
    let mut rows = Vec::new();
    for value in QUANTILE_VALUES {
        let encoded = encoder.encode(value);
        let fill_sum = encoded[..encoded.len() - 1]
            .iter()
            .map(|x| *x as f64)
            .sum::<f64>();
        assert!(
            fill_sum >= last_fill,
            "quantile fill must be monotone: value={value} fill={fill_sum} last={last_fill}"
        );
        last_fill = fill_sum;
        rows.push(json!({
            "value": value,
            "fill_sum_excluding_bias": fill_sum,
            "vector_hash": vector_hash(&encoded)
        }));
    }

    let canonical = encoder.encode(50.0);
    let sanitized =
        QuantileEncoder::new(vec![1000.0, 100.0, f64::NAN, 10.0, 0.0, 10.0]).encode(50.0);
    assert_eq!(
        canonical, sanitized,
        "quantile edge sanitization must be deterministic"
    );

    json!({
        "edges": [0.0, 10.0, 100.0, 1000.0],
        "values": rows,
        "unsorted_duplicate_nonfinite_edges_match_canonical": true
    })
}

fn edge_rff_nonfinite_zero_vector() -> Value {
    let vector = RffEncoder::new(RFF_SEED, 8, RFF_SIGMA).encode(f64::NAN);
    assert!(vector.iter().all(|x| *x == 0.0));
    json!({"len": vector.len(), "all_zero": true, "vector_hash": vector_hash(&vector)})
}

fn edge_quantile_nonfinite_zero_vector() -> Value {
    let vector = QuantileEncoder::new(vec![0.0, 1.0, 2.0]).encode(f64::INFINITY);
    assert!(vector.iter().all(|x| *x == 0.0));
    json!({"len": vector.len(), "all_zero": true, "vector_hash": vector_hash(&vector)})
}

fn edge_one_hot_out_of_range_zero_vector() -> Value {
    let vector = one_hot(5, 3);
    assert_eq!(vector, vec![0.0, 0.0, 0.0]);
    json!({"len": vector.len(), "all_zero": true, "vector_hash": vector_hash(&vector)})
}

fn true_rbf(x: f64, y: f64, sigma: f64) -> f64 {
    (-(x - y).powi(2) / (2.0 * sigma.powi(2))).exp()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (*x as f64) * (*y as f64))
        .sum::<f64>();
    let na = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let nb = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if na <= f64::EPSILON || nb <= f64::EPSILON {
        0.0
    } else {
        dot / (na * nb)
    }
}

fn vector_hash(data: &[f32]) -> String {
    let mut hasher = blake3::Hasher::new();
    for value in data {
        hasher.update(&value.to_le_bytes());
    }
    hex(hasher.finalize().as_bytes())
}
