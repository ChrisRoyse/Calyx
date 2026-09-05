use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use calyx_forge::vram::{
    HostGpuReservation, HostGpuReservationRequest, HostGpuReservationSnapshot,
    HostGpuReservationStore,
};
use calyx_registry::{LensRuntime, LensSpec, lens_spec_from_manifest_path};

use crate::error::{CliError, CliResult};

mod admission;
mod child_supervisor;

pub(crate) use admission::admit_batch_gpu;

pub(crate) const DEFAULT_ASSAY_I8BIN_PEAK_MIB: u64 = 768;
const OWNER_ENV: &str = "CALYX_GPU_JOB_OWNER";
const ASSAY_STRICT_CUDA_ENV: &str = "CALYX_ASSAY_CUDA_STRICT";
const ACTIVE_RESERVATION_ID_ENV: &str = "CALYX_GPU_RESERVATION_ID";
const RESERVATION_REMEDIATION: &str = "wait for the named live GPU reservation to release or reduce the declared measured peak; inspect `calyx gpu reservations`; never bypass admission, fall back to CPU, or kill an unrelated owner";

pub(crate) fn run(topic: &str, args: &[String]) -> CliResult {
    match topic {
        "reservations" => reservations(args),
        "reservation-hold" => reservation_hold(args),
        "run" => reservation_run(args),
        other => Err(CliError::usage(format!(
            "unknown gpu topic {other}; expected reservations, reservation-hold, or run"
        ))),
    }
}

pub(crate) fn acquire_strict_assay(
    job_id: &str,
    requested_mib: u64,
) -> CliResult<Option<HostGpuReservation>> {
    if std::env::var_os(ASSAY_STRICT_CUDA_ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
        return Ok(None);
    }
    let owner = resolved_owner(None)?;
    let store = HostGpuReservationStore::from_env(0).map_err(reservation_error)?;
    let request = HostGpuReservationRequest::new(
        owner,
        job_id,
        "calyx assay i8bin-ensemble-card",
        requested_mib,
    );
    store.acquire(request).map(Some).map_err(reservation_error)
}

pub(crate) fn release(
    reservation: Option<HostGpuReservation>,
) -> CliResult<Option<HostGpuReservationSnapshot>> {
    reservation
        .map(HostGpuReservation::release)
        .transpose()
        .map_err(reservation_error)
}

pub(crate) fn require_active_reservation(
    operation: &'static str,
) -> CliResult<HostGpuReservationSnapshot> {
    let reservation_id = std::env::var(ACTIVE_RESERVATION_ID_ENV).map_err(|_| {
        backpressure_cli(format!(
            "{operation} has no host GPU lease; launch it through `calyx gpu run --mib <measured-peak> --job <stable-id> -- <command> ...`"
        ))
    })?;
    if reservation_id.trim().is_empty() {
        return Err(backpressure_cli(format!(
            "{operation} received an empty {ACTIVE_RESERVATION_ID_ENV}"
        )));
    }
    let snapshot = HostGpuReservationStore::from_env(0)
        .and_then(|store| store.readback())
        .map_err(reservation_error)?;
    if !snapshot
        .reservations
        .iter()
        .any(|row| row.reservation_id == reservation_id)
    {
        return Err(backpressure_cli(format!(
            "{operation} reservation {reservation_id} is not live in {}",
            snapshot.state_path
        )));
    }
    Ok(snapshot)
}

