use calyx_core::CalyxError;

pub const CALYX_TURSO_OPEN: &str = "CALYX_TURSO_OPEN";
pub const CALYX_TURSO_ENGINE_MISMATCH: &str = "CALYX_TURSO_ENGINE_MISMATCH";
pub const CALYX_TURSO_SCHEMA_MISMATCH: &str = "CALYX_TURSO_SCHEMA_MISMATCH";
pub const CALYX_TURSO_RECORD_INVALID: &str = "CALYX_TURSO_RECORD_INVALID";
pub const CALYX_TURSO_BATCH_LIMIT: &str = "CALYX_TURSO_BATCH_LIMIT";
pub const CALYX_TURSO_TRANSACTION: &str = "CALYX_TURSO_TRANSACTION";
pub const CALYX_TURSO_CHECKPOINT: &str = "CALYX_TURSO_CHECKPOINT";
pub const CALYX_TURSO_PROJECTION_MISMATCH: &str = "CALYX_TURSO_PROJECTION_MISMATCH";
pub const CALYX_TURSO_IMMUTABLE_PROJECTED: &str = "CALYX_TURSO_IMMUTABLE_PROJECTED";
pub const CALYX_TURSO_PERMISSIONS_UNSAFE: &str = "CALYX_TURSO_PERMISSIONS_UNSAFE";

pub(crate) fn open(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_TURSO_OPEN,
        message,
        "inspect the database path, permissions, and emitted Turso engine detail",
    )
}

pub(crate) fn engine(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_TURSO_ENGINE_MISMATCH,
        message,
        "open this database with the exact supported Turso engine version",
    )
}

pub(crate) fn schema(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_TURSO_SCHEMA_MISMATCH,
        message,
        "use a Calyx-initialized Turso database with the supported schema version",
    )
}

pub(crate) fn record(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_TURSO_RECORD_INVALID,
        message,
        "correct the named record field; invalid records are never partially committed",
    )
}

pub(crate) fn batch(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_TURSO_BATCH_LIMIT,
        message,
        "submit between 1 and MAX_BATCH_SIZE records per transaction",
    )
}

pub(crate) fn transaction(context: &str, source: impl std::fmt::Display) -> CalyxError {
    error(
        CALYX_TURSO_TRANSACTION,
        format!("{context}: {source}"),
        "inspect the named SQL operation and Turso detail; the transaction was rolled back",
    )
}

pub(crate) fn checkpoint(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_TURSO_CHECKPOINT,
        message,
        "close competing readers if named, inspect storage health, and retry the checkpoint",
    )
}

pub(crate) fn projection(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_TURSO_PROJECTION_MISMATCH,
        message,
        "inspect the Turso record, Aster constellation, and projection failure row before retrying",
    )
}

pub(crate) fn immutable_projected(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_TURSO_IMMUTABLE_PROJECTED,
        message,
        "ingest the changed measurement under a new record_id so the projected association remains immutable",
    )
}

pub(crate) fn permissions(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_TURSO_PERMISSIONS_UNSAFE,
        message,
        "place the database under a trusted directory and use regular, non-linked owner-only storage files",
    )
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
