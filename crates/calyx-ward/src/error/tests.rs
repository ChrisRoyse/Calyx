use super::*;
use crate::profile::NoveltyAction;
use crate::verdict::GuardVerdict;
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn ood_display_contains_code_and_failing_slot_values() {
    let error = WardError::Ood {
        guard_id: guard_id(),
        failing: vec![slot_verdict(2, 0.40, 0.70, false)],
    };
    let formatted = error.to_string();

    assert!(formatted.contains(CALYX_GUARD_OOD));
    assert!(formatted.contains("slot 2"));
    assert!(formatted.contains("cos=0.4"));
    assert!(formatted.contains("tau=0.7"));
}

#[test]
fn policy_violation_display_contains_code_and_counts() {
    let error = WardError::PolicyViolation {
        k: 5,
        n_required: 3,
    };
    let formatted = error.to_string();

    assert!(formatted.contains(CALYX_GUARD_POLICY_VIOLATION));
    assert!(formatted.contains("k=5"));
    assert!(formatted.contains("n_required=3"));
}

#[test]
fn provisional_display_contains_code_and_high_stakes_advice() {
    let error = WardError::Provisional {
        guard_id: guard_id(),
    };
    let formatted = error.to_string();

    assert!(formatted.contains(CALYX_GUARD_PROVISIONAL));
    assert!(formatted.contains("calibrate before high-stakes use"));
}

#[test]
fn missing_slot_calibration_display_contains_code_and_slot() {
    let error = WardError::MissingSlotCalibration {
        guard_id: guard_id(),
        slot: slot(7),
    };
    let formatted = error.to_string();

    assert_eq!(error.code(), CALYX_GUARD_PROVISIONAL);
    assert!(formatted.contains(CALYX_GUARD_PROVISIONAL));
    assert!(formatted.contains("required slot 7"));
    assert!(formatted.contains("high-stakes calibration provenance"));
}

#[test]
fn inert_profile_display_contains_code_and_reason() {
    let error = WardError::InertProfile {
        guard_id: guard_id(),
        reason: "kofn_zero",
    };
    let formatted = error.to_string();

    assert_eq!(error.code(), CALYX_GUARD_INERT_PROFILE);
    assert!(formatted.contains(CALYX_GUARD_INERT_PROFILE));
    assert!(formatted.contains("kofn_zero"));
    assert!(formatted.contains("trusted guard surfaces"));
}

#[test]
fn missing_slot_display_contains_code_and_slot() {
    let error = WardError::MissingSlot { slot: slot(7) };
    let formatted = error.to_string();

    assert!(formatted.contains(CALYX_GUARD_MISSING_SLOT));
    assert!(formatted.contains("slot 7"));
}

#[test]
fn insufficient_calibration_data_uses_provisional_code() {
    let error = WardError::InsufficientCalibrationData { n: 49, min: 50 };
    let formatted = error.to_string();

    assert_eq!(error.code(), CALYX_GUARD_PROVISIONAL);
    assert!(formatted.contains(CALYX_GUARD_PROVISIONAL));
    assert!(formatted.contains("n=49"));
    assert!(formatted.contains("min=50"));
}

#[test]
fn invalid_required_slot_derivation_uses_provisional_code() {
    let error = WardError::InvalidRequiredSlotDerivation {
        reason: "no load-bearing slots for anchor",
    };
    let formatted = error.to_string();

    assert_eq!(error.code(), CALYX_GUARD_PROVISIONAL);
    assert!(formatted.contains(CALYX_GUARD_PROVISIONAL));
    assert!(formatted.contains("required-slot derivation"));
}

#[test]
fn novelty_errors_have_stable_codes() {
    let not_failure = WardError::NotAFailure {
        guard_id: guard_id(),
    };
    let mismatch = WardError::GuardIdMismatch {
        profile_guard_id: guard_id(),
        verdict_guard_id: other_guard_id(),
    };
    let sink = WardError::NoveltySink {
        reason: "synthetic write failure".to_string(),
    };
    let identity_slot = WardError::IdentitySlotNotRequired { slot: slot(9) };
    let model_missing = WardError::ModelNotFound {
        path: "/missing/wavlm.onnx".into(),
    };
    let invalid_input = WardError::InvalidInput {
        reason: "empty audio".to_string(),
    };
    let dim_mismatch = WardError::ModelDimMismatch {
        expected: 256,
        actual: 128,
    };
    let runtime = WardError::Runtime {
        reason: "ONNX init failed".to_string(),
    };

    assert_eq!(not_failure.code(), CALYX_GUARD_NOT_A_FAILURE);
    assert!(not_failure.to_string().contains("novelty handling"));
    assert_eq!(mismatch.code(), CALYX_GUARD_ID_MISMATCH);
    assert!(mismatch.to_string().starts_with(CALYX_GUARD_ID_MISMATCH));
    assert_eq!(sink.code(), CALYX_GUARD_NOVELTY_SINK);
    assert!(sink.to_string().contains("synthetic write failure"));
    assert_eq!(identity_slot.code(), CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED);
    assert!(identity_slot.to_string().contains("identity slot 9"));
    assert_eq!(model_missing.code(), CALYX_WARD_MODEL_NOT_FOUND);
    assert!(model_missing.to_string().contains("/missing/wavlm.onnx"));
    assert_eq!(invalid_input.code(), CALYX_WARD_INVALID_INPUT);
    assert_eq!(dim_mismatch.code(), CALYX_WARD_MODEL_DIM_MISMATCH);
    assert_eq!(runtime.code(), CALYX_WARD_RUNTIME_ERROR);
}

