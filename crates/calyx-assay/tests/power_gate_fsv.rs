use std::fs;

use calyx_assay::{
    CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY, CALYX_ASSAY_ESTIMATOR_UNDERPOWERED, EstimatorKind,
    MiEstimate, PowerCalibration, PowerCalibrationStatus, TrustTag,
    ensure_informative_binary_labels, panel_sufficiency_from_estimate,
};
use serde_json::json;

#[test]
fn power_gate_rejects_underpowered_and_degenerate_targets() {
    let mut labels = vec![false; 200];
    labels[0] = true;
    let entropy_error = ensure_informative_binary_labels(&labels).unwrap_err();
    assert_eq!(entropy_error.code, CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY);

    let missing = estimate_without_calibration();
    let missing_error =
        panel_sufficiency_from_estimate(&missing, 1.0, &[], TrustTag::Trusted).unwrap_err();
    assert_eq!(missing_error.code, CALYX_ASSAY_ESTIMATOR_UNDERPOWERED);

    let underpowered = PowerCalibration::new(1.0, 0.25, 0.5, 64, 4096, 4095).unwrap();
    assert_eq!(underpowered.status, PowerCalibrationStatus::Underpowered);
    assert_eq!(
        underpowered.require_passed().unwrap_err().code,
        CALYX_ASSAY_ESTIMATOR_UNDERPOWERED
    );
    let underpowered_estimate = estimate_without_calibration().with_power_calibration(underpowered);
    let underpowered_error =
        panel_sufficiency_from_estimate(&underpowered_estimate, 1.0, &[], TrustTag::Trusted)
            .unwrap_err();
    assert_eq!(underpowered_error.code, CALYX_ASSAY_ESTIMATOR_UNDERPOWERED);

    let passed = PowerCalibration::new(1.0, 0.75, 0.5, 64, 4096, 4095).unwrap();
    assert_eq!(passed.status, PowerCalibrationStatus::Passed);
    let passed_estimate = estimate_without_calibration().with_power_calibration(passed);
    let sufficiency =
        panel_sufficiency_from_estimate(&passed_estimate, 1.0, &[], TrustTag::Trusted).unwrap();
    assert!(sufficiency.sufficient);

    write_fsv_artifact(json!({
        "schema": "calyx-assay-power-gate-fsv-v1",
        "entropy_floor": {
            "labels_total": labels.len(),
            "positives": 1,
            "negatives": labels.len() - 1,
            "error_code": entropy_error.code,
        },
        "underpowered": {
            "n_samples": 64,
            "n_features": 4096,
            "recovery_ratio": 0.25,
            "status": "underpowered",
            "missing_calibration_error_code": missing_error.code,
            "sufficiency_error_code": underpowered_error.code,
        },
        "passed": {
            "status": "passed",
            "sufficient": sufficiency.sufficient,
            "sufficiency_basis_bits": sufficiency.sufficiency_basis_bits,
            "power_status": "passed",
        }
    }));
}

fn estimate_without_calibration() -> MiEstimate {
    MiEstimate::new(
        1.25,
        1.10,
        1.40,
        64,
        EstimatorKind::LogisticProbe,
        TrustTag::Trusted,
    )
}

fn write_fsv_artifact(value: serde_json::Value) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("issue874_power_gate_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(readback["schema"], "calyx-assay-power-gate-fsv-v1");
    assert_eq!(
        readback["underpowered"]["sufficiency_error_code"],
        CALYX_ASSAY_ESTIMATOR_UNDERPOWERED
    );
    assert_eq!(
        readback["entropy_floor"]["error_code"],
        CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY
    );
}
