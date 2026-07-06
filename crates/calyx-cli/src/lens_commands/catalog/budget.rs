use std::env;

use calyx_core::Placement;
use calyx_registry::PlacementBudget;
use calyxd::vram::{NvmlVramUsage, VramUsage};

use super::LensCatalog;
use crate::error::{CliError, CliResult};

pub(super) fn placement_budget_from_catalog(catalog: &LensCatalog) -> CliResult<PlacementBudget> {
    let vram_allocated_bytes = catalog
        .lenses
        .iter()
        .filter(|entry| entry.placement == Placement::Gpu)
        .map(|entry| entry.cost.vram_bytes)
        .fold(0_u64, u64::saturating_add);
    let ram_used_bytes = catalog
        .lenses
        .iter()
        .filter(|entry| entry.placement == Placement::Cpu)
        .map(|entry| entry.cost.ram_bytes)
        .fold(0_u64, u64::saturating_add);
    let cpu_resident_count = catalog
        .lenses
        .iter()
        .filter(|entry| entry.placement == Placement::Cpu)
        .count();
    let (vram_soft_cap_bytes, tei_reserved_bytes) = resolve_gpu_vram_budget()?;
    let available = vram_soft_cap_bytes
        .saturating_sub(tei_reserved_bytes)
        .saturating_sub(vram_allocated_bytes);
    let mib = 1024 * 1024;
    eprintln!(
        "[placement] vram cap={} MiB reserved(other)={} MiB allocated(gpu lenses)={} MiB \
         available={} MiB",
        vram_soft_cap_bytes / mib,
        tei_reserved_bytes / mib,
        vram_allocated_bytes / mib,
        available / mib,
    );
    Ok(PlacementBudget {
        vram_soft_cap_bytes,
        tei_reserved_bytes,
        vram_allocated_bytes,
        ram_soft_cap_bytes: env_u64("CALYX_PANEL_RAM_SOFT_CAP_BYTES", 121 * gib())?,
        ram_used_bytes,
        cpu_resident_limit: env_usize("CALYX_CPU_LENS_POOL_CAP", 128)?,
        cpu_resident_count,
    })
}

fn resolve_gpu_vram_budget() -> CliResult<(u64, u64)> {
    let cap_override = env_opt_u64("CALYX_PANEL_VRAM_SOFT_CAP_BYTES")?;
    let tei_override = env_opt_u64("CALYX_TEI_RESERVED_BYTES")?;
    let headroom = env_u64("CALYX_GPU_HEADROOM_BYTES", DEFAULT_GPU_HEADROOM_BYTES)?;
    let probe = if cap_override.is_some() && tei_override.is_some() {
        None
    } else {
        Some(probe_gpu_vram_bytes().map_err(|err| {
            CliError::usage(format!(
                "GPU VRAM probe via NVML failed and CALYX_PANEL_VRAM_SOFT_CAP_BYTES / \
                 CALYX_TEI_RESERVED_BYTES are not both set ({err}); set both to explicit byte \
                 budgets, or ensure the NVIDIA driver NVML library is reachable"
            ))
        })?)
    };
    compute_vram_budget(cap_override, tei_override, probe, headroom)
}

pub(super) fn compute_vram_budget(
    cap_override: Option<u64>,
    tei_override: Option<u64>,
    probe: Option<(u64, u64)>,
    headroom: u64,
) -> CliResult<(u64, u64)> {
    if let (Some(cap), Some(tei)) = (cap_override, tei_override) {
        return Ok((cap, tei));
    }
    let (total_bytes, used_bytes) = probe.ok_or_else(|| {
        CliError::usage(
            "VRAM probe reading required to derive placement budget but none was supplied"
                .to_string(),
        )
    })?;
    let cap = cap_override.unwrap_or_else(|| total_bytes.saturating_sub(headroom));
    let tei = tei_override.unwrap_or(used_bytes);
    Ok((cap, tei))
}

fn probe_gpu_vram_bytes() -> Result<(u64, u64), calyxd::error::DaemonError> {
    let reading = NvmlVramUsage::init()?.read()?;
    const BYTES_PER_MIB: u64 = 1024 * 1024;
    Ok((
        u64::from(reading.total_mib) * BYTES_PER_MIB,
        u64::from(reading.used_mib) * BYTES_PER_MIB,
    ))
}

fn env_opt_u64(name: &str) -> CliResult<Option<u64>> {
    match env::var(name) {
        Ok(raw) => raw
            .parse()
            .map(Some)
            .map_err(|err| CliError::usage(format!("parse {name}={raw}: {err}"))),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(CliError::usage(format!("read {name}: {err}"))),
    }
}

fn env_u64(name: &str, default: u64) -> CliResult<u64> {
    match env::var(name) {
        Ok(raw) => raw
            .parse()
            .map_err(|err| CliError::usage(format!("parse {name}={raw}: {err}"))),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(CliError::usage(format!("read {name}: {err}"))),
    }
}

fn env_usize(name: &str, default: usize) -> CliResult<usize> {
    match env::var(name) {
        Ok(raw) => raw
            .parse()
            .map_err(|err| CliError::usage(format!("parse {name}={raw}: {err}"))),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(CliError::usage(format!("read {name}: {err}"))),
    }
}

const DEFAULT_GPU_HEADROOM_BYTES: u64 = 4 * gib();

const fn gib() -> u64 {
    1024 * 1024 * 1024
}