#[test]
fn calibration_slot_errors_have_stable_codes_and_name_the_slot() {
    let shape = WardError::CalibrationSlotShape {
        slot: slot(1),
        shape: "sparse(30522)".to_string(),
    };
    let unknown = WardError::CalibrationSlotUnknown {
        slot: slot(99),
        panel_version: 7,
    };
    let state = WardError::CalibrationSlotState {
        slot: slot(3),
        state: "parked".to_string(),
    };

    assert_eq!(shape.code(), CALYX_GUARD_CALIBRATION_SLOT_SHAPE);
    assert!(shape.to_string().contains("slot 1"));
    assert!(shape.to_string().contains("sparse(30522)"));
    assert_eq!(unknown.code(), CALYX_GUARD_CALIBRATION_SLOT_UNKNOWN);
    assert!(unknown.to_string().contains("slot 99"));
    assert!(unknown.to_string().contains("panel version 7"));
    assert_eq!(state.code(), CALYX_GUARD_CALIBRATION_SLOT_STATE);
    assert!(state.to_string().contains("slot 3"));
    assert!(state.to_string().contains("parked"));
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_ERROR_FSV_DIR"]
fn ward_error_fsv_fixture_writes_readback_artifacts() {
    let root =
        std::env::var("CALYX_WARD_ERROR_FSV_DIR").expect("CALYX_WARD_ERROR_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let pass = slot_verdict(1, 0.92, 0.70, true);
    let fail = slot_verdict(2, 0.40, 0.70, false);
    let verdict = GuardVerdict {
        guard_id: guard_id(),
        overall_pass: false,
        provisional: false,
        per_slot: vec![pass, fail.clone()],
        action: Some(NoveltyAction::Quarantine),
    };
    let errors = [
        WardError::Ood {
            guard_id: guard_id(),
            failing: vec![fail],
        },
        WardError::Provisional {
            guard_id: guard_id(),
        },
        WardError::MissingSlotCalibration {
            guard_id: guard_id(),
            slot: slot(7),
        },
        WardError::InertProfile {
            guard_id: guard_id(),
            reason: "empty_required_slots",
        },
        WardError::MissingSlot { slot: slot(3) },
        WardError::PolicyViolation {
            k: 5,
            n_required: 3,
        },
        WardError::InsufficientCalibrationData { n: 49, min: 50 },
        WardError::InvalidRequiredSlotDerivation {
            reason: "no load-bearing slots for anchor",
        },
        WardError::NotAFailure {
            guard_id: guard_id(),
        },
        WardError::GuardIdMismatch {
            profile_guard_id: guard_id(),
            verdict_guard_id: other_guard_id(),
        },
        WardError::IdentitySlotNotRequired { slot: slot(9) },
        WardError::ModelNotFound {
            path: "/missing/wavlm.onnx".into(),
        },
        WardError::InvalidInput {
            reason: "empty audio".to_string(),
        },
        WardError::ModelDimMismatch {
            expected: 256,
            actual: 128,
        },
        WardError::Runtime {
            reason: "synthetic ONNX failure".to_string(),
        },
        WardError::NoveltySink {
            reason: "synthetic write failure".to_string(),
        },
    ];
    let error_readback: Vec<_> = errors
        .iter()
        .map(|error| {
            println!("FSV_ERROR_CODE={} MESSAGE={}", error.code(), error);
            json!({
                "code": error.code(),
                "message": error.to_string(),
            })
        })
        .collect();

    write_json(&root, "verdict.json", &verdict);
    write_json(&root, "errors.json", &error_readback);
}

fn write_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
    let path = std::path::Path::new(root).join(name);
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

fn other_guard_id() -> GuardId {
    "118f48a4-9a79-74d2-8a5c-9ad7f6b8c101"
        .parse()
        .expect("other guard id")
}

fn slot_verdict(slot_id: u16, cos: f32, tau: f32, pass: bool) -> SlotVerdict {
    SlotVerdict {
        slot: slot(slot_id),
        cos,
        tau,
        pass,
    }
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
