//! `calyx resource-status` — PRD 18 §4 aggregate resource readback (issue #592).

use std::path::Path;

use calyx_anneal::{BudgetConfig, BudgetEnforcer};
use calyx_aster::resource::VramBudgetStatus;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, SystemClock};

pub(crate) const RESOURCE_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
pub(crate) const RESOURCE_VAULT_SALT: &[u8] = b"calyx-resource-status";

/// Output rendering for the collected status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceStatusFormat {
    Json,
    Metrics,
}

pub(crate) fn run_resource_status(
    vault: &Path,
    format: ResourceStatusFormat,
) -> crate::error::CliResult {
    // A status probe must never create vault state: refuse paths that are not
    // already an Aster vault instead of letting open() materialize skeleton dirs.
    if !vault.is_dir() {
        return Err(CalyxError::disk_pressure(format!(
            "vault dir {} does not exist",
            vault.display()
        ))
        .into());
    }
    if !vault.join("cf").is_dir() {
        return Err(CalyxError::disk_pressure(format!(
            "{} has no cf/ root; not an Aster vault",
            vault.display()
        ))
        .into());
    }
    let store = open_resource_vault(vault, VaultOptions::default())?;
    let vram = vram_status_from_vault(vault)?;
    let status = store
        .resource_status(vault, vram)
        .map_err(|error| error.to_string())?;
    match format {
        ResourceStatusFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&status).map_err(|error| error.to_string())?
        ),
        ResourceStatusFormat::Metrics => print!("{}", status.to_metrics_text(&vault_label(vault))),
    }
    Ok(())
}

/// Opens the vault for resource inspection through the full recovery path.
pub(crate) fn open_resource_vault(
    vault: &Path,
    options: VaultOptions,
) -> Result<AsterVault, String> {
    let vault_id = RESOURCE_VAULT_ID
        .parse()
        .map_err(|error| format!("parse resource vault id: {error}"))?;
    AsterVault::open(vault, vault_id, RESOURCE_VAULT_SALT.to_vec(), options)
        .map_err(|error| error.to_string())
}

/// Builds the VRAM budget section from the vault Anneal budget config.
///
/// Uses the canonical `BudgetConfig::load_from_vault` accessor: the first call
/// materializes the default `.anneal/budget.toml` exactly as Anneal would.
/// Probe degradation (e.g. NVML unavailable) surfaces in `probe_warning` —
/// never as a silent zero.
pub(crate) fn vram_status_from_vault(vault: &Path) -> Result<VramBudgetStatus, String> {
    let config = BudgetConfig::load_from_vault(vault).map_err(|error| error.to_string())?;
    let clock = SystemClock;
    let enforcer = BudgetEnforcer::new(config, &clock).map_err(|error| error.to_string())?;
    let status = enforcer.tick().map_err(|error| error.to_string())?;
    Ok(VramBudgetStatus {
        budget_bytes: config.vram_bytes,
        used_bytes: status.vram_used_bytes,
        probe_warning: status.warning_code,
    })
}

fn vault_label(vault: &Path) -> String {
    vault.file_name().map_or_else(
        || vault.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
