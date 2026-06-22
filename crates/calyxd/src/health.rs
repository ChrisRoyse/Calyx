//! PH65 · T04 — `calyxd` daemon-readiness healthcheck.
//!
//! [`run_healthcheck`] performs the real two-part operational probe that the
//! systemd unit (PH66) runs as `ExecStartPost`:
//!
//! 1. **CUDA init** via the Forge probe ([`crate::cuda_probe::probe_cuda_device`]),
//!    with the configured VRAM budget honored against live NVML usage
//!    ([`crate::vram`]). No silent CPU fallback — a missing/failed GPU is fatal
//!    to health.
//! 2. **A real Aster vault read-back** ([`crate::verify::verify_restore`]): every
//!    constellation, anchor, and ledger link is physically scanned and the first
//!    constellation is decoded back completely. This is not a ping.
//!
//! The verdict is written atomically to the configured `health_log_path` as a
//! [`CalyxHealthResult`] JSON whose `.status` is the literal `"pass"` or
//! `"fail"`. There is no silent success: any probe failure records its exact
//! `CALYX_*` code under `.error_code`, sets `.status = "fail"`, and (via the CLI)
//! exits non-zero. The `--wait` retry loop tolerates a slow CUDA init by
//! re-probing at 1-second intervals.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::config::CalyxConfig;
use crate::cuda_probe::probe_cuda_device;
use crate::error::DaemonError;
use crate::vram::{NvmlVramUsage, VramBudget};

/// The healthcheck source-of-truth record written to `health_log_path`.
///
/// `status` is the literal `"pass"` (every probe healthy) or `"fail"` (at least
/// one probe failed). On failure `error_code`/`error_detail` carry the exact
/// `CALYX_*` code and context of the first failing probe, while `cuda_device`
/// (`None`) and `vault_read_ok` (`false`) independently record each subsystem's
/// state — so a multi-probe failure is fully visible, never collapsed to one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalyxHealthResult {
    /// `"pass"` or `"fail"` — the at-a-glance verdict for monitoring.
    pub status: &'static str,
    /// ISO-8601 UTC timestamp (e.g. `2026-06-13T17:04:05Z`) of the probe.
    pub timestamp_utc: String,
    /// CUDA device name when the device probe succeeded; `None` on failure.
    pub cuda_device: Option<String>,
    /// Configured Forge VRAM ceiling in MiB (the authoritative `calyx.toml` budget).
    pub vram_budget_mib: u32,
    /// Whether the real Aster vault read-back verified intact with data present.
    pub vault_read_ok: bool,
    /// Exact `CALYX_*` code of the first failing probe; `None` when healthy.
    pub error_code: Option<String>,
    /// Context for the first failing probe; `None` when healthy.
    pub error_detail: Option<String>,
}

/// Clean-shutdown source-of-truth record written over `health_log_path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalyxShutdownResult {
    /// Literal shutdown verdict consumed by monitoring.
    pub status: &'static str,
    /// ISO-8601 UTC timestamp of the clean shutdown.
    pub timestamp_utc: String,
}

impl CalyxHealthResult {
    /// True only when every probe passed.
    pub fn is_pass(&self) -> bool {
        self.status == "pass"
    }
}

/// Run the full daemon-readiness probe once. Never panics and never returns an
/// `Err`: every failure mode is encoded into the returned [`CalyxHealthResult`]
/// (fail-closed), so the caller always has a complete record to write.
pub fn run_healthcheck(cfg: &CalyxConfig) -> CalyxHealthResult {
    let timestamp_utc = iso8601_from_unix_secs(unix_secs_now());
    let vram_budget_mib = cfg.vram_budget_mib;

    // Probe 1: CUDA device. The full info (not just the name) is needed to
    // validate the VRAM budget below.
    let cuda = probe_cuda_device();
    let cuda_device = cuda.as_ref().ok().map(|info| info.device_name.clone());
    // First failure wins for the single `error_code` field; the per-subsystem
    // fields (`cuda_device`, `vault_read_ok`) still record every outcome.
    let mut error: Option<DaemonError> = cuda.as_ref().err().cloned();

    // Probe 1b: VRAM budget honored against live NVML usage. Only meaningful
    // with a live device + driver; skipped (not faked) when CUDA is down.
    if let Ok(device) = &cuda {
        let audit = NvmlVramUsage::init()
            .and_then(|nvml| VramBudget::from_config(vram_budget_mib, device, nvml))
            .and_then(|budget| budget.startup_vram_audit());
        if let Err(vram_error) = audit {
            error.get_or_insert(vram_error);
        }
    }

    // Probe 2: real Aster vault read-back. ANY failure (unreadable path or a
    // present-but-unverified vault) maps to CALYX_DAEMON_HEALTH_FAIL, preserving
    // the underlying cause in the detail — never a panic, never a silent pass.
    let vault_path = cfg.vault_path_resolved();
    let (vault_read_ok, vault_error) = probe_vault(&vault_path);
    if let Some(vault_error) = vault_error {
        error.get_or_insert(vault_error);
    }

    let status = if error.is_none() { "pass" } else { "fail" };
    let (error_code, error_detail) = match &error {
        Some(error) => (
            Some(error.code().to_string()),
            Some(error.detail().to_string()),
        ),
        None => (None, None),
    };
    CalyxHealthResult {
        status,
        timestamp_utc,
        cuda_device,
        vram_budget_mib,
        vault_read_ok,
        error_code,
        error_detail,
    }
}

