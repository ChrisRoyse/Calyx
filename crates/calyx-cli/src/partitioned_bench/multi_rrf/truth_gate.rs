use calyx_core::CalyxError;

use crate::error::{CliError, CliResult};

pub(super) fn enforce(required: bool, truth_n: usize, has_scale_truth: bool) -> CliResult {
    if !required || truth_n == 0 || has_scale_truth {
        return Ok(());
    }
    Err(CliError::Calyx(CalyxError {
        code: "CALYX_FSV_PARTITIONED_RRF_SCALE_TRUTH_REQUIRED",
        message:
            "gate-bearing partitioned-rrf recall requires byte-readable accepted-reference truth"
                .to_string(),
        remediation: "pass --fused-ground-truth-file with its manifest or --slot-ground-truth-manifest; CPU brute force is diagnostic-only",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_rejects_diagnostic_cpu_truth() {
        let err = enforce(true, 4, false).unwrap_err();

        assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RRF_SCALE_TRUTH_REQUIRED");
    }

    #[test]
    fn gate_accepts_scale_truth() {
        enforce(true, 4, true).unwrap();
    }

    #[test]
    fn diagnostic_without_recall_floor_can_use_cpu_truth() {
        enforce(false, 4, false).unwrap();
    }
}
