//! Daemon/CLI wrapper for Aster byte-level restore verification.

use std::path::Path;

pub use calyx_aster::verify_restore::VerifyRestoreReport;
use calyx_aster::verify_restore::{CALYX_ASTER_RESTORE_INVALID, verify_restore as verify_aster};

use crate::error::DaemonError;

/// Verifies a restored vault with the daemon's historical error taxonomy.
pub fn verify_restore(vault_path: &Path) -> Result<VerifyRestoreReport, DaemonError> {
    verify_aster(vault_path).map_err(|error| {
        if error.code == CALYX_ASTER_RESTORE_INVALID {
            DaemonError::config_invalid(error.message)
        } else {
            DaemonError::health_failed(error.to_string())
        }
    })
}
