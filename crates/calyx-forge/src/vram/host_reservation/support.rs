use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::{
    DEFAULT_HOST_HEADROOM_MIB, DEFAULT_REQUIRED_FREE_MIB, HOST_CAP_MIB_ENV,
    HostGpuReservationRequest, HostGpuReservationSnapshot, PersistedState, REMEDIATION,
};
use crate::{ForgeError, Result};

const BYTES_PER_MIB: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct PhysicalDevice {
    pub(super) uuid: String,
    pub(super) name: String,
    pub(super) total_mib: u64,
    pub(super) free_mib: u64,
}

pub(super) fn read_physical_device(device_index: u32) -> Result<PhysicalDevice> {
    let library = if cfg!(windows) {
        "nvml.dll"
    } else {
        "libnvidia-ml.so.1"
    };
    let nvml = nvml_wrapper::Nvml::builder()
        .lib_path(std::ffi::OsStr::new(library))
        .init()
        .map_err(|error| {
            config_error(format!(
                "NVML init failed loading {library}; physical GPU state is unprovable: {error}"
            ))
        })?;
    let device = nvml.device_by_index(device_index).map_err(|error| {
        config_error(format!(
            "NVML device_by_index({device_index}) failed: {error}"
        ))
    })?;
    let memory = device.memory_info().map_err(|error| {
        config_error(format!(
            "NVML memory_info failed for device {device_index}: {error}"
        ))
    })?;
    let uuid = device.uuid().map_err(|error| {
        config_error(format!(
            "NVML UUID failed for device {device_index}: {error}"
        ))
    })?;
    let name = device.name().map_err(|error| {
        config_error(format!(
            "NVML name failed for device {device_index}: {error}"
        ))
    })?;
    Ok(PhysicalDevice {
        uuid,
        name,
        total_mib: memory.total / BYTES_PER_MIB,
        free_mib: memory.free / BYTES_PER_MIB,
    })
}

pub(super) fn validate_request(request: &HostGpuReservationRequest) -> Result<()> {
    for (field, value) in [
        ("owner", request.owner.as_str()),
        ("job_id", request.job_id.as_str()),
        ("command", request.command.as_str()),
    ] {
        if value.trim().is_empty() || value.chars().any(char::is_control) || value.len() > 512 {
            return Err(config_error(format!(
                "GPU reservation {field} must be non-blank, control-free, and at most 512 bytes"
            )));
        }
    }
    if request.requested_mib == 0 {
        return Err(config_error(
            "GPU reservation requested_mib must be > 0".to_string(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn default_reservation_root() -> Result<PathBuf> {
    Ok(PathBuf::from("/tmp/calyx-gpu-reservations"))
}

#[cfg(windows)]
pub(super) fn default_reservation_root() -> Result<PathBuf> {
    let program_data = std::env::var_os("ProgramData").ok_or_else(|| {
        config_error(
            "ProgramData is unavailable; set CALYX_GPU_RESERVATION_ROOT to an absolute host-wide directory"
                .to_string(),
        )
    })?;
    Ok(PathBuf::from(program_data)
        .join("Calyx")
        .join("gpu-reservations"))
}

#[cfg(not(any(unix, windows)))]
pub(super) fn default_reservation_root() -> Result<PathBuf> {
    Err(config_error(
        "no host-wide GPU reservation root exists for this operating system; set CALYX_GPU_RESERVATION_ROOT to an absolute directory"
            .to_string(),
    ))
}

pub(super) fn validate_root(root: &Path) -> Result<()> {
    if root.as_os_str().is_empty() {
        return Err(config_error("GPU reservation root is empty".to_string()));
    }
    if !root.is_absolute() {
        return Err(config_error(format!(
            "GPU reservation root {} is relative and could split host admission by process cwd",
            root.display()
        )));
    }
    if root.to_str().is_none() {
        return Err(config_error(
            "GPU reservation root must be valid Unicode for exact persisted lease identity"
                .to_string(),
        ));
    }
    Ok(())
}

/// Principled default aggregate Calyx reservation cap when [`HOST_CAP_MIB_ENV`]
/// is unset: the physical device capacity minus the safety floor
/// (`required_free` + `headroom`). This lets the *realistic physical free*
/// govern the reservation budget instead of a stale flat constant that
/// under-provisions large cards (#1979: a hard-coded 12 GiB default starved a
/// 32 GiB RTX 5090). The `required_free`/physical-free admission gate still
/// prevents host OOM, and co-resident tenants (e.g. the leapable TEI fleet)
/// keep their already-allocated memory untouched — Calyx only reserves within
/// the free remainder. Set [`HOST_CAP_MIB_ENV`] to cap Calyx lower on a shared
/// device when a co-tenant is expected to *grow* (card − co-tenant − margin).
pub(super) fn default_host_cap_mib(device_total_mib: u64) -> u64 {
    device_total_mib
        .saturating_sub(DEFAULT_REQUIRED_FREE_MIB)
        .saturating_sub(DEFAULT_HOST_HEADROOM_MIB)
}

pub(super) fn host_cap_mib(device_total_mib: u64) -> Result<u64> {
    match std::env::var(HOST_CAP_MIB_ENV) {
        Ok(raw) => {
            let value = raw.trim().parse::<u64>().map_err(|error| {
                config_error(format!(
                    "{HOST_CAP_MIB_ENV}={raw:?} is not a positive MiB count: {error}"
                ))
            })?;
            if value == 0 {
                return Err(config_error(format!("{HOST_CAP_MIB_ENV} must be > 0")));
            }
            Ok(value)
        }
        Err(std::env::VarError::NotPresent) => Ok(default_host_cap_mib(device_total_mib)),
        Err(error) => Err(config_error(format!(
            "{HOST_CAP_MIB_ENV} is not valid Unicode: {error}"
        ))),
    }
}

pub(super) fn reserved_mib(state: &PersistedState) -> Result<u64> {
    state.reservations.iter().try_fold(0_u64, |sum, row| {
        sum.checked_add(row.requested_mib)
            .ok_or_else(|| config_error("persisted reservation sum overflow".to_string()))
    })
}

pub(super) fn reservation_id(request: &HostGpuReservationRequest, now: u128) -> String {
    let mut hasher = Sha256::new();
    hasher.update(request.device_index.to_be_bytes());
    hasher.update(std::process::id().to_be_bytes());
    hasher.update(now.to_be_bytes());
    hasher.update(request.owner.as_bytes());
    hasher.update(request.job_id.as_bytes());
    hasher.update(request.command.as_bytes());
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn reservation_identities(state: &PersistedState) -> String {
    if state.reservations.is_empty() {
        return "none".to_string();
    }
    state
        .reservations
        .iter()
        .map(|row| {
            format!(
                "{}:{}:{}:{}MiB:pid{}",
                row.owner, row.job_id, row.reservation_id, row.requested_mib, row.pid
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn snapshot(
    state: &PersistedState,
    state_path: &Path,
    bytes: &[u8],
) -> Result<HostGpuReservationSnapshot> {
    let reserved_mib = reserved_mib(state)?;
    Ok(HostGpuReservationSnapshot {
        schema_version: state.schema_version,
        state_path: state_path.to_string_lossy().into_owned(),
        state_sha256: hex_sha256(bytes),
        device_index: state.device_index,
        device_uuid: state.device_uuid.clone(),
        device_name: state.device_name.clone(),
        device_total_mib: state.device_total_mib,
        host_cap_mib: state.host_cap_mib,
        required_free_mib: state.required_free_mib,
        headroom_mib: state.headroom_mib,
        epoch_free_mib: state.epoch_free_mib,
        epoch_capacity_mib: state.epoch_capacity_mib,
        last_physical_free_mib: state.last_physical_free_mib,
        reserved_mib,
        available_reservation_mib: state.epoch_capacity_mib.saturating_sub(reserved_mib),
        admitted_total: state.admitted_total,
        rejected_total: state.rejected_total,
        stale_reaped_total: state.stale_reaped_total,
        reservations: state.reservations.clone(),
        last_rejection: state.last_rejection.clone(),
    })
}

pub(super) fn open_lock_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            io_error(
                "open GPU reservation lock",
                path,
                error,
                "fix lease directory ownership/permissions",
            )
        })
}

pub(super) fn create_lease_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            io_error(
                "create unique GPU reservation lease",
                path,
                error,
                "inspect the device ledger for a reservation-id collision or stale orphan lease; never reuse an ambiguous lease",
            )
        })
}

pub(super) fn open_existing_lock_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            io_error(
                "open persisted GPU reservation lease",
                path,
                error,
                "the ledger names a missing/unreadable lease; inspect process state and repair the source of truth rather than assuming the reservation is stale",
            )
        })
}

pub(super) fn unix_ms() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| config_error(format!("system clock is before Unix epoch: {error}")))
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn backpressure(detail: String) -> ForgeError {
    ForgeError::HostVramBackpressure {
        detail,
        remediation: REMEDIATION.to_string(),
    }
}

