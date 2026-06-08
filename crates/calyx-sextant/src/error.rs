//! Sextant-local fail-closed error helpers.

use calyx_core::CalyxError;

pub const CALYX_SEXTANT_PLAN_UNBOUNDED: &str = "CALYX_SEXTANT_PLAN_UNBOUNDED";
pub const CALYX_SEXTANT_PLAN_COST_EXCEEDED: &str = "CALYX_SEXTANT_PLAN_COST_EXCEEDED";
pub const CALYX_SEXTANT_RERANKER_TIMEOUT: &str = "CALYX_SEXTANT_RERANKER_TIMEOUT";
pub const CALYX_SEXTANT_NO_LENSES: &str = "CALYX_SEXTANT_NO_LENSES";
pub const CALYX_SEXTANT_SLOT_ALREADY_REGISTERED: &str = "CALYX_SEXTANT_SLOT_ALREADY_REGISTERED";
pub const CALYX_SEXTANT_SLOT_MISSING: &str = "CALYX_SEXTANT_SLOT_MISSING";
pub const CALYX_SEXTANT_INDEX_EMPTY: &str = "CALYX_SEXTANT_INDEX_EMPTY";
pub const CALYX_SEXTANT_EF_TOO_SMALL: &str = "CALYX_SEXTANT_EF_TOO_SMALL";
pub const CALYX_SEXTANT_DIM_MISMATCH: &str = "CALYX_SEXTANT_DIM_MISMATCH";
pub const CALYX_SEXTANT_VECTOR_SHAPE: &str = "CALYX_SEXTANT_VECTOR_SHAPE";

pub fn sextant_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    let remediation = match code {
        CALYX_SEXTANT_PLAN_UNBOUNDED => "tighten k/ef/slot limits or raise operator cap",
        CALYX_SEXTANT_PLAN_COST_EXCEEDED => "reduce k, ef, participating slots, or index scope",
        CALYX_SEXTANT_RERANKER_TIMEOUT => "retry after reranker health is restored",
        CALYX_SEXTANT_NO_LENSES => "register at least one slot index before planning or searching",
        CALYX_SEXTANT_SLOT_ALREADY_REGISTERED => {
            "use a distinct SlotId or rebuild the existing slot"
        }
        CALYX_SEXTANT_SLOT_MISSING => "register or rebuild the requested slot index",
        CALYX_SEXTANT_INDEX_EMPTY => "insert or rebuild at least one vector before searching",
        CALYX_SEXTANT_EF_TOO_SMALL => "set ef greater than or equal to requested result count",
        CALYX_SEXTANT_DIM_MISMATCH => "submit a query vector matching the slot dimension",
        CALYX_SEXTANT_VECTOR_SHAPE => "submit a vector matching the slot index shape",
        _ => "inspect Sextant query/index state",
    };
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
