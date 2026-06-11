//! Record-schema validation helpers.

use crate::CalyxError;

/// Record/schema boundary failure.
pub const CALYX_RECORD_SCHEMA_VIOLATION: &str = "CALYX_RECORD_SCHEMA_VIOLATION";

pub(crate) fn record_schema_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_RECORD_SCHEMA_VIOLATION,
        message: message.into(),
        remediation: "submit a constellation matching the record schema with finite values",
    }
}
