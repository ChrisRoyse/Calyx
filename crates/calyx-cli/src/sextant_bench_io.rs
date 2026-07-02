//! JSON artifact I/O for the sextant bench commands: pretty-printed report
//! files and stdout dumps, with explicit error classification (issue #1145) —
//! serializer failures are runtime errors, never usage errors.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::error::{CliError, CliResult};

/// Writes `value` as pretty-printed JSON, creating parent directories.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(serialize_error)?;
    fs::write(path, bytes)?;
    Ok(())
}

/// Prints `value` as pretty-printed JSON on stdout.
pub(crate) fn print_json<T: Serialize>(value: &T) -> CliResult {
    let text = serde_json::to_string_pretty(value).map_err(serialize_error)?;
    println!("{text}");
    Ok(())
}

fn serialize_error(error: serde_json::Error) -> CliError {
    CliError::runtime(format!("serialize json report: {error}"))
}
