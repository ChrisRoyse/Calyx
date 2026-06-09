//! Ward error catalog with fail-closed Calyx codes.

use std::error::Error;
use std::fmt;

use calyx_core::SlotId;

use crate::profile::GuardId;
use crate::verdict::SlotVerdict;

pub const CALYX_GUARD_OOD: &str = "CALYX_GUARD_OOD";
pub const CALYX_GUARD_PROVISIONAL: &str = "CALYX_GUARD_PROVISIONAL";
pub const CALYX_GUARD_MISSING_SLOT: &str = "CALYX_GUARD_MISSING_SLOT";
pub const CALYX_GUARD_POLICY_VIOLATION: &str = "CALYX_GUARD_POLICY_VIOLATION";
pub const CALYX_GUARD_NOT_A_FAILURE: &str = "CALYX_GUARD_NOT_A_FAILURE";
pub const CALYX_GUARD_NOVELTY_SINK: &str = "CALYX_GUARD_NOVELTY_SINK";

/// Fail-closed errors emitted by Ward guard policy checks.
#[derive(Clone, Debug, PartialEq)]
pub enum WardError {
    Ood {
        guard_id: GuardId,
        failing: Vec<SlotVerdict>,
    },
    Provisional {
        guard_id: GuardId,
    },
    MissingSlot {
        slot: SlotId,
    },
    PolicyViolation {
        k: usize,
        n_required: usize,
    },
    InsufficientCalibrationData {
        n: usize,
        min: usize,
    },
    InvalidCalibrationInput {
        reason: &'static str,
    },
    InvalidRequiredSlotDerivation {
        reason: &'static str,
    },
    NotAFailure {
        guard_id: GuardId,
    },
    NoveltySink {
        reason: String,
    },
}

impl WardError {
    /// Returns the stable Calyx error code for this error.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Ood { .. } => CALYX_GUARD_OOD,
            Self::Provisional { .. } => CALYX_GUARD_PROVISIONAL,
            Self::InsufficientCalibrationData { .. }
            | Self::InvalidCalibrationInput { .. }
            | Self::InvalidRequiredSlotDerivation { .. } => CALYX_GUARD_PROVISIONAL,
            Self::MissingSlot { .. } => CALYX_GUARD_MISSING_SLOT,
            Self::PolicyViolation { .. } => CALYX_GUARD_POLICY_VIOLATION,
            Self::NotAFailure { .. } => CALYX_GUARD_NOT_A_FAILURE,
            Self::NoveltySink { .. } => CALYX_GUARD_NOVELTY_SINK,
        }
    }
}

impl fmt::Display for WardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ood { guard_id, failing } => {
                write!(f, "{CALYX_GUARD_OOD}: guard {guard_id} out of distribution")?;
                for slot in failing {
                    write!(f, "; slot {} cos={} tau={}", slot.slot, slot.cos, slot.tau)?;
                }
                Ok(())
            }
            Self::Provisional { guard_id } => write!(
                f,
                "{CALYX_GUARD_PROVISIONAL}: guard {guard_id} is uncalibrated; calibrate before high-stakes use -- run calibrate() with an anchored set >=50 examples"
            ),
            Self::MissingSlot { slot } => {
                write!(
                    f,
                    "{CALYX_GUARD_MISSING_SLOT}: required slot {slot} is missing"
                )
            }
            Self::PolicyViolation { k, n_required } => write!(
                f,
                "{CALYX_GUARD_POLICY_VIOLATION}: KofN k={k} exceeds required slot count n_required={n_required}"
            ),
            Self::InsufficientCalibrationData { n, min } => write!(
                f,
                "{CALYX_GUARD_PROVISIONAL}: insufficient calibration data n={n} min={min}"
            ),
            Self::InvalidCalibrationInput { reason } => write!(
                f,
                "{CALYX_GUARD_PROVISIONAL}: invalid calibration input: {reason}"
            ),
            Self::InvalidRequiredSlotDerivation { reason } => write!(
                f,
                "{CALYX_GUARD_PROVISIONAL}: invalid required-slot derivation: {reason}"
            ),
            Self::NotAFailure { guard_id } => write!(
                f,
                "{CALYX_GUARD_NOT_A_FAILURE}: guard {guard_id} verdict already passed; novelty handling requires a failed verdict"
            ),
            Self::NoveltySink { reason } => {
                write!(f, "{CALYX_GUARD_NOVELTY_SINK}: {reason}")
            }
        }
    }
}

impl Error for WardError {}

#[cfg(test)]
mod tests {
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
        let sink = WardError::NoveltySink {
            reason: "synthetic write failure".to_string(),
        };

        assert_eq!(not_failure.code(), CALYX_GUARD_NOT_A_FAILURE);
        assert!(not_failure.to_string().contains("novelty handling"));
        assert_eq!(sink.code(), CALYX_GUARD_NOVELTY_SINK);
        assert!(sink.to_string().contains("synthetic write failure"));
    }

    #[test]
    #[ignore = "manual aiwonder FSV fixture; set CALYX_WARD_ERROR_FSV_DIR"]
    fn ward_error_fsv_fixture_writes_readback_artifacts() {
        let root = std::env::var("CALYX_WARD_ERROR_FSV_DIR")
            .expect("CALYX_WARD_ERROR_FSV_DIR is required");
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
}
