//! Loom-local fail-closed error helpers.

use calyx_core::CalyxError;

pub const CALYX_LOOM_ZERO_NORM_VECTOR: &str = "CALYX_LOOM_ZERO_NORM_VECTOR";
pub const CALYX_LOOM_DIM_MISMATCH: &str = "CALYX_LOOM_DIM_MISMATCH";
pub const CALYX_LOOM_NON_FINITE_VECTOR: &str = "CALYX_LOOM_NON_FINITE_VECTOR";
pub const CALYX_LOOM_SLOT_MISSING: &str = "CALYX_LOOM_SLOT_MISSING";
pub const CALYX_LOOM_FORGE_UNAVAILABLE: &str = "CALYX_LOOM_FORGE_UNAVAILABLE";

pub fn loom_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    let remediation = match code {
        CALYX_LOOM_ZERO_NORM_VECTOR => "supply non-zero slot vectors before weaving agreements",
        CALYX_LOOM_DIM_MISMATCH => "use slot vectors with matching dimensions for this xterm",
        CALYX_LOOM_NON_FINITE_VECTOR => "remove NaN or infinite values from slot vectors",
        CALYX_LOOM_SLOT_MISSING => "load the requested cx/slot vectors before computing xterms",
        CALYX_LOOM_FORGE_UNAVAILABLE => "enable Loom's cuda feature and verify Forge CUDA first",
        _ => "inspect Loom xterm inputs",
    };
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