/// Real read-back of the configured vault. Returns `(vault_read_ok, error)`.
/// Every failure is wrapped as `CALYX_DAEMON_HEALTH_FAIL` so health failures are
/// unambiguous regardless of the underlying read error's own code.
fn probe_vault(vault_path: &Path) -> (bool, Option<DaemonError>) {
    match crate::verify::verify_restore(vault_path) {
        Ok(report) if report.success() => (true, None),
        Ok(report) => (
            false,
            Some(DaemonError::health_failed(format!(
                "vault {} read-back unverified: {}",
                vault_path.display(),
                report.failure_reasons().join("; ")
            ))),
        ),
        Err(error) => (
            false,
            Some(DaemonError::health_failed(format!(
                "vault {} unreadable: {error}",
                vault_path.display()
            ))),
        ),
    }
}

/// Atomically write the result to `path` as pretty JSON.
///
/// Creates the parent directory, writes to a temp file **in the same directory**
/// (so the rename is same-filesystem — no `EXDEV` across ZFS datasets, per
/// `01 §4`), then `rename`s it into place. A reader therefore never observes a
/// half-written file.
pub fn write_health_result(result: &CalyxHealthResult, path: &Path) -> Result<(), DaemonError> {
    write_json_atomic(result, path)
}

/// Atomically write `{"status":"shutdown","timestamp_utc":"…"}` to `path`.
pub fn write_shutdown_status(path: &Path) -> Result<CalyxShutdownResult, DaemonError> {
    let result = CalyxShutdownResult {
        status: "shutdown",
        timestamp_utc: iso8601_from_unix_secs(unix_secs_now()),
    };
    write_json_atomic(&result, path)?;
    Ok(result)
}

fn write_json_atomic<T: Serialize>(value: &T, path: &Path) -> Result<(), DaemonError> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent).map_err(|error| {
            DaemonError::health_failed(format!(
                "create health log dir {}: {error}",
                parent.display()
            ))
        })?;
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| DaemonError::health_failed(format!("serialize health result: {error}")))?;
    let tmp = temp_sibling(path);
    fs::write(&tmp, format!("{json}\n")).map_err(|error| {
        DaemonError::health_failed(format!("write temp health file {}: {error}", tmp.display()))
    })?;
    fs::rename(&tmp, path).map_err(|error| {
        // Best-effort cleanup so a failed rename does not litter temp files.
        let _ = fs::remove_file(&tmp);
        DaemonError::health_failed(format!(
            "atomic rename {} -> {}: {error}",
            tmp.display(),
            path.display()
        ))
    })
}

/// A temp path beside `path` (same directory ⇒ same filesystem). The PID keeps
/// concurrent daemons from colliding; the systemd `ExecStartPost` invocation is
/// single-shot so a counter is unnecessary.
fn temp_sibling(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "health".to_string());
    let tmp_name = format!(".{name}.{}.tmp", std::process::id());
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => parent.join(tmp_name),
        None => PathBuf::from(tmp_name),
    }
}

/// Run `attempt` up to `wait_secs + 1` times at 1-second intervals, returning
/// the first `"pass"` result with the attempt count, or the last result if all
/// attempts failed. `wait_secs == 0` ⇒ exactly one attempt (no retry loop).
pub fn run_with_wait<F>(wait_secs: u32, attempt: F) -> (CalyxHealthResult, u32)
where
    F: FnMut() -> CalyxHealthResult,
{
    run_with_wait_interval(wait_secs, Duration::from_secs(1), attempt)
}

/// [`run_with_wait`] with an injectable sleep interval so tests exercise the
/// retry/attempt-count logic without sleeping for real seconds.
fn run_with_wait_interval<F>(
    wait_secs: u32,
    interval: Duration,
    mut attempt: F,
) -> (CalyxHealthResult, u32)
where
    F: FnMut() -> CalyxHealthResult,
{
    let max_attempts = wait_secs.saturating_add(1);
    let mut last: Option<(CalyxHealthResult, u32)> = None;
    for n in 1..=max_attempts {
        let result = attempt();
        eprintln!(
            "INFO healthcheck: attempt {n}/{max_attempts} status={}",
            result.status
        );
        let passed = result.is_pass();
        last = Some((result, n));
        if passed {
            break;
        }
        if n < max_attempts {
            std::thread::sleep(interval);
        }
    }
    last.expect("max_attempts >= 1 guarantees at least one attempt")
}

