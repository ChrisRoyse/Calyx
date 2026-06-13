use calyx_core::CalyxError;

use crate::collection::CALYX_INVALID_ARGUMENT;

pub const CALYX_SCHEMA_VIOLATION: &str = "CALYX_SCHEMA_VIOLATION";

pub(super) fn schema_violation(message: impl Into<String>) -> CalyxError {
    document_error(
        CALYX_SCHEMA_VIOLATION,
        message,
        "submit a document matching the collection SchemaFull definition",
    )
}

pub(super) fn invalid_argument(message: impl Into<String>) -> CalyxError {
    document_error(CALYX_INVALID_ARGUMENT, message, "fix the document input")
}

pub(super) fn corrupt_doc(message: impl Into<String>) -> CalyxError {
    CalyxError::aster_corrupt_shard(message)
}

fn document_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