/// The trimmed, non-blank live reservation id from the environment, or a
/// structured `CALYX_BACKPRESSURE` if the process was not launched under a GPU
/// lease. Used by the resident to republish the lease it validated (#1936).
pub(crate) fn active_reservation_id(operation: &'static str) -> CliResult<String> {
    let reservation_id = std::env::var(ACTIVE_RESERVATION_ID_ENV).map_err(|_| {
        backpressure_cli(format!(
            "{operation} has no host GPU lease; launch it through `calyx gpu run --mib <measured-peak> --job <stable-id> -- <command> ...`"
        ))
    })?;
    let trimmed = reservation_id.trim();
    if trimmed.is_empty() {
        return Err(backpressure_cli(format!(
            "{operation} received an empty {ACTIVE_RESERVATION_ID_ENV}"
        )));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn require_for_lens_specs(
    operation: &'static str,
    specs: &[LensSpec],
) -> CliResult<Option<HostGpuReservationSnapshot>> {
    if specs.iter().any(|spec| runtime_uses_gpu(&spec.runtime)) {
        return require_active_reservation(operation).map(Some);
    }
    Ok(None)
}

pub(crate) fn require_for_manifests(
    operation: &'static str,
    manifests: &[PathBuf],
) -> CliResult<Option<HostGpuReservationSnapshot>> {
    let specs = manifests
        .iter()
        .map(|path| {
            lens_spec_from_manifest_path(path).map_err(|error| {
                CliError::runtime(format!(
                    "read frozen lens manifest {} before GPU admission: {error}",
                    path.display()
                ))
            })
        })
        .collect::<CliResult<Vec<_>>>()?;
    require_for_lens_specs(operation, &specs)
}

fn runtime_uses_gpu(runtime: &LensRuntime) -> bool {
    !matches!(
        runtime,
        LensRuntime::Algorithmic { .. }
            | LensRuntime::StaticLookup { .. }
            | LensRuntime::ExternalCmd { .. }
    )
}

fn reservations(args: &[String]) -> CliResult {
    if !args.is_empty() {
        return Err(CliError::usage(
            "gpu reservations accepts no positional arguments",
        ));
    }
    let snapshot = HostGpuReservationStore::from_env(0)
        .and_then(|store| store.readback())
        .map_err(reservation_error)?;
    print_json(&snapshot)
}

fn reservation_run(args: &[String]) -> CliResult {
    let mut requested_mib = None;
    let mut owner = None;
    let mut job_id = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--mib" => {
                requested_mib = Some(parse_u64(value(args, index, "--mib")?, "--mib")?);
                index += 2;
            }
            "--owner" => {
                owner = Some(value(args, index, "--owner")?.to_string());
                index += 2;
            }
            "--job" => {
                job_id = Some(value(args, index, "--job")?.to_string());
                index += 2;
            }
            "--" => {
                index += 1;
                break;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unknown gpu run arg {other}; terminate reservation flags with --"
                )));
            }
        }
    }
    let requested_mib = requested_mib.ok_or_else(|| CliError::usage("--mib is required"))?;
    if requested_mib == 0 {
        return Err(CliError::usage("--mib must be > 0"));
    }
    let job_id = job_id.ok_or_else(|| CliError::usage("--job is required"))?;
    let child = args
        .get(index)
        .ok_or_else(|| CliError::usage("gpu run requires a child command after --"))?;
    let child_args = &args[index + 1..];
    let owner = resolved_owner(owner.as_deref())?;
    let store = HostGpuReservationStore::from_env(0).map_err(reservation_error)?;
    let reservation = store
        .acquire(HostGpuReservationRequest::new(
            owner,
            job_id,
            format!("calyx gpu run child={child}"),
            requested_mib,
        ))
        .map_err(reservation_error)?;
    print_event(&serde_json::json!({
        "event": "gpu_child_admitted",
        "reservation_id": reservation.reservation_id(),
        "source_of_truth": reservation.admitted_snapshot(),
    }))?;
    // The supervisor owns the child lifecycle: it forwards wrapper SIGTERM/
    // SIGINT to the child's process group and releases the lease ONLY after
    // the child's exit is confirmed (#1932).
    child_supervisor::supervise(child, child_args, reservation)
}

