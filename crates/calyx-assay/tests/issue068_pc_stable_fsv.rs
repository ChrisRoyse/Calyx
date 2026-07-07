//! Full-State-Verification for #68 Gaussian PC-stable causal skeleton discovery.
//!
//! Source of truth: one JSON report under CALYX_ISSUE068_FSV_ROOT, then a
//! separate readback that re-checks retained/removed edges and edge-case codes.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{PcSeries, pc_stable_gaussian};
use serde_json::{Value, json};

#[test]
fn issue068_pc_stable_fsv_writes_and_reads_back_skeleton() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let report_path = root.join("issue068_pc_stable_fsv_report.json");
    let before = file_state(&report_path);

    let (x, z, y, noise) = fork_series(180);
    let series = [
        PcSeries {
            name: "x",
            values: &x,
        },
        PcSeries {
            name: "z",
            values: &z,
        },
        PcSeries {
            name: "y",
            values: &y,
        },
        PcSeries {
            name: "noise",
            values: &noise,
        },
    ];
    let report = pc_stable_gaussian(&series, 0.01, 1).unwrap();
    assert!(has_edge(&report, "x", "z"), "{report:?}");
    assert!(has_edge(&report, "z", "y"), "{report:?}");
    assert!(!has_edge(&report, "x", "y"), "{report:?}");
    assert!(
        removed_with_conditioning(&report, "x", "y", &["z"]),
        "{report:?}"
    );
    assert!(
        report
            .retained_edges
            .iter()
            .all(|edge| edge.left != "noise" && edge.right != "noise"),
        "{report:?}"
    );

    let edges = edge_readbacks();
    let body = json!({
        "schema": "poly.issue068.pc_stable_fsv.v1",
        "proof_claim": "Gaussian PC-stable removes conditionally independent edges with recorded separating sets while retaining direct fork edges.",
        "scope": "Gaussian/linear scalar PC-stable skeleton discovery only; no nonlinear CI and no orientation.",
        "source_of_truth": {
            "path": report_path.to_string_lossy(),
            "before": before,
        },
        "minimum_sufficient_corpus": {
            "samples": x.len(),
            "variables": ["x", "z", "y", "noise"],
            "alpha": 0.01,
            "max_conditioning": 1,
            "why_smaller_insufficient": "Needs marginal X/Y association, conditional X/Y removal by Z, retained X-Z and Z-Y edges, independent-noise pruning, and fail-closed boundaries.",
            "why_larger_wasteful": "Larger data would exercise the same graph, CI-test, separating-set, write, readback, and edge paths without adding proof."
        },
        "pc_stable": report,
        "edge_cases": edges,
    });
    let bytes = serde_json::to_vec_pretty(&body).unwrap();
    fs::write(&report_path, &bytes).unwrap();
    assert_eq!(fs::read(&report_path).unwrap(), bytes);

    let readback = read_json(&report_path);
    assert!(
        readback["pc_stable"]["retained_edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge_pair(edge, "x", "z"))
    );
    assert!(
        readback["pc_stable"]["retained_edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge_pair(edge, "z", "y"))
    );
    assert!(
        readback["pc_stable"]["removed_edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge_pair(edge, "x", "y")
                && edge["conditioning_set"].as_array().unwrap() == &vec![json!("z")])
    );
    assert!(
        readback["edge_cases"]
            .as_array()
            .unwrap()
            .iter()
            .all(|edge| edge["after"]["code"] == "CALYX_ASSAY_INSUFFICIENT_SAMPLES")
    );

    let digest = blake3::hash(&fs::read(&report_path).unwrap());
    println!(
        "ISSUE068_FSV path={} blake3={} retained={} removed={} edges={}",
        report_path.display(),
        digest,
        readback["pc_stable"]["retained_edges"]
            .as_array()
            .unwrap()
            .len(),
        readback["pc_stable"]["removed_edges"]
            .as_array()
            .unwrap()
            .len(),
        readback["edge_cases"].as_array().unwrap().len()
    );
}

fn edge_readbacks() -> Vec<Value> {
    let x = [1.0f32, 2.0, 3.0, 4.0];
    let y = [1.0f32, 3.0, 4.0, 8.0];
    let duplicate_name = [
        PcSeries {
            name: "x",
            values: &x,
        },
        PcSeries {
            name: "x",
            values: &y,
        },
    ];
    let duplicate = pc_stable_gaussian(&duplicate_name, 0.05, 0).unwrap_err();
    assert_eq!(duplicate.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let bad_alpha = pc_stable_gaussian(
        &[
            PcSeries {
                name: "x",
                values: &x,
            },
            PcSeries {
                name: "y",
                values: &y,
            },
        ],
        1.0,
        0,
    )
    .unwrap_err();
    assert_eq!(bad_alpha.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let short_y = [1.0f32, 2.0, 3.0];
    let mismatch = pc_stable_gaussian(
        &[
            PcSeries {
                name: "x",
                values: &x,
            },
            PcSeries {
                name: "y",
                values: &short_y,
            },
        ],
        0.05,
        0,
    )
    .unwrap_err();
    assert_eq!(mismatch.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let too_deep = pc_stable_gaussian(
        &[
            PcSeries {
                name: "x",
                values: &x,
            },
            PcSeries {
                name: "y",
                values: &y,
            },
        ],
        0.05,
        1,
    )
    .unwrap_err();
    assert_eq!(too_deep.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    vec![
        json!({
            "case": "duplicate_name",
            "before": {"names": ["x", "x"]},
            "after": {"code": duplicate.code},
        }),
        json!({
            "case": "bad_alpha",
            "before": {"alpha": 1.0},
            "after": {"code": bad_alpha.code},
        }),
        json!({
            "case": "length_mismatch",
            "before": {"x_len": 4, "y_len": 3},
            "after": {"code": mismatch.code},
        }),
        json!({
            "case": "max_conditioning_too_deep",
            "before": {"variables": 2, "max_conditioning": 1},
            "after": {"code": too_deep.code},
        }),
    ]
}

fn fork_series(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut x = Vec::with_capacity(n);
    let mut z = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    let mut noise = Vec::with_capacity(n);
    for t in 0..n {
        let zt = centered_noise(t as u64, 11);
        let ex = centered_noise(t as u64, 29);
        let ey = centered_noise(t as u64, 47);
        z.push(zt);
        x.push(1.2 * zt + 0.45 * ex);
        y.push(-1.1 * zt + 0.45 * ey);
        noise.push(centered_noise(t as u64, 83));
    }
    (x, z, y, noise)
}

fn has_edge(report: &calyx_assay::PcStableReport, left: &str, right: &str) -> bool {
    report
        .retained_edges
        .iter()
        .any(|edge| pair_names(&edge.left, &edge.right, left, right))
}

fn removed_with_conditioning(
    report: &calyx_assay::PcStableReport,
    left: &str,
    right: &str,
    conditioning: &[&str],
) -> bool {
    report.removed_edges.iter().any(|edge| {
        pair_names(&edge.left, &edge.right, left, right)
            && edge.conditioning_set
                == conditioning
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
    })
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
    std::env::var_os("CALYX_ISSUE068_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx_issue068_pc_stable_fsv"))
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
