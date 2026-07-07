//! Full-State-Verification for #69 sparse partial-correlation network.
//!
//! Source of truth: one JSON report under CALYX_ISSUE069_FSV_ROOT, then a
//! separate readback that re-checks retained/pruned edges and edge-case codes.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{PartialNetworkReport, PartialNetworkSeries, partial_correlation_network};
use serde_json::{Value, json};

#[test]
fn issue069_partial_network_fsv_writes_and_reads_back_graph() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let report_path = root.join("issue069_partial_network_fsv_report.json");
    let before = file_state(&report_path);

    let known = known_graph_series(100);
    let series = [
        PartialNetworkSeries {
            name: "a",
            values: &known.a,
        },
        PartialNetworkSeries {
            name: "b",
            values: &known.b,
        },
        PartialNetworkSeries {
            name: "z",
            values: &known.z,
        },
        PartialNetworkSeries {
            name: "x",
            values: &known.x,
        },
        PartialNetworkSeries {
            name: "y",
            values: &known.y,
        },
        PartialNetworkSeries {
            name: "noise",
            values: &known.noise,
        },
    ];
    let report = partial_correlation_network(&series, 0.001, 0.25).unwrap();
    assert_eq!(report.retained_edges.len(), 3, "{report:?}");
    assert!(has_edge(&report, "a", "b"), "{report:?}");
    assert!(has_edge(&report, "z", "x"), "{report:?}");
    assert!(has_edge(&report, "z", "y"), "{report:?}");
    assert!(!has_edge(&report, "x", "y"), "{report:?}");
    assert!(
        report
            .retained_edges
            .iter()
            .all(|edge| edge.left != "noise" && edge.right != "noise"),
        "{report:?}"
    );
    let xy = pruned_edge(&report, "x", "y").unwrap();
    assert!(xy.zero_order_r.abs() > 0.80, "{xy:?}");
    assert!(xy.partial_r.abs() < 0.20, "{xy:?}");

    let edge_cases = edge_readbacks();
    let body = json!({
        "schema": "poly.issue069.partial_network_fsv.v1",
        "proof_claim": "Gaussian partial-correlation network retains direct conditional edges and prunes confounded or independent edges after conditioning on all other variables.",
        "scope": "Gaussian/linear all-other-controls partial-correlation network only; no graphical LASSO regularization.",
        "source_of_truth": {
            "path": report_path.to_string_lossy(),
            "before": before,
        },
        "minimum_sufficient_corpus": {
            "samples": known.a.len(),
            "variables": ["a", "b", "z", "x", "y", "noise"],
            "alpha": 0.001,
            "min_abs_partial_r": 0.25,
            "expected_retained_edges": ["a-b", "z-x", "z-y"],
            "why_smaller_insufficient": "Below 100 deterministic rows, finite-sample false-edge margins become borderline while exercising the same graph path; 100 cleanly separates direct edges from confounded and noise edges.",
            "why_larger_wasteful": "More rows would not exercise additional validation, all-other-controls partial-correlation, retain/prune, artifact write, or readback paths for this proof claim."
        },
        "partial_network": report,
        "edge_cases": edge_cases,
    });
    let bytes = serde_json::to_vec_pretty(&body).unwrap();
    fs::write(&report_path, &bytes).unwrap();
    assert_eq!(fs::read(&report_path).unwrap(), bytes);

    let readback = read_json(&report_path);
    assert_eq!(
        readback["partial_network"]["retained_edges"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    for (left, right) in [("a", "b"), ("z", "x"), ("z", "y")] {
        assert!(
            readback["partial_network"]["retained_edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|edge| edge_pair(edge, left, right)),
            "{left}-{right} missing from {readback:#?}"
        );
    }
    assert!(
        readback["partial_network"]["pruned_edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge_pair(edge, "x", "y")
                && edge["zero_order_r"].as_f64().unwrap().abs() > 0.80
                && edge["partial_r"].as_f64().unwrap().abs() < 0.20)
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
        "ISSUE069_FSV path={} blake3={} retained={} pruned={} edge_cases={}",
        report_path.display(),
        digest,
        readback["partial_network"]["retained_edges"]
            .as_array()
            .unwrap()
            .len(),
        readback["partial_network"]["pruned_edges"]
            .as_array()
            .unwrap()
            .len(),
        readback["edge_cases"].as_array().unwrap().len()
    );
}

fn edge_readbacks() -> Vec<Value> {
    let x = [1.0f32, 2.0, 3.0, 4.0, 5.0];
    let y = [1.1f32, 1.9, 3.2, 4.1, 5.2];
    let z = [4.9f32, 4.0, 3.0, 2.1, 1.1];
    let duplicate_name = [
        PartialNetworkSeries {
            name: "x",
            values: &x,
        },
        PartialNetworkSeries {
            name: "x",
            values: &y,
        },
        PartialNetworkSeries {
            name: "z",
            values: &z,
        },
    ];
    let duplicate = partial_correlation_network(&duplicate_name, 0.05, 0.1).unwrap_err();
    assert_eq!(duplicate.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let bad_alpha = partial_correlation_network(
        &[
            PartialNetworkSeries {
                name: "x",
                values: &x,
            },
            PartialNetworkSeries {
                name: "y",
                values: &y,
            },
            PartialNetworkSeries {
                name: "z",
                values: &z,
            },
        ],
        1.0,
        0.1,
    )
    .unwrap_err();
    assert_eq!(bad_alpha.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let short = [1.0f32, 2.0, 3.0, 4.0];
    let mismatch = partial_correlation_network(
        &[
            PartialNetworkSeries {
                name: "x",
                values: &x,
            },
            PartialNetworkSeries {
                name: "y",
                values: &y,
            },
            PartialNetworkSeries {
                name: "z",
                values: &short,
            },
        ],
        0.05,
        0.1,
    )
    .unwrap_err();
    assert_eq!(mismatch.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let bad_floor = partial_correlation_network(
        &[
            PartialNetworkSeries {
                name: "x",
                values: &x,
            },
            PartialNetworkSeries {
                name: "y",
                values: &y,
            },
            PartialNetworkSeries {
                name: "z",
                values: &z,
            },
        ],
        0.05,
        1.5,
    )
    .unwrap_err();
    assert_eq!(bad_floor.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let constant = [1.0f32; 5];
    let degenerate = partial_correlation_network(
        &[
            PartialNetworkSeries {
                name: "x",
                values: &x,
            },
            PartialNetworkSeries {
                name: "y",
                values: &y,
            },
            PartialNetworkSeries {
                name: "constant",
                values: &constant,
            },
        ],
        0.05,
        0.1,
    )
    .unwrap_err();
    assert_eq!(degenerate.code, "CALYX_ASSAY_DEGENERATE_INPUT");

    vec![
        json!({
            "case": "duplicate_name",
            "before": {"names": ["x", "x", "z"]},
            "after": {"code": duplicate.code},
        }),
        json!({
            "case": "bad_alpha",
            "before": {"alpha": 1.0},
            "after": {"code": bad_alpha.code},
        }),
        json!({
            "case": "length_mismatch",
            "before": {"x_len": 5, "z_len": 4},
            "after": {"code": mismatch.code},
        }),
        json!({
            "case": "bad_effect_floor",
            "before": {"min_abs_partial_r": 1.5},
            "after": {"code": bad_floor.code},
        }),
        json!({
            "case": "constant_column",
            "before": {"constant": true},
            "after": {"code": degenerate.code},
        }),
    ]
}

struct KnownGraphSeries {
    a: Vec<f32>,
    b: Vec<f32>,
    z: Vec<f32>,
    x: Vec<f32>,
    y: Vec<f32>,
    noise: Vec<f32>,
}

fn known_graph_series(n: usize) -> KnownGraphSeries {
    let mut a = Vec::with_capacity(n);
    let mut b = Vec::with_capacity(n);
    let mut z = Vec::with_capacity(n);
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    let mut noise = Vec::with_capacity(n);
    for t in 0..n {
        let t = t as u64;
        let root = centered_noise(t, 11);
        let driver = centered_noise(t, 23);
        let at = driver + 0.25 * centered_noise(t, 31);
        let bt = 0.9 * at + 0.25 * centered_noise(t, 37);
        let zt = root;
        let xt = 1.1 * zt + 0.30 * centered_noise(t, 41);
        let yt = -zt + 0.30 * centered_noise(t, 47);
        a.push(at);
        b.push(bt);
        z.push(zt);
        x.push(xt);
        y.push(yt);
        noise.push(centered_noise(t, 59));
    }
    KnownGraphSeries {
        a,
        b,
        z,
        x,
        y,
        noise,
    }
}

fn has_edge(report: &PartialNetworkReport, left: &str, right: &str) -> bool {
    report
        .retained_edges
        .iter()
        .any(|edge| pair_names(&edge.left, &edge.right, left, right))
}

fn pruned_edge<'a>(
    report: &'a PartialNetworkReport,
    left: &str,
    right: &str,
) -> Option<&'a calyx_assay::PartialNetworkPrunedEdge> {
    report
        .pruned_edges
        .iter()
        .find(|edge| pair_names(&edge.left, &edge.right, left, right))
}

fn edge_pair(edge: &Value, left: &str, right: &str) -> bool {
    pair_names(
        edge["left"].as_str().unwrap(),
        edge["right"].as_str().unwrap(),
        left,
        right,
    )
}

fn pair_names(a: &str, b: &str, left: &str, right: &str) -> bool {
    (a == left && b == right) || (a == right && b == left)
}

fn centered_noise(t: u64, salt: u64) -> f32 {
    (splitmix(t ^ salt.rotate_left(13)) - 0.5) as f32
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
    std::env::var_os("CALYX_ISSUE069_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx_issue069_partial_network_fsv"))
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
