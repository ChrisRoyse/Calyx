//! Sextant-local fail-closed error helpers.

use calyx_core::CalyxError;

pub const CALYX_SEXTANT_PLAN_UNBOUNDED: &str = "CALYX_SEXTANT_PLAN_UNBOUNDED";
pub const CALYX_SEXTANT_RERANKER_TIMEOUT: &str = "CALYX_SEXTANT_RERANKER_TIMEOUT";
pub const CALYX_SEXTANT_SLOT_MISSING: &str = "CALYX_SEXTANT_SLOT_MISSING";
pub const CALYX_SEXTANT_VECTOR_SHAPE: &str = "CALYX_SEXTANT_VECTOR_SHAPE";

pub fn sextant_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    let remediation = match code {
        CALYX_SEXTANT_PLAN_UNBOUNDED => "tighten k/ef/slot limits or raise operator cap",
        CALYX_SEXTANT_RERANKER_TIMEOUT => "retry after reranker health is restored",
        CALYX_SEXTANT_SLOT_MISSING => "register or rebuild the requested slot index",
        CALYX_SEXTANT_VECTOR_SHAPE => "submit a vector matching the slot index shape",
        _ => "inspect Sextant query/index state",
    };
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