fn reservation_hold(args: &[String]) -> CliResult {
    let mut requested_mib = None;
    let mut owner = None;
    let mut job_id = None;
    let mut hold_secs = 30_u64;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--mib" => {
                requested_mib = Some(parse_u64(value(args, index, "--mib")?, "--mib")?);
                index += 2;
            }
            "--owner" => {
                owner = Some(value(args, index, "--owner")?.to_string());
                index += 2;
            }
            "--job" => {
                job_id = Some(value(args, index, "--job")?.to_string());
                index += 2;
            }
            "--hold-secs" => {
                hold_secs = parse_u64(value(args, index, "--hold-secs")?, "--hold-secs")?;
                index += 2;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unknown gpu reservation-hold arg {other}"
                )));
            }
        }
    }
    let requested_mib = requested_mib.ok_or_else(|| CliError::usage("--mib is required"))?;
    if requested_mib == 0 {
        return Err(CliError::usage("--mib must be > 0"));
    }
    let job_id = job_id.ok_or_else(|| CliError::usage("--job is required"))?;
    let owner = resolved_owner(owner.as_deref())?;
    let store = HostGpuReservationStore::from_env(0).map_err(reservation_error)?;
    let reservation = store
        .acquire(HostGpuReservationRequest::new(
            owner,
            job_id,
            "calyx gpu reservation-hold",
            requested_mib,
        ))
        .map_err(reservation_error)?;
    print_json(&serde_json::json!({
        "event": "acquired",
        "reservation_id": reservation.reservation_id(),
        "source_of_truth": reservation.admitted_snapshot(),
    }))?;
    std::io::stdout()
        .flush()
        .map_err(|error| CliError::io(format!("flush reservation acquisition output: {error}")))?;
    if hold_secs > 0 {
        std::thread::sleep(Duration::from_secs(hold_secs));
    }
    let released = reservation.release().map_err(reservation_error)?;
    print_json(&serde_json::json!({
        "event": "released",
        "source_of_truth": released,
    }))
}

fn resolved_owner(explicit: Option<&str>) -> CliResult<String> {
    if let Some(value) = explicit {
        return nonblank_owner(value);
    }
    if let Ok(value) = std::env::var(OWNER_ENV) {
        return nonblank_owner(&value);
    }
    for name in ["USER", "USERNAME"] {
        if let Ok(value) = std::env::var(name)
            && !value.trim().is_empty()
        {
            return nonblank_owner(&value);
        }
    }
    Err(CliError::from(calyx_core::CalyxError::backpressure(
        format!(
            "GPU reservation owner is unavailable; set {OWNER_ENV} to a stable operator/service identity"
        ),
    )))
}

fn nonblank_owner(value: &str) -> CliResult<String> {
    if value.trim().is_empty() {
        return Err(CliError::usage("GPU reservation owner must be non-blank"));
    }
    Ok(value.to_string())
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn parse_u64(value: &str, flag: &str) -> CliResult<u64> {
    value
        .parse::<u64>()
        .map_err(|error| CliError::usage(format!("invalid {flag} {value:?}: {error}")))
}

fn print_json(value: &impl serde::Serialize) -> CliResult {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| CliError::runtime(format!(
            "serialize GPU reservation JSON: {error}"
        )))?
    );
    Ok(())
}

fn print_event(value: &impl serde::Serialize) -> CliResult {
    eprintln!(
        "CALYX_GPU_RESERVATION {}",
        serde_json::to_string(value).map_err(|error| CliError::runtime(format!(
            "serialize GPU reservation event: {error}"
        )))?
    );
    Ok(())
}

fn backpressure_cli(message: String) -> CliError {
    CliError::from(calyx_core::CalyxError {
        code: "CALYX_BACKPRESSURE",
        message,
        remediation: RESERVATION_REMEDIATION,
    })
}

fn reservation_error(error: calyx_forge::ForgeError) -> CliError {
    match error {
        calyx_forge::ForgeError::HostVramBackpressure { detail, .. } => backpressure_cli(detail),
        other => other.into(),
    }
}
