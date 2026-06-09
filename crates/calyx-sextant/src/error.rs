//! Sextant-local fail-closed error helpers.

use calyx_core::CalyxError;
pub use calyx_core::{
    CALYX_TEMPORAL_AP60_VIOLATION, CALYX_TEMPORAL_INVALID_PERIOD, CALYX_TEMPORAL_INVALID_WINDOW,
    CALYX_TEMPORAL_WEIGHT_SUM,
};

pub const CALYX_SEXTANT_PLAN_UNBOUNDED: &str = "CALYX_SEXTANT_PLAN_UNBOUNDED";
pub const CALYX_SEXTANT_PLAN_COST_EXCEEDED: &str = "CALYX_SEXTANT_PLAN_COST_EXCEEDED";
pub const CALYX_SEXTANT_RERANKER_TIMEOUT: &str = "CALYX_SEXTANT_RERANKER_TIMEOUT";
pub const CALYX_SEXTANT_NO_LENSES: &str = "CALYX_SEXTANT_NO_LENSES";
pub const CALYX_SEXTANT_SLOT_ALREADY_REGISTERED: &str = "CALYX_SEXTANT_SLOT_ALREADY_REGISTERED";
pub const CALYX_SEXTANT_SLOT_MISSING: &str = "CALYX_SEXTANT_SLOT_MISSING";
pub const CALYX_SEXTANT_SLOT_INACTIVE: &str = "CALYX_SEXTANT_SLOT_INACTIVE";
pub const CALYX_SEXTANT_INDEX_EMPTY: &str = "CALYX_SEXTANT_INDEX_EMPTY";
pub const CALYX_SEXTANT_EF_TOO_SMALL: &str = "CALYX_SEXTANT_EF_TOO_SMALL";
pub const CALYX_SEXTANT_DIM_MISMATCH: &str = "CALYX_SEXTANT_DIM_MISMATCH";
pub const CALYX_SEXTANT_VECTOR_SHAPE: &str = "CALYX_SEXTANT_VECTOR_SHAPE";
pub const CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE: &str = "CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE";
pub const CALYX_SEXTANT_POSTINGS_CORRUPT: &str = "CALYX_SEXTANT_POSTINGS_CORRUPT";
pub const CALYX_SEXTANT_POSTINGS_NOT_SORTED: &str = "CALYX_SEXTANT_POSTINGS_NOT_SORTED";
pub const CALYX_SEXTANT_PROVENANCE_MISSING: &str = "CALYX_SEXTANT_PROVENANCE_MISSING";
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
        CALYX_SEXTANT_SLOT_INACTIVE => "unpark the slot before measuring or searching it",
        CALYX_SEXTANT_INDEX_EMPTY => "insert or rebuild at least one vector before searching",
        CALYX_SEXTANT_EF_TOO_SMALL => "set ef greater than or equal to requested result count",
        CALYX_SEXTANT_DIM_MISMATCH => "submit a query vector matching the slot dimension",
        CALYX_SEXTANT_VECTOR_SHAPE => "submit a vector matching the slot index shape",
        CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE => {
            "wire a real Forge GPU path before claiming Sextant CPU/GPU parity"
        }
        CALYX_SEXTANT_POSTINGS_CORRUPT => "discard/rebuild the sparse postings block",
        CALYX_SEXTANT_POSTINGS_NOT_SORTED => "sort postings by increasing document id",
        CALYX_SEXTANT_PROVENANCE_MISSING => {
            "attach the stored constellation before requiring provenance"
        }
        CALYX_TEMPORAL_AP60_VIOLATION => {
            "keep temporal signals post-retrieval only and never dominant"
        }
        CALYX_TEMPORAL_INVALID_PERIOD => "set target_hour 0..=23 and day_of_week 0..=6",
        CALYX_TEMPORAL_INVALID_WINDOW => "set a non-empty temporal window within i64 bounds",
        CALYX_TEMPORAL_WEIGHT_SUM => "normalize recency + sequence + periodic to exactly 1.0",
        _ => "inspect Sextant query/index state",
    };
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
