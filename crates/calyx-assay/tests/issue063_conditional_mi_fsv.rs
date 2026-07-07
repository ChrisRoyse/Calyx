//! Full-State-Verification for #63 Gaussian conditional mutual information.
//!
//! Source of truth: one JSON report under CALYX_ISSUE063_FSV_ROOT, then a
//! separate readback that re-checks formula, decisions, and edge-case codes.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    ConditionalIndependence, DEFAULT_CMI_ALPHA, GAUSSIAN_CMI_FORMULA,
    conditional_mutual_information_gaussian, conditional_mutual_information_gaussian_with_alpha,
};
use serde_json::{Value, json};

const TOL: f32 = 1e-4;

#[test]
fn issue063_conditional_mi_fsv_writes_and_reads_back_oracle() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let report_path = root.join("issue063_conditional_mi_fsv_report.json");
    let before = file_state(&report_path);

    let formula_lock = formula_lock_case();
    let confounded = confounded_only_case();
    let direct = direct_dependency_case();
    let edges = edge_readbacks();

    let report = json!({
        "schema": "poly.issue063.conditional_mi_fsv.v1",
        "proof_claim": "Gaussian CMI computes -0.5*log2(1-r^2) from fail-closed partial correlation and returns a conditional-independence decision.",
        "scope": "Gaussian/linear scalar X,Y with one or more scalar controls; not arbitrary non-Gaussian KSG-CMI.",
        "source_of_truth": {
            "path": report_path.to_string_lossy(),
            "before": before,
        },
        "minimum_sufficient_corpus": {
            "formula_lock_samples": 6,
            "confounded_samples": 160,
            "direct_dependency_samples": 160,
            "edge_cases": edges.len(),
            "why_smaller_insufficient": "Needs formula-lock, conditionally-independent confounding, conditionally-dependent direct signal, and fail-closed boundaries.",
            "why_larger_wasteful": "Larger data would exercise the same partial-correlation, CMI formula, decision, write, and readback paths without adding proof."
        },
        "formula": GAUSSIAN_CMI_FORMULA,
        "default_alpha": DEFAULT_CMI_ALPHA,
        "formula_lock": formula_lock,
        "confounded_only": confounded,
        "direct_dependency": direct,
        "edge_cases": edges,
    });
    let bytes = serde_json::to_vec_pretty(&report).unwrap();
    fs::write(&report_path, &bytes).unwrap();
    assert_eq!(fs::read(&report_path).unwrap(), bytes);

    let readback = read_json(&report_path);
    assert_eq!(readback["formula"], GAUSSIAN_CMI_FORMULA);
    assert_eq!(readback["formula_lock"]["decision"], "Independent");
    assert_eq!(readback["confounded_only"]["decision"], "Independent");
    assert_eq!(readback["direct_dependency"]["decision"], "Dependent");
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
        "ISSUE063_FSV path={} blake3={} formula_bits={} confounded_bits={} direct_bits={} edges={}",
        report_path.display(),
        digest,
        readback["formula_lock"]["cmi_bits"],
        readback["confounded_only"]["cmi_bits"],
        readback["direct_dependency"]["cmi_bits"],
        readback["edge_cases"].as_array().unwrap().len()
    );
}

fn formula_lock_case() -> Value {
    let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
    let y = [2.0f32, 1.0, 4.0, 3.0, 7.0, 5.0];
    let z = [5.0f32, 6.0, 2.0, 1.0, 4.0, 3.0];
    let report = conditional_mutual_information_gaussian(&x, &y, &z).unwrap();
    assert!((report.partial_r - 0.760_417_2).abs() <= TOL, "{report:?}");
    assert!((report.cmi_bits - 0.622_743_2).abs() <= 1e-3, "{report:?}");
    assert_eq!(report.decision, ConditionalIndependence::Independent);
    json!(report)
}

fn confounded_only_case() -> Value {
    let (x, y, z) = confounded_only_series(160);
    let report = conditional_mutual_information_gaussian(&x, &y, &z).unwrap();
    assert!(
        report.cmi_bits < 0.02,
        "pure confounding should collapse after Z: {report:?}"
    );
    assert_eq!(report.decision, ConditionalIndependence::Independent);
    json!(report)
}

fn direct_dependency_case() -> Value {
    let (x, y, z) = direct_dependency_series(160);
    let report = conditional_mutual_information_gaussian(&x, &y, &z).unwrap();
    assert!(
        report.cmi_bits > 0.4,
        "direct X->Y signal should remain after Z: {report:?}"
    );
    assert!(report.p_value < 0.001, "{report:?}");
    assert_eq!(report.decision, ConditionalIndependence::Dependent);
    json!(report)
}

fn edge_readbacks() -> Vec<Value> {
    let empty_controls = conditional_mutual_information_gaussian_with_alpha(
        &[1.0, 2.0, 3.0, 4.0],
        &[1.0, 2.0, 4.0, 8.0],
        &[],
        DEFAULT_CMI_ALPHA,
    )
    .unwrap_err();
    assert_eq!(empty_controls.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let bad_alpha = conditional_mutual_information_gaussian_with_alpha(
        &[1.0, 2.0, 3.0, 4.0],
        &[1.0, 3.0, 4.0, 8.0],
        &[&[0.0, 1.0, 0.0, 1.0]],
        0.0,
    )
    .unwrap_err();
    assert_eq!(bad_alpha.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let short_z = conditional_mutual_information_gaussian(
        &[1.0, 2.0, 3.0, 4.0],
        &[1.0, 3.0, 4.0, 8.0],
        &[0.0, 1.0],
    )
    .unwrap_err();
    assert_eq!(short_z.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let constant_control = conditional_mutual_information_gaussian(
        &[1.0, 2.0, 3.0, 4.0],
        &[1.0, 3.0, 4.0, 8.0],
        &[5.0, 5.0, 5.0, 5.0],
    )
    .unwrap_err();
    assert_eq!(constant_control.code, "CALYX_ASSAY_DEGENERATE_INPUT");

    vec![
        json!({
            "case": "empty_controls",
            "before": {"controls": 0},
            "after": {"code": empty_controls.code},
        }),
        json!({
            "case": "bad_alpha",
            "before": {"alpha": 0.0},
            "after": {"code": bad_alpha.code},
        }),
        json!({
            "case": "length_mismatch",
            "before": {"x_len": 4, "z_len": 2},
            "after": {"code": short_z.code},
        }),
        json!({
            "case": "constant_control",
            "before": {"z": [5, 5, 5, 5]},
            "after": {"code": constant_control.code},
        }),
    ]
}

fn confounded_only_series(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    let mut z = Vec::with_capacity(n);
    for t in 0..n {
        let zt = centered_noise(t as u64, 11);
        let ex = centered_noise(t as u64, 29);
        let ey = centered_noise(t as u64, 47);
        z.push(zt);
        x.push(1.4 * zt + 0.8 * ex);
        y.push(-1.2 * zt + 0.8 * ey);
    }
    (x, y, z)
}

fn direct_dependency_series(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    let mut z = Vec::with_capacity(n);
    for t in 0..n {
        let zt = centered_noise(t as u64, 101);
        let xt = centered_noise(t as u64, 131);
        let ey = centered_noise(t as u64, 173);
        z.push(zt);
        x.push(xt);
        y.push(0.95 * xt + 0.45 * zt + 0.2 * ey);
    }
    (x, y, z)
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
    std::env::var_os("CALYX_ISSUE063_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx_issue063_conditional_mi_fsv"))
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
