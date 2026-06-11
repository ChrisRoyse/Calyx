use calyx_core::CalyxError;
use calyx_forge::ForgeError;

use crate::CALYX_ANNEAL_BUDGET_EXHAUSTED;

use super::types::{
    CALYX_ANNEAL_AB_CACHE_WRITE_FAIL, CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE,
    CALYX_ANNEAL_TRIAL_INVALID_RESULT, CALYX_ANNEAL_TRIAL_NOT_ACTIVE,
};

pub(super) fn already_active(label: String) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE,
        message: format!("A/B trial already active for {label}"),
        remediation: "finish or abandon the current A/B trial before starting another",
    }
}

pub(super) fn not_active(label: String) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_TRIAL_NOT_ACTIVE,
        message: format!("no A/B trial active for {label}"),
        remediation: "start an A/B trial before recording query results",
    }
}

pub(super) fn invalid_result(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_TRIAL_INVALID_RESULT,
        message: message.into(),
        remediation: "drop the invalid A/B sample and re-measure the live request",
    }
}

pub(super) fn budget_exhausted(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_BUDGET_EXHAUSTED,
        message: message.into(),
        remediation: "wait for background budget or reduce A/B shadow work",
    }
}

pub(super) fn cache_write_fail(error: ForgeError) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_AB_CACHE_WRITE_FAIL,
        message: format!("A/B promotion cache write failed: {error}"),
        remediation: "repair the PH16 autotune cache path before promoting the A/B candidate",
    }
}
