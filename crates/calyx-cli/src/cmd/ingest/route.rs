//! Resident-route resolution for GPU lens measurement (#1004).
//!
//! `calyx ingest` must not silently fall back to cold per-invocation GPU lens
//! workers: every cold run pays full model/session reloads (observed ~12-14s
//! per qwen-class lens) and is exactly the accidental slow path #999 measured
//! at 43.5 minutes for a 118-row batch. Route resolution order:
//!
//! 1. explicit `--resident-addr` flag,
//! 2. `CALYX_RESIDENT_ADDR` env,
//! 3. the discovery file `calyx panel resident serve` writes under
//!    `<CALYX_HOME>/resident/discovery.json`, validated by a live readiness
//!    probe (pid must match) and by vault identity when the service was warmed
//!    from a vault.
//!
//! Discovery anomalies never hard-fail here — they resolve to "no resident
//! route" with a recorded reason. Enforcement is at the GPU measurement gate:
//! active GPU lenses with no resident route refuse to measure
//! (`CALYX_INGEST_GPU_ROUTE_REQUIRED`) unless the operator explicitly opted
//! into cold workers via `--allow-cold-gpu-workers` or
//! `CALYX_INGEST_ALLOW_COLD_GPU_WORKERS=1`.

use std::net::SocketAddr;
use std::path::Path;

use calyx_core::{CalyxError, Modality};

use super::command::ingest_runtime_log;
use crate::error::CliResult;
use crate::panel_commands::{
    ResidentDiscovery, read_resident_discovery, resident_discovery_path, resident_ready_value_at,
};

pub(crate) const RESIDENT_ADDR_ENV: &str = "CALYX_RESIDENT_ADDR";
pub(crate) const ALLOW_COLD_GPU_WORKERS_ENV: &str = "CALYX_INGEST_ALLOW_COLD_GPU_WORKERS";

/// Resolved measurement route for GPU-placed lenses, threaded from the ingest
/// command into microbatch measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IngestGpuRoute {
    pub(crate) resident_addr: Option<SocketAddr>,
    pub(crate) allow_cold_gpu_workers: bool,
    /// Why no resident address was resolved (static class; details are in the
    /// `phase=gpu_route*` runtime log lines emitted during resolution).
    pub(crate) no_route_reason: Option<&'static str>,
}

impl IngestGpuRoute {
    /// Cold-worker route for in-crate tests and the `calyx measure` debug
    /// command, which measure single inputs and are not the batch-ingest
    /// surface #1004 gates.
    pub(crate) fn cold_workers_allowed() -> Self {
        Self {
            resident_addr: None,
            allow_cold_gpu_workers: true,
            no_route_reason: None,
        }
    }
}

pub(crate) fn resolve_ingest_gpu_route(
    vault_path: &Path,
    flag_addr: Option<SocketAddr>,
    allow_cold_flag: bool,
) -> CliResult<IngestGpuRoute> {
    let allow_cold = allow_cold_flag || env_flag_enabled(ALLOW_COLD_GPU_WORKERS_ENV);
    let (resident_addr, source, no_route_reason) = if let Some(addr) = flag_addr {
        (Some(addr), "flag", None)
    } else if let Some(addr) = env_resident_addr()? {
        (Some(addr), "env", None)
    } else {
        match discover_resident_addr(vault_path)? {
            Ok(addr) => (Some(addr), "discovery", None),
            Err(reason) => (None, "none", Some(reason)),
        }
    };
    ingest_runtime_log(format_args!(
        "phase=gpu_route source={source} resident_addr={resident_addr:?} allow_cold_gpu_workers={allow_cold} no_route_reason={}",
        no_route_reason.unwrap_or("-")
    ));
    Ok(IngestGpuRoute {
        resident_addr,
        allow_cold_gpu_workers: allow_cold,
        no_route_reason,
    })
}

