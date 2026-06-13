use std::path::Path;

use calyx_core::CalyxError;

pub const CALYX_MIGRATE_SQLITE_SCHEMA: &str = "CALYX_MIGRATE_SQLITE_SCHEMA";
pub const CALYX_MIGRATE_EMBEDDING_FORMAT: &str = "CALYX_MIGRATE_EMBEDDING_FORMAT";
pub const CALYX_MIGRATE_MANIFEST: &str = "CALYX_MIGRATE_MANIFEST";
pub const CALYX_MIGRATE_VERIFY_MISMATCH: &str = "CALYX_MIGRATE_VERIFY_MISMATCH";
pub const CALYX_MIGRATE_BACKFILL_INCOMPLETE: &str = "CALYX_MIGRATE_BACKFILL_INCOMPLETE";

pub fn schema(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_MIGRATE_SQLITE_SCHEMA,
        message,
        "provide a Leapable Vault SQLite DB with chunks(chunk_id,database_name,content,embedding)",
    )
}

pub fn embedding(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_MIGRATE_EMBEDDING_FORMAT,
        message,
        "embedding must be raw little-endian finite f32 values",
    )
}

pub fn manifest(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_MIGRATE_MANIFEST,
        message,
        "repair or regenerate the migration manifest next to the vault",
    )
}

pub fn backfill_incomplete(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_MIGRATE_BACKFILL_INCOMPLETE,
        message,
        "run calyx migrate backfill until every default-panel slot has a row",
    )
}

pub fn verify_mismatch(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_MIGRATE_VERIFY_MISMATCH,
        message,
        "re-run migration from the source SQLite DB and inspect the named row",
    )
}

pub fn io(context: &str, path: &Path, err: std::io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context} {}: {err}", path.display()))
}

pub fn sqlite(context: &str, err: rusqlite::Error) -> CalyxError {
    schema(format!("{context}: {err}"))
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
