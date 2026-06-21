//! Daemon error taxonomy mapping to stable `CALYX_*` codes (PH65).
//!
//! Every variant carries a remediation hint (A16): `Display` always renders
//! `<code>: <detail> (remediation: <hint>)`, so an operator reading stderr or a
//! log line gets the stable code, the specific context, and the next action in
//! one string. Server mode fails loud — there is no silent/error-free path.

use std::fmt;

use calyx_core::CALYX_TLS_CONFIG_INVALID;

/// Fail-closed daemon startup/runtime errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonError {
    /// Refused to bind a non-loopback address or the OS bind failed.
    BindFailed { detail: String },
    /// Invalid CLI arguments, config file, or verify-target paths.
    ConfigInvalid { detail: String },
    /// Invalid TLS/mTLS material or policy.
    TlsConfigInvalid { detail: String },
    /// VRAM budget out of the daemon's accepted range (`0 < x <= ceiling`).
    VramBudget { detail: String },
    /// CUDA device init failed (or was force-failed for FSV). Server mode is
    /// fatal on this — never a silent CPU fallback.
    DeviceUnavailable { detail: String },
    /// A healthcheck probe (CUDA / VRAM / vault read) did not reach a healthy
    /// state. Used by the `calyxd::health` daemon-readiness probe (T04) when the
    /// failure is not already covered by a more specific `CALYX_*` code (e.g. a
    /// vault that is present but does not verify on read-back).
    HealthFailed { detail: String },
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

    pub fn tls_config_invalid(detail: impl Into<String>) -> Self {
        Self::TlsConfigInvalid {
            detail: detail.into(),
        }
    }

    pub fn vram_budget(detail: impl Into<String>) -> Self {
        Self::VramBudget {
            detail: detail.into(),
        }
    }

    pub fn device_unavailable(detail: impl Into<String>) -> Self {
        Self::DeviceUnavailable {
            detail: detail.into(),
        }
    }

    pub fn health_failed(detail: impl Into<String>) -> Self {
        Self::HealthFailed {
            detail: detail.into(),
        }
    }

    /// Stable wire code for the error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BindFailed { .. } => "CALYX_DAEMON_BIND_FAILED",
            Self::ConfigInvalid { .. } => "CALYX_DAEMON_CONFIG_INVALID",
            Self::TlsConfigInvalid { .. } => CALYX_TLS_CONFIG_INVALID,
            Self::VramBudget { .. } => "CALYX_FORGE_VRAM_BUDGET",
            Self::DeviceUnavailable { .. } => "CALYX_FORGE_DEVICE_UNAVAILABLE",
            Self::HealthFailed { .. } => "CALYX_DAEMON_HEALTH_FAIL",
        }
    }

    /// Operator remediation hint (A16: every structured error carries one).
    /// Kept in one place so `Display` never double-renders it and call sites
    /// supply only the specific `detail`.
    pub fn remediation(&self) -> &'static str {
        match self {
            Self::BindFailed { .. } => {
                "set bind_addr to a loopback address (127.0.0.1 or [::1]) in calyx.toml"
            }
            Self::ConfigInvalid { .. } => {
                "fix the calyx.toml key or CLI argument named in the detail and retry"
            }
            Self::TlsConfigInvalid { .. } => {
                "point mcp_mtls cert_pem_path/key_pem_path/ca_pem_path at readable PEM files and require client certificates"
            }
            Self::VramBudget { .. } => {
                "lower vram_budget_mib in calyx.toml or free resident GPU memory, then retry"
            }
            Self::DeviceUnavailable { .. } => {
                "ensure an NVIDIA CUDA GPU + driver are present and calyxd was built with \
                 --features cuda; server mode requires a working GPU and will not start without one"
            }
            Self::HealthFailed { .. } => {
                "inspect the failing probe named in the detail (CUDA / VRAM / vault read), fix the \
                 underlying cause, then re-run `calyx healthcheck`; the daemon is not healthy until it passes"
            }
        }
    }

    /// The variant-specific context string.
    pub fn detail(&self) -> &str {
        match self {
            Self::BindFailed { detail }
            | Self::ConfigInvalid { detail }
            | Self::TlsConfigInvalid { detail }
            | Self::VramBudget { detail }
            | Self::DeviceUnavailable { detail }
            | Self::HealthFailed { detail } => detail,
        }
    }
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} (remediation: {})",
            self.code(),
            self.detail(),
            self.remediation()
        )
    }
}

