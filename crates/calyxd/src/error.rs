//! Daemon error taxonomy mapping to stable `CALYX_DAEMON_*` codes (PH65).

use std::fmt;

/// Fail-closed daemon startup/runtime errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonError {
    /// Refused to bind a non-loopback address or the OS bind failed.
    BindFailed { detail: String },
    /// Invalid CLI arguments or verify-target paths.
    ConfigInvalid { detail: String },
}

impl DaemonError {
    pub fn bind_failed(detail: impl Into<String>) -> Self {
        Self::BindFailed {
            detail: detail.into(),
        }
    }

    pub fn config_invalid(detail: impl Into<String>) -> Self {
        Self::ConfigInvalid {
            detail: detail.into(),
        }
    }

    /// Stable wire code for the error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BindFailed { .. } => "CALYX_DAEMON_BIND_FAILED",
            Self::ConfigInvalid { .. } => "CALYX_DAEMON_CONFIG_INVALID",
        }
    }
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let detail = match self {
            Self::BindFailed { detail } | Self::ConfigInvalid { detail } => detail,
        };
        write!(f, "{}: {detail}", self.code())
    }
}

impl std::error::Error for DaemonError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_failed_displays_stable_code_and_detail() {
        let error = DaemonError::bind_failed("refused 0.0.0.0:7700");
        assert_eq!(error.code(), "CALYX_DAEMON_BIND_FAILED");
        assert_eq!(
            error.to_string(),
            "CALYX_DAEMON_BIND_FAILED: refused 0.0.0.0:7700"
        );
    }

    #[test]
    fn config_invalid_displays_stable_code_and_detail() {
        let error = DaemonError::config_invalid("missing --vault");
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        assert_eq!(
            error.to_string(),
            "CALYX_DAEMON_CONFIG_INVALID: missing --vault"
        );
    }
}