/// The fail-closed gate raised at the microbatch routing decision when active
/// GPU lenses have no resident route and cold workers were not explicitly
/// allowed.
pub(crate) fn gpu_route_required_error(
    gpu_lens_count: usize,
    modality: Modality,
    route: IngestGpuRoute,
) -> CalyxError {
    CalyxError {
        code: "CALYX_INGEST_GPU_ROUTE_REQUIRED",
        message: format!(
            "{gpu_lens_count} active GPU lens(es) for modality {modality:?} but no resident measurement route is available (reason={}); cold per-invocation GPU lens workers reload every model per command and are refused by default",
            route.no_route_reason.unwrap_or("unresolved")
        ),
        remediation: "start `calyx panel resident serve --vault <vault>` (ingest auto-discovers it via CALYX_HOME) or pass --resident-addr <addr>; to explicitly accept cold per-invocation GPU workers pass --allow-cold-gpu-workers or set CALYX_INGEST_ALLOW_COLD_GPU_WORKERS=1",
    }
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|raw| {
            let raw = raw.trim();
            raw == "1" || raw.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false)
}

fn env_resident_addr() -> CliResult<Option<SocketAddr>> {
    let Ok(raw) = std::env::var(RESIDENT_ADDR_ENV) else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    // Explicit operator input: parse/loopback failures are loud, mirroring
    // the --resident-addr flag contract.
    super::parse::parse_resident_addr(raw).map(Some)
}

fn discover_resident_addr(vault_path: &Path) -> CliResult<Result<SocketAddr, &'static str>> {
    let Some(home) = std::env::var_os("CALYX_HOME").map(std::path::PathBuf::from) else {
        return Ok(Err("no_calyx_home"));
    };
    let discovery = match read_resident_discovery(&home)? {
        Ok(discovery) => discovery,
        Err(reason) => {
            ingest_runtime_log(format_args!(
                "phase=gpu_route_discovery_skipped path={} reason={reason}",
                resident_discovery_path(&home).display()
            ));
            return Ok(Err(reason));
        }
    };
    if let Some(reason) = vault_mismatch(&discovery, vault_path) {
        return Ok(Err(reason));
    }
    probe_discovered_service(&home, &discovery)
}

/// A service warmed from a different vault serves a different panel/registry;
/// measuring through it would be refuted later by contract checks, so treat it
/// as not-discovered up front. Template-warmed services carry no vault
/// identity and are accepted; the resident measure path still fail-closes on
/// modality/placement/contract mismatches.
fn vault_mismatch(discovery: &ResidentDiscovery, vault_path: &Path) -> Option<&'static str> {
    let discovery_vault = discovery.vault.as_deref()?;
    let Ok(ingest_vault) = vault_path.canonicalize() else {
        return Some("ingest_vault_not_canonicalizable");
    };
    if ingest_vault == discovery_vault {
        None
    } else {
        ingest_runtime_log(format_args!(
            "phase=gpu_route_discovery_skipped reason=vault_mismatch discovery_vault={} ingest_vault={}",
            discovery_vault.display(),
            ingest_vault.display()
        ));
        Some("vault_mismatch")
    }
}

fn probe_discovered_service(
    home: &Path,
    discovery: &ResidentDiscovery,
) -> CliResult<Result<SocketAddr, &'static str>> {
    let ready = match resident_ready_value_at(discovery.bind) {
        Ok(value) => value,
        Err(error) => {
            ingest_runtime_log(format_args!(
                "phase=gpu_route_discovery_skipped path={} reason=stale_unreachable bind={} error_code={}",
                resident_discovery_path(home).display(),
                discovery.bind,
                error.code()
            ));
            return Ok(Err("stale_unreachable"));
        }
    };
    let ready_ok = ready.get("ready").and_then(|value| value.as_bool()) == Some(true);
    let pid = ready
        .get("process_id")
        .and_then(|value| value.as_u64())
        .map(|pid| pid as u32);
    if !ready_ok || pid != Some(discovery.process_id) {
        ingest_runtime_log(format_args!(
            "phase=gpu_route_discovery_skipped path={} reason=stale_identity bind={} ready={ready_ok} probe_pid={pid:?} discovery_pid={}",
            resident_discovery_path(home).display(),
            discovery.bind,
            discovery.process_id
        ));
        return Ok(Err("stale_identity"));
    }
    ingest_runtime_log(format_args!(
        "phase=gpu_route_discovered bind={} process_id={} vault={:?} template={:?}",
        discovery.bind, discovery.process_id, discovery.vault, discovery.template
    ));
    Ok(Ok(discovery.bind))
}