impl std::error::Error for DaemonError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_failed_displays_code_detail_and_remediation() {
        let error = DaemonError::bind_failed("refused 0.0.0.0:7700");
        assert_eq!(error.code(), "CALYX_DAEMON_BIND_FAILED");
        let shown = error.to_string();
        assert!(shown.contains("CALYX_DAEMON_BIND_FAILED"));
        assert!(shown.contains("refused 0.0.0.0:7700"));
        assert!(shown.contains("remediation:"));
    }

    #[test]
    fn config_invalid_displays_code_detail_and_remediation() {
        let error = DaemonError::config_invalid("missing --vault");
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        let shown = error.to_string();
        assert!(shown.contains("CALYX_DAEMON_CONFIG_INVALID"));
        assert!(shown.contains("missing --vault"));
        assert!(shown.contains("remediation:"));
    }

    #[test]
    fn vram_budget_displays_code_detail_and_remediation() {
        let error = DaemonError::vram_budget("vram_budget_mib 0 out of range");
        assert_eq!(error.code(), "CALYX_FORGE_VRAM_BUDGET");
        assert!(error.to_string().contains("CALYX_FORGE_VRAM_BUDGET"));
        assert!(error.to_string().contains("out of range"));
        assert!(error.to_string().contains("remediation:"));
    }

    #[test]
    fn tls_config_invalid_displays_code_detail_and_remediation() {
        let error = DaemonError::tls_config_invalid("mcp_mtls.ca_pem_path missing");
        assert_eq!(error.code(), CALYX_TLS_CONFIG_INVALID);
        let shown = error.to_string();
        assert!(shown.contains(CALYX_TLS_CONFIG_INVALID));
        assert!(shown.contains("ca_pem_path"));
        assert!(shown.contains("remediation:"));
    }

    #[test]
    fn device_unavailable_displays_code_and_remediation() {
        let error = DaemonError::device_unavailable("CUDA init on device 0 failed: no device");
        assert_eq!(error.code(), "CALYX_FORGE_DEVICE_UNAVAILABLE");
        let shown = error.to_string();
        assert!(shown.contains("CALYX_FORGE_DEVICE_UNAVAILABLE"));
        assert!(shown.contains("remediation:"));
        // Fail-loud guidance points at the GPU/build, never at a CPU fallback.
        assert!(shown.contains("--features cuda"));
        assert!(shown.contains("GPU"));
    }

    #[test]
    fn health_failed_displays_code_detail_and_remediation() {
        let error = DaemonError::health_failed("vault read-back unverified: chain not intact");
        assert_eq!(error.code(), "CALYX_DAEMON_HEALTH_FAIL");
        let shown = error.to_string();
        assert!(shown.contains("CALYX_DAEMON_HEALTH_FAIL"));
        assert!(shown.contains("chain not intact"));
        assert!(shown.contains("remediation:"));
    }

    #[test]
    fn every_variant_display_carries_a_nonempty_remediation() {
        let variants = [
            DaemonError::bind_failed("d"),
            DaemonError::config_invalid("d"),
            DaemonError::tls_config_invalid("d"),
            DaemonError::vram_budget("d"),
            DaemonError::device_unavailable("d"),
            DaemonError::health_failed("d"),
        ];
        for error in &variants {
            assert!(
                format!("{error}").contains("remediation:"),
                "{} must render a remediation",
                error.code()
            );
            assert!(!error.remediation().is_empty());
        }
    }
}