fn unix_secs_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Civil UTC date-time from a Unix timestamp, formatted ISO-8601 with a `Z`
/// suffix. Uses Howard Hinnant's `civil_from_days` algorithm — exact for every
/// date, no leap-second table, and dependency-free (the workspace has no
/// date/time crate). Deterministic: a fixed input yields a fixed string.
fn iso8601_from_unix_secs(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60,
    );

    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // day-of-era  [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year (Mar-based) [0, 365]
    let mp = (5 * doy + 2) / 153; // month-of-year (Mar-based) [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let civil_year = if month <= 2 { year + 1 } else { year };

    format!("{civil_year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(status: &'static str) -> CalyxHealthResult {
        CalyxHealthResult {
            status,
            timestamp_utc: "2026-06-13T00:00:00Z".to_string(),
            cuda_device: Some("NVIDIA CUDA GPU".to_string()),
            vram_budget_mib: 8192,
            vault_read_ok: status == "pass",
            error_code: (status != "pass").then(|| "CALYX_DAEMON_HEALTH_FAIL".to_string()),
            error_detail: (status != "pass").then(|| "synthetic".to_string()),
        }
    }

    #[test]
    fn pass_result_serializes_with_exact_json_keys() {
        let json = serde_json::to_value(result("pass")).expect("serialize");
        assert_eq!(json["status"], "pass");
        assert_eq!(json["vault_read_ok"], true);
        assert_eq!(json["vram_budget_mib"], 8192);
        assert_eq!(json["cuda_device"], "NVIDIA CUDA GPU");
        assert!(json["timestamp_utc"].is_string());
        // A healthy result carries no error fields.
        assert!(json["error_code"].is_null());
        assert!(json["error_detail"].is_null());
    }

    #[test]
    fn write_health_result_round_trips_and_creates_parent() {
        let dir = std::env::temp_dir().join(format!("calyx-health-write-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        // Parent does NOT exist yet — write_health_result must create it.
        let path = dir.join("nested").join("latest.json");
        write_health_result(&result("pass"), &path).expect("write");

        assert!(path.exists(), "health JSON must exist at the SoT path");
        let text = fs::read_to_string(&path).expect("read back");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["status"], "pass");
        // No temp sibling left behind after the atomic rename.
        let leftover = temp_sibling(&path);
        assert!(!leftover.exists(), "temp sibling must be renamed away");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_shutdown_status_overwrites_health_file_with_shutdown_json() {
        let dir =
            std::env::temp_dir().join(format!("calyx-health-shutdown-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("latest.json");
        write_health_result(&result("pass"), &path).expect("write pass");

        let shutdown = write_shutdown_status(&path).expect("write shutdown");

        assert_eq!(shutdown.status, "shutdown");
        let text = fs::read_to_string(&path).expect("read shutdown file");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["status"], "shutdown");
        assert!(value["timestamp_utc"].as_str().unwrap().ends_with('Z'));
        assert!(
            value.get("vault_read_ok").is_none(),
            "shutdown record must not look like a stale health probe: {text}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wait_loop_stops_on_first_pass_with_exact_attempt_count() {
        // Fails twice, then passes on the 3rd attempt. wait=2 ⇒ max 3 attempts.
        let mut calls = 0;
        let (final_result, attempts) = run_with_wait_interval(2, Duration::ZERO, || {
            calls += 1;
            if calls < 3 {
                result("fail")
            } else {
                result("pass")
            }
        });
        assert_eq!(attempts, 3, "exactly 3 attempts (2 fail + 1 pass)");
        assert_eq!(final_result.status, "pass");
    }

    #[test]
    fn wait_zero_is_a_single_attempt() {
        let mut calls = 0;
        let (final_result, attempts) = run_with_wait_interval(0, Duration::ZERO, || {
            calls += 1;
            result("fail")
        });
        assert_eq!(attempts, 1, "--wait 0 ⇒ no retry loop");
        assert_eq!(calls, 1);
        assert_eq!(final_result.status, "fail");
    }

    #[test]
    fn wait_loop_exhausts_and_returns_last_failure() {
        let (final_result, attempts) = run_with_wait_interval(2, Duration::ZERO, || result("fail"));
        assert_eq!(attempts, 3, "all 3 attempts ran");
        assert_eq!(final_result.status, "fail");
    }

    #[test]
    fn vault_probe_on_missing_path_is_health_fail_not_panic() {
        let missing = std::env::temp_dir().join("calyx-health-no-such-vault-xyz");
        let _ = fs::remove_dir_all(&missing);
        let (ok, error) = probe_vault(&missing);
        assert!(!ok);
        let error = error.expect("missing vault must produce an error");
        assert_eq!(error.code(), "CALYX_DAEMON_HEALTH_FAIL");
    }

    #[test]
    fn iso8601_matches_hand_computed_epochs() {
        // Hand-computed UTC instants (the 2+2=4 discipline).
        assert_eq!(iso8601_from_unix_secs(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_from_unix_secs(86_400), "1970-01-02T00:00:00Z");
        assert_eq!(
            iso8601_from_unix_secs(1_609_459_200),
            "2021-01-01T00:00:00Z"
        );
        assert_eq!(
            iso8601_from_unix_secs(1_700_000_000),
            "2023-11-14T22:13:20Z"
        );
    }
}
