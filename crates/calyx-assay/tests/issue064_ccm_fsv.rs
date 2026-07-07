//! Full-State-Verification for #64 Convergent Cross Mapping.
//!
//! Source of truth: one JSON report under CALYX_ISSUE064_FSV_ROOT, then a
//! separate readback that re-checks directional skill and edge-case codes.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{CcmConfig, CcmVerdict, convergent_cross_mapping};
use serde_json::{Value, json};

#[test]
fn issue064_ccm_fsv_writes_and_reads_back_direction() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let report_path = root.join("issue064_ccm_fsv_report.json");
    let before = file_state(&report_path);

    let (x, y) = coupled_logistic_series(230, 50, 0.20);
    let config = CcmConfig::new(3, 1, vec![40, 80, 120, 170], 0.05, 0.10);
    let report = convergent_cross_mapping("x", &x, "y", &y, &config).unwrap();
    assert_eq!(report.verdict, CcmVerdict::XCausesY, "{report:?}");
    assert!(report.y_manifold_to_x.final_rho > 0.95, "{report:?}");
    assert!(
        report.y_manifold_to_x.convergence_delta > 0.09,
        "{report:?}"
    );
    assert!(
        report.y_manifold_to_x.final_rho - report.x_manifold_to_y.final_rho > 0.25,
        "{report:?}"
    );
    assert!(
        report
            .y_manifold_to_x
            .library_skills
            .windows(2)
            .all(|pair| pair[1].rho > pair[0].rho),
        "{report:?}"
    );

    let edge_cases = edge_readbacks();
    let body = json!({
        "schema": "poly.issue064.ccm_fsv.v1",
        "proof_claim": "Scalar CCM detects that x drives y when y's shadow manifold increasingly cross-maps x better than x's manifold cross-maps y.",
        "scope": "Scalar deterministic-ish CCM with delay embeddings and leave-one-out simplex cross mapping only; not a general nonlinear causal discovery stack.",
        "source_of_truth": {
            "path": report_path.to_string_lossy(),
            "before": before,
        },
        "minimum_sufficient_corpus": {
            "generator": "unidirectionally coupled logistic system",
            "raw_samples": 230,
            "burn_in": 50,
            "samples": x.len(),
            "effective_points": report.effective_points,
            "coupling": 0.20,
            "embedding_dim": 3,
            "tau": 1,
            "library_sizes": config.library_sizes,
            "min_convergence_delta": 0.05,
            "min_skill_gap": 0.10,
            "why_smaller_insufficient": "The directional proof requires increasing library sizes through 170 effective points; smaller corpora either cannot run the largest library or reduce convergence margin.",
            "why_larger_wasteful": "Larger corpora would exercise the same embedding, nearest-neighbor, simplex weighting, convergence, write, and readback paths without adding proof."
        },
        "ccm": report,
        "edge_cases": edge_cases,
    });
    let bytes = serde_json::to_vec_pretty(&body).unwrap();
    fs::write(&report_path, &bytes).unwrap();
    assert_eq!(fs::read(&report_path).unwrap(), bytes);

    let readback = read_json(&report_path);
    assert_eq!(readback["ccm"]["verdict"], "XCausesY");
    assert!(
        readback["ccm"]["y_manifold_to_x"]["final_rho"]
            .as_f64()
            .unwrap()
            > 0.95
    );
    assert!(
        readback["ccm"]["y_manifold_to_x"]["convergence_delta"]
            .as_f64()
            .unwrap()
            > 0.09
    );
    assert!(
        readback["ccm"]["y_manifold_to_x"]["final_rho"]
            .as_f64()
            .unwrap()
            - readback["ccm"]["x_manifold_to_y"]["final_rho"]
                .as_f64()
                .unwrap()
            > 0.25
    );
    let codes: Vec<&str> = readback["edge_cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["after"]["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"CALYX_ASSAY_INSUFFICIENT_SAMPLES"));
    assert!(codes.contains(&"CALYX_ASSAY_DEGENERATE_INPUT"));

    let digest = blake3::hash(&fs::read(&report_path).unwrap());
    println!(
        "ISSUE064_FSV path={} blake3={} verdict={:?} y_to_x_final={} x_to_y_final={} edge_cases={}",
        report_path.display(),
        digest,
        readback["ccm"]["verdict"],
        readback["ccm"]["y_manifold_to_x"]["final_rho"],
        readback["ccm"]["x_manifold_to_y"]["final_rho"],
        readback["edge_cases"].as_array().unwrap().len()
    );
}

fn edge_readbacks() -> Vec<Value> {
    let x = [0.1f32, 0.2, 0.4, 0.8, 0.3, 0.6, 0.7, 0.5];
    let y = [0.2f32, 0.3, 0.5, 0.7, 0.4, 0.9, 0.6, 0.1];
    let short = [0.2f32, 0.3, 0.5, 0.7, 0.4, 0.9, 0.6];
    let config = CcmConfig::new(2, 1, vec![5, 6], 0.05, 0.05);

    let mismatch = convergent_cross_mapping("x", &x, "y", &short, &config).unwrap_err();
    assert_eq!(mismatch.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let bad_embedding_config = CcmConfig::new(1, 1, vec![5, 6], 0.05, 0.05);
    let bad_embedding =
        convergent_cross_mapping("x", &x, "y", &y, &bad_embedding_config).unwrap_err();
    assert_eq!(bad_embedding.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let duplicate_name = convergent_cross_mapping("x", &x, "x", &y, &config).unwrap_err();
    assert_eq!(duplicate_name.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let too_large_config = CcmConfig::new(2, 1, vec![5, 20], 0.05, 0.05);
    let library_too_large =
        convergent_cross_mapping("x", &x, "y", &y, &too_large_config).unwrap_err();
    assert_eq!(library_too_large.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let constant = [1.0f32; 8];
    let degenerate = convergent_cross_mapping("x", &constant, "y", &y, &config).unwrap_err();
    assert_eq!(degenerate.code, "CALYX_ASSAY_DEGENERATE_INPUT");

    vec![
        json!({
            "case": "length_mismatch",
            "before": {"x_len": 8, "y_len": 7},
            "after": {"code": mismatch.code},
        }),
        json!({
            "case": "bad_embedding_dim",
            "before": {"embedding_dim": 1},
            "after": {"code": bad_embedding.code},
        }),
        json!({
            "case": "duplicate_name",
            "before": {"x_name": "x", "y_name": "x"},
            "after": {"code": duplicate_name.code},
        }),
        json!({
            "case": "library_too_large",
            "before": {"library_sizes": [5, 20]},
            "after": {"code": library_too_large.code},
        }),
        json!({
            "case": "constant_series",
            "before": {"constant_x": true},
            "after": {"code": degenerate.code},
        }),
    ]
}

fn coupled_logistic_series(total: usize, burn_in: usize, coupling: f32) -> (Vec<f32>, Vec<f32>) {
    let mut x = Vec::with_capacity(total);
    let mut y = Vec::with_capacity(total);
    x.push(0.217);
    y.push(0.433);
    for t in 1..total {
        let xn = 3.82 * x[t - 1] * (1.0 - x[t - 1]);
        let intrinsic_y = 3.55 * y[t - 1] * (1.0 - y[t - 1]);
        let yn = (1.0 - coupling) * intrinsic_y + coupling * xn;
        x.push(xn);
        y.push(yn);
    }
    (x[burn_in..].to_vec(), y[burn_in..].to_vec())
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE064_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx_issue064_ccm_fsv"))
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
