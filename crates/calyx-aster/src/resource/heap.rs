//! Heap RSS probe over `/proc/self/status` (fail-closed off Linux).

use calyx_core::{CalyxError, Result};

/// Stable code for resource probes that cannot run on this host.
pub const CALYX_RESOURCE_PROBE_UNAVAILABLE: &str = "CALYX_RESOURCE_PROBE_UNAVAILABLE";

pub(crate) fn probe_unavailable(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_RESOURCE_PROBE_UNAVAILABLE,
        message: message.into(),
        remediation: "run resource_status on a Linux host with /proc mounted",
    }
}

/// Reads the resident set size of this process in bytes.
///
/// Source of truth is the kernel `VmRSS` line in `/proc/self/status`
/// (`proc_pid_status(5)`). There is no cross-platform fallback: on hosts
/// without `/proc` this fails closed with `CALYX_RESOURCE_PROBE_UNAVAILABLE`.
pub fn heap_rss_bytes() -> Result<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/self/status")
            .map_err(|error| probe_unavailable(format!("read /proc/self/status: {error}")))?;
        parse_vm_rss_bytes(&text)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(probe_unavailable(
            "heap RSS probe requires /proc/self/status (Linux)",
        ))
    }
}

/// Parses the `VmRSS:` line of a `/proc/<pid>/status` document into bytes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn parse_vm_rss_bytes(status_text: &str) -> Result<u64> {
    for line in status_text.lines() {
        let Some(rest) = line.strip_prefix("VmRSS:") else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let value = fields
            .next()
            .ok_or_else(|| probe_unavailable("VmRSS line has no value field"))?;
        let unit = fields
            .next()
            .ok_or_else(|| probe_unavailable("VmRSS line has no unit field"))?;
        if unit != "kB" {
            return Err(probe_unavailable(format!(
                "VmRSS unit {unit:?} is not kB; refusing to guess a scale"
            )));
        }
        let kib = value
            .parse::<u64>()
            .map_err(|error| probe_unavailable(format!("parse VmRSS value {value:?}: {error}")))?;
        return Ok(kib.saturating_mul(1024));
    }
    Err(probe_unavailable(
        "VmRSS line not found in /proc/self/status",
    ))
}
