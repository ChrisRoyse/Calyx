//! Full-State-Verification for #66 fixed-decay Hawkes branching ratios.
//!
//! Source of truth: one JSON report under CALYX_ISSUE066_FSV_ROOT, then a
//! separate readback that re-checks self/mutual edges and edge-case codes.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{HawkesConfig, HawkesEventSeries, HawkesStability, exponential_hawkes_em};
use serde_json::{Value, json};

#[test]
fn issue066_hawkes_fsv_writes_and_reads_back_branching_matrix() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let report_path = root.join("issue066_hawkes_fsv_report.json");
    let before = file_state(&report_path);

    let (alpha_events, beta_events) = self_mutual_events();
    let config = HawkesConfig::new(110.0, 2.0, 80, 0.15);
    let processes = [
        HawkesEventSeries {
            name: "alpha",
            event_times: &alpha_events,
        },
        HawkesEventSeries {
            name: "beta",
            event_times: &beta_events,
        },
    ];
    let report = exponential_hawkes_em(&processes, &config).unwrap();
    assert_eq!(report.stability, HawkesStability::Subcritical, "{report:?}");
    assert!(
        report.spectral_radius > 0.60 && report.spectral_radius < 0.85,
        "{report:?}"
    );
    for (source, target) in [
        ("alpha", "alpha"),
        ("beta", "beta"),
        ("alpha", "beta"),
        ("beta", "alpha"),
    ] {
        assert!(has_edge(&report, source, target), "{report:?}");
        assert!(edge_ratio(&report, source, target).unwrap() > 0.15);
    }
    assert!(edge_ratio(&report, "alpha", "beta").unwrap() > 0.40);
    assert!(edge_ratio(&report, "beta", "alpha").unwrap() > 0.30);

    let edge_cases = edge_readbacks();
    let body = json!({
        "schema": "poly.issue066.hawkes_fsv.v1",
        "proof_claim": "Fixed-decay exponential Hawkes EM recovers self- and mutually-exciting branching ratios and a subcritical spectral radius from known event streams.",
        "scope": "Fixed exponential decay Hawkes EM only; no online fitting, no nonparametric kernels, and no market execution integration.",
        "source_of_truth": {
            "path": report_path.to_string_lossy(),
            "before": before,
        },
        "minimum_sufficient_corpus": {
            "generator": "two deterministic clustered event streams",
            "processes": ["alpha", "beta"],
            "alpha_events": alpha_events.len(),
            "beta_events": beta_events.len(),
            "total_events": alpha_events.len() + beta_events.len(),
            "observation_end": config.observation_end,
            "decay": config.decay,
            "iterations": config.iterations,
            "min_edge_branching_ratio": config.min_edge_branching_ratio,
            "why_smaller_insufficient": "The proof needs enough repeated clusters to keep all four self/mutual branching ratios above 0.15 while preserving a subcritical spectral-radius margin; this 50-event corpus is the smallest checked corpus with clean margins.",
            "why_larger_wasteful": "More events would exercise the same validation, EM E-step, exposure-corrected M-step, edge filtering, spectral-radius, write, and readback paths without adding proof."
        },
        "hawkes": report,
        "edge_cases": edge_cases,
    });
    let bytes = serde_json::to_vec_pretty(&body).unwrap();
    fs::write(&report_path, &bytes).unwrap();
    assert_eq!(fs::read(&report_path).unwrap(), bytes);

    let readback = read_json(&report_path);
    assert_eq!(readback["hawkes"]["stability"], "Subcritical");
    let spectral = readback["hawkes"]["spectral_radius"].as_f64().unwrap();
    assert!(spectral > 0.60 && spectral < 0.85, "{spectral}");
    for (source, target) in [
        ("alpha", "alpha"),
        ("beta", "beta"),
        ("alpha", "beta"),
        ("beta", "alpha"),
    ] {
        assert!(
            readback["hawkes"]["retained_edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|edge| edge_pair(edge, source, target)
                    && edge["branching_ratio"].as_f64().unwrap() > 0.15),
            "{source}->{target} missing from {readback:#?}"
        );
    }
    assert!(
        readback["edge_cases"]
            .as_array()
            .unwrap()
            .iter()
            .all(|case| case["after"]["code"] == "CALYX_ASSAY_INSUFFICIENT_SAMPLES")
    );

    let digest = blake3::hash(&fs::read(&report_path).unwrap());
    println!(
        "ISSUE066_FSV path={} blake3={} spectral={} edges={} edge_cases={}",
        report_path.display(),
        digest,
        readback["hawkes"]["spectral_radius"],
        readback["hawkes"]["retained_edges"]
            .as_array()
            .unwrap()
            .len(),
        readback["edge_cases"].as_array().unwrap().len()
    );
}

