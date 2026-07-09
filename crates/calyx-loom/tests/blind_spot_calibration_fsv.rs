use std::fs;

use calyx_core::{CxId, SlotId};
use calyx_loom::{
    BlindSpotCalibration, BlindSpotCalibrationParams, CALYX_LOOM_UNCALIBRATED_BLINDSPOT, Severity,
    detect_blind_spot, detect_blind_spot_calibrated,
};
use serde_json::json;

fn cx(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue1209-blind-spot-calibration")
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

#[test]
fn calibrated_blind_spots_are_rank_based_across_lens_scales() {
    let params = BlindSpotCalibrationParams {
        min_samples: 50,
        alpha: 0.05,
    };
    let compressed = scaled_deltas(0.02, 0.18);
    let wide = scaled_deltas(0.12, 0.98);
    let compressed_calibration =
        BlindSpotCalibration::from_deltas(compressed.clone(), params).unwrap();
    let wide_calibration = BlindSpotCalibration::from_deltas(wide.clone(), params).unwrap();
    let compressed_delta = *compressed.last().unwrap();
    let wide_delta = *wide.last().unwrap();

    let compressed_alert = detect_blind_spot_calibrated(
        cx("compressed"),
        slot(1),
        slot(2),
        compressed_delta,
        0.0,
        &compressed_calibration,
    )
    .unwrap()
    .unwrap();
    let wide_alert = detect_blind_spot_calibrated(
        cx("wide"),
        slot(3),
        slot(4),
        wide_delta,
        0.0,
        &wide_calibration,
    )
    .unwrap()
    .unwrap();
    let legacy_compressed = detect_blind_spot(
        cx("legacy-compressed"),
        slot(1),
        slot(2),
        compressed_delta,
        0.0,
    );
    let legacy_wide = detect_blind_spot(cx("legacy-wide"), slot(3), slot(4), wide_delta, 0.0);
    let null_alerts = count_null_alerts(&wide, &wide_calibration);
    let under_sampled_error =
        BlindSpotCalibration::from_deltas(wide[..12].to_vec(), params).unwrap_err();

    assert_eq!(compressed_alert.severity, wide_alert.severity);
    assert_eq!(compressed_alert.severity, Severity::High);
    assert_eq!(
        compressed_alert.calibration.as_ref().unwrap().percentile,
        wide_alert.calibration.as_ref().unwrap().percentile
    );
    assert!(legacy_compressed.is_none());
    assert!(legacy_wide.is_some());
    assert_eq!(null_alerts, 3);
    assert_eq!(under_sampled_error.code, CALYX_LOOM_UNCALIBRATED_BLINDSPOT);

    maybe_write_fsv(json!({
        "source_of_truth": "calyx-loom calibrated blind-spot detector outputs read back from this JSON artifact",
        "params": params,
        "same_rank_different_scale": {
            "compressed_delta": compressed_delta,
            "wide_delta": wide_delta,
            "compressed_calibrated": compressed_alert,
            "wide_calibrated": wide_alert,
            "legacy_compressed_alert": legacy_compressed.is_some(),
            "legacy_wide_alert": legacy_wide.is_some(),
        },
        "null_fdr": {
            "sample_count": wide.len(),
            "alpha": params.alpha,
            "alert_count": null_alerts,
        },
        "edge_case": {
            "case": "below_min_calibration_samples",
            "samples": 12,
            "error": under_sampled_error.code,
            "message": under_sampled_error.message,
        },
    }));
}

#[test]
fn calibration_alpha_rejects_zero_and_one() {
    for alpha in [0.0, 1.0] {
        let err = BlindSpotCalibration::from_deltas(
            scaled_deltas(0.02, 0.18),
            BlindSpotCalibrationParams {
                min_samples: 50,
                alpha,
            },
        )
        .unwrap_err();

        assert_eq!(err.code, CALYX_LOOM_UNCALIBRATED_BLINDSPOT);
    }
}

fn scaled_deltas(start: f32, end: f32) -> Vec<f32> {
    let n = 60;
    (0..n)
        .map(|index| {
            let t = index as f32 / (n - 1) as f32;
            start + (end - start) * t
        })
        .collect()
}

fn count_null_alerts(deltas: &[f32], calibration: &BlindSpotCalibration) -> usize {
    deltas
        .iter()
        .enumerate()
        .filter(|(index, delta)| {
            detect_blind_spot_calibrated(
                cx(&format!("null-{index}")),
                slot(9),
                slot(10),
                **delta,
                0.0,
                calibration,
            )
            .unwrap()
            .is_some()
        })
        .count()
}

fn maybe_write_fsv(readback: serde_json::Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).unwrap();
    let path = root.join("issue1209_blind_spot_calibration_readback.json");
    let bytes = serde_json::to_vec_pretty(&readback).unwrap();
    fs::write(&path, &bytes).unwrap();
    let stored = fs::read(&path).unwrap();
    assert_eq!(stored, bytes);
    println!(
        "ISSUE1209_BLIND_SPOT_CALIBRATION_READBACK={}",
        path.display()
    );
}
