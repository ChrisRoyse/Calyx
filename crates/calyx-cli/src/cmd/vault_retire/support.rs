use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::CalyxError;

use super::{CLOCK_FAILED_CODE, REMEDIATION};
use crate::error::{CliError, CliResult};

pub(super) fn relative_to_home(home: &Path, path: &Path) -> String {
    path.strip_prefix(home)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub(super) fn index_path(home: &Path) -> PathBuf {
    home.join("vaults").join("index.json")
}

pub(super) fn retire_error(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation: REMEDIATION,
    })
}

pub(super) fn now_ms() -> CliResult<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            retire_error(
                CLOCK_FAILED_CODE,
                format!("system clock is before UNIX_EPOCH: {error}"),
            )
        })?;
    Ok(duration.as_millis() as u64)
}