fn edge_readbacks() -> Vec<Value> {
    let alpha = [1.0f32, 1.3, 2.4, 11.0, 11.3];
    let beta = [1.9f32, 2.2, 11.9, 12.2];
    let config = HawkesConfig::new(20.0, 2.0, 10, 0.1);

    let duplicate = exponential_hawkes_em(
        &[
            HawkesEventSeries {
                name: "alpha",
                event_times: &alpha,
            },
            HawkesEventSeries {
                name: "alpha",
                event_times: &beta,
            },
        ],
        &config,
    )
    .unwrap_err();
    assert_eq!(duplicate.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let unsorted = [1.0f32, 2.0, 1.5, 3.0];
    let unsorted_err = exponential_hawkes_em(
        &[
            HawkesEventSeries {
                name: "alpha",
                event_times: &unsorted,
            },
            HawkesEventSeries {
                name: "beta",
                event_times: &beta,
            },
        ],
        &config,
    )
    .unwrap_err();
    assert_eq!(unsorted_err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let outside = [1.0f32, 2.0, 25.0];
    let outside_err = exponential_hawkes_em(
        &[
            HawkesEventSeries {
                name: "alpha",
                event_times: &outside,
            },
            HawkesEventSeries {
                name: "beta",
                event_times: &beta,
            },
        ],
        &config,
    )
    .unwrap_err();
    assert_eq!(outside_err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let bad_decay = HawkesConfig::new(20.0, 0.0, 10, 0.1);
    let bad_decay_err = exponential_hawkes_em(
        &[
            HawkesEventSeries {
                name: "alpha",
                event_times: &alpha,
            },
            HawkesEventSeries {
                name: "beta",
                event_times: &beta,
            },
        ],
        &bad_decay,
    )
    .unwrap_err();
    assert_eq!(bad_decay_err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let too_few = [1.0f32];
    let too_few_err = exponential_hawkes_em(
        &[
            HawkesEventSeries {
                name: "alpha",
                event_times: &too_few,
            },
            HawkesEventSeries {
                name: "beta",
                event_times: &beta,
            },
        ],
        &config,
    )
    .unwrap_err();
    assert_eq!(too_few_err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    vec![
        json!({
            "case": "duplicate_name",
            "before": {"names": ["alpha", "alpha"]},
            "after": {"code": duplicate.code},
        }),
        json!({
            "case": "unsorted_events",
            "before": {"alpha": unsorted},
            "after": {"code": unsorted_err.code},
        }),
        json!({
            "case": "event_outside_observation",
            "before": {"event_time": 25.0, "observation_end": 20.0},
            "after": {"code": outside_err.code},
        }),
        json!({
            "case": "bad_decay",
            "before": {"decay": 0.0},
            "after": {"code": bad_decay_err.code},
        }),
        json!({
            "case": "too_few_events",
            "before": {"event_count": 1},
            "after": {"code": too_few_err.code},
        }),
    ]
}

fn self_mutual_events() -> (Vec<f32>, Vec<f32>) {
    let mut alpha = Vec::new();
    let mut beta = Vec::new();
    for base in (5..100).step_by(10) {
        let base = base as f32;
        alpha.push(base);
        alpha.push(base + 0.3);
        alpha.push(base + 1.4);
        beta.push(base + 0.9);
        beta.push(base + 1.2);
    }
    (alpha, beta)
}

fn has_edge(report: &calyx_assay::HawkesReport, source: &str, target: &str) -> bool {
    report
        .retained_edges
        .iter()
        .any(|edge| edge.source == source && edge.target == target)
}

fn edge_ratio(report: &calyx_assay::HawkesReport, source: &str, target: &str) -> Option<f32> {
    report
        .retained_edges
        .iter()
        .find(|edge| edge.source == source && edge.target == target)
        .map(|edge| edge.branching_ratio)
}

fn edge_pair(edge: &Value, source: &str, target: &str) -> bool {
    edge["source"].as_str().unwrap() == source && edge["target"].as_str().unwrap() == target
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE066_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx_issue066_hawkes_fsv"))
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