pub(super) fn config_error(detail: String) -> ForgeError {
    ForgeError::VramBudget {
        detail,
        remediation: "repair the named host reservation configuration/source of truth; never assume unknown GPU state is safe".to_string(),
    }
}

pub(super) fn io_error(
    operation: &str,
    path: &Path,
    error: std::io::Error,
    remediation: &str,
) -> ForgeError {
    ForgeError::CacheError {
        op: operation.to_string(),
        path: path.to_string_lossy().into_owned(),
        detail: error.to_string(),
        remediation: remediation.to_string(),
    }
}

#[cfg(test)]
mod default_cap_tests {
    use super::super::{
        DEFAULT_HOST_CAP_MIB, DEFAULT_HOST_HEADROOM_MIB, DEFAULT_REQUIRED_FREE_MIB,
    };
    use super::default_host_cap_mib;

    const FLOOR: u64 = DEFAULT_REQUIRED_FREE_MIB + DEFAULT_HOST_HEADROOM_MIB;

    #[test]
    fn large_card_is_not_capped_at_the_legacy_12gib() {
        // RTX 5090 (#1979): 32,607 MiB card must not be limited to the stale 12 GiB default.
        let cap = default_host_cap_mib(32_607);
        assert_eq!(cap, 32_607 - FLOOR);
        assert!(
            cap > DEFAULT_HOST_CAP_MIB,
            "device-relative default {cap} must exceed the legacy flat cap {DEFAULT_HOST_CAP_MIB} on a 32 GiB card"
        );
    }

    #[test]
    fn small_card_stays_within_physical_capacity() {
        // On a 12 GiB card the legacy flat 12288 exceeded the card; the device-relative
        // default leaves the safety floor free.
        let cap = default_host_cap_mib(12 * 1024);
        assert_eq!(cap, 12 * 1024 - FLOOR);
        assert!(cap < 12 * 1024);
    }

    #[test]
    fn degenerate_tiny_device_saturates_to_zero_without_panicking() {
        assert_eq!(default_host_cap_mib(0), 0);
        assert_eq!(default_host_cap_mib(FLOOR), 0);
        assert_eq!(default_host_cap_mib(FLOOR - 1), 0);
    }
}
