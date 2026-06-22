use std::env;
use std::fs;
use std::path::{Path, PathBuf};

mod a38_bundle;
mod template_cards;
mod template_model;
mod template_store;
mod templates;

use calyx_assay::{PanelResourceBudget, ResourceDensity, ResourceUsage, pack_panel_by_density};
use calyx_core::{LensCost, Placement, SlotState};
use calyx_registry::{
    LensHealth, PanelSlotListing, lens_spec_from_manifest_path, list_panel, load_vault_panel_state,
};
use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LensCatalog {
    lenses: Vec<LensCatalogEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LensCatalogEntry {
    lens_id: String,
    name: String,
    modality: String,
    runtime: String,
    dim: u32,
    weights_sha256: String,
    manifest: PathBuf,
    #[serde(default)]
    cost: LensCost,
    #[serde(default)]
    placement: Placement,
}

#[derive(Serialize)]
struct PanelStatusReport {
    catalog: PathBuf,
    count: usize,
    cpu_lenses: usize,
    gpu_lenses: usize,
    total_ram_bytes: u64,
    total_ram_mb: f32,
    total_vram_bytes: u64,
    total_vram_mb: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget: Option<PanelResourceBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remaining_budget: Option<ResourceUsage>,
    lenses: Vec<PanelLensStatus>,
}

#[derive(Serialize)]
struct VaultPanelStatusReport {
    vault: PathBuf,
    panel_version: u32,
    slot_count: usize,
    registry_lens_count: usize,
    panel_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget: Option<PanelResourceBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remaining_budget: Option<ResourceUsage>,
    slots: Vec<PanelSlotStatus>,
}

#[derive(Serialize)]
struct PanelLensStatus {
    lens_id: String,
    name: String,
    runtime: String,
    placement: Placement,
    cost: LensCost,
    ram_mb: f32,
    vram_mb: f32,
    batch_ceiling: u32,
    manifest: PathBuf,
    health: LensHealth,
    #[serde(skip_serializing_if = "Option::is_none")]
    remaining_budget_after: Option<ResourceUsage>,
}

#[derive(Serialize)]
struct PanelSlotStatus {
    #[serde(flatten)]
    listing: PanelSlotListing,
    cost: LensCost,
    placement: Placement,
    ram_mb: f32,
    vram_mb: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    density: Option<ResourceDensity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remaining_budget_after: Option<ResourceUsage>,
}

pub(crate) fn run(topic: &str, rest: &[String]) -> CliResult {
    match topic {
        "a38-bundle" => a38_bundle::run(rest),
        "status" => status(rest),
        "template" => templates::run(rest),
        other => Err(CliError::usage(format!(
            "unknown panel subcommand {other}; expected a38-bundle, status, or template"
        ))),
    }
}

fn status(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    if let Some(vault) = flags.vault {
        return status_vault(vault, flags.panel_budget_json.as_deref());
    }
    let budget = match flags.panel_budget_json.as_deref() {
        Some(path) => Some(read_budget(path)?),
        None => None,
    };
    let catalog_path = catalog_path(flags.home.as_deref())?;
    let catalog = read_catalog(&catalog_path)?;
    let (lenses, remaining_budget) = catalog_lens_status(catalog.lenses, budget);
    let total_ram_bytes = lenses
        .iter()
        .map(|lens| lens.cost.ram_bytes)
        .fold(0_u64, u64::saturating_add);
    let total_vram_bytes = lenses
        .iter()
        .map(|lens| lens.cost.vram_bytes)
        .fold(0_u64, u64::saturating_add);
    let cpu_lenses = lenses
        .iter()
        .filter(|lens| lens.placement == Placement::Cpu)
        .count();
    let gpu_lenses = lenses.len().saturating_sub(cpu_lenses);

    print_json(&PanelStatusReport {
        catalog: catalog_path,
        count: lenses.len(),
        cpu_lenses,
        gpu_lenses,
        total_ram_bytes,
        total_ram_mb: mib(total_ram_bytes),
        total_vram_bytes,
        total_vram_mb: mib(total_vram_bytes),
        budget,
        remaining_budget,
        lenses,
    })
}

fn status_vault(vault: PathBuf, budget_path: Option<&Path>) -> CliResult {
    let state = load_vault_panel_state(&vault)?;
    let budget = match budget_path {
        Some(path) => Some(read_budget(path)?),
        None => None,
    };
    let (slots, remaining_budget) =
        vault_slot_status(list_panel(&state.panel, &state.registry), budget);
    let panel_ref = state
        .registry_snapshot
        .as_ref()
        .map(|snapshot| snapshot.panel_ref.logical_path.clone());
    let registry_lens_count = state
        .registry_snapshot
        .as_ref()
        .map_or(0, |snapshot| snapshot.lenses.len());
    print_json(&VaultPanelStatusReport {
        vault,
        panel_version: state.panel.version,
        slot_count: state.panel.slots.len(),
        registry_lens_count,
        panel_ref,
        budget,
        remaining_budget,
        slots,
    })
}

fn catalog_lens_status(
    entries: Vec<LensCatalogEntry>,
    budget: Option<PanelResourceBudget>,
) -> (Vec<PanelLensStatus>, Option<ResourceUsage>) {
    let mut used = ResourceUsage::default();
    let lenses = entries
        .into_iter()
        .map(|entry| {
            let usage = ResourceUsage::from_lens_cost(entry.cost);
            let remaining = budget.map(|cap| {
                used = used.saturating_add(usage);
                budget_usage(cap).remaining_after(used)
            });
            status_from_entry(entry, remaining)
        })
        .collect::<Vec<_>>();
    let remaining = budget.map(|cap| budget_usage(cap).remaining_after(used));
    (lenses, remaining)
}

fn status_from_entry(
    entry: LensCatalogEntry,
    remaining_budget_after: Option<ResourceUsage>,
) -> PanelLensStatus {
    PanelLensStatus {
        lens_id: entry.lens_id,
        name: entry.name,
        runtime: entry.runtime,
        placement: entry.placement,
        ram_mb: mib(entry.cost.ram_bytes),
        vram_mb: mib(entry.cost.vram_bytes),
        batch_ceiling: entry.cost.batch_ceiling,
        cost: entry.cost,
        health: health_from_manifest(&entry.manifest),
        manifest: entry.manifest,
        remaining_budget_after,
    }
}

fn health_from_manifest(path: &Path) -> LensHealth {
    match lens_spec_from_manifest_path(path) {
        Ok(spec) => spec.health(),
        Err(error) => LensHealth::Failing {
            code: error.code.to_string(),
            reason: error.message,
        },
    }
}

fn vault_slot_status(
    slots: Vec<PanelSlotListing>,
    budget: Option<PanelResourceBudget>,
) -> (Vec<PanelSlotStatus>, Option<ResourceUsage>) {
    let mut used = ResourceUsage::default();
    let statuses = slots
        .into_iter()
        .map(|listing| {
            let cost = listing.resource.cost;
            let placement = listing.resource.placement;
            let usage = ResourceUsage::from_lens_cost(cost);
            let density = match (listing.bits_about, budget) {
                (Some(bits), Some(cap)) => {
                    Some(ResourceDensity::compute(bits, usage, placement, cap))
                }
                _ => None,
            };
            let remaining = budget.and_then(|cap| {
                if listing.state == SlotState::Retired {
                    None
                } else {
                    used = used.saturating_add(usage);
                    Some(budget_usage(cap).remaining_after(used))
                }
            });
            PanelSlotStatus {
                listing,
                cost,
                placement,
                ram_mb: mib(cost.ram_bytes),
                vram_mb: mib(cost.vram_bytes),
                density,
                remaining_budget_after: remaining,
            }
        })
        .collect::<Vec<_>>();
    let remaining = budget.map(|cap| budget_usage(cap).remaining_after(used));
    (statuses, remaining)
}

#[derive(Default)]
struct Flags {
    home: Option<PathBuf>,
    vault: Option<PathBuf>,
    panel_budget_json: Option<PathBuf>,
}

impl Flags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut flags = Self::default();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--home" => {
                    idx += 1;
                    flags.home = Some(value(args, idx, "--home")?.into());
                }
                "--vault" => {
                    idx += 1;
                    flags.vault = Some(value(args, idx, "--vault")?.into());
                }
                "--panel-budget-json" => {
                    idx += 1;
                    flags.panel_budget_json = Some(value(args, idx, "--panel-budget-json")?.into());
                }
                other => return Err(CliError::usage(format!("unexpected panel flag {other}"))),
            }
            idx += 1;
        }
        if flags.home.is_some() && flags.vault.is_some() {
            return Err(CliError::usage(
                "calyx panel status accepts --home or --vault, not both",
            ));
        }
        Ok(flags)
    }
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn catalog_path(home: Option<&Path>) -> CliResult<PathBuf> {
    let root = match home {
        Some(path) => path.to_path_buf(),
        None => env::var_os("CALYX_HOME")
            .map(PathBuf::from)
            .ok_or_else(|| CliError::usage("CALYX_HOME is required or pass --home <dir>"))?,
    };
    Ok(root.join("lenses").join("registry.json"))
}

fn read_catalog(path: &Path) -> CliResult<LensCatalog> {
    if !path.exists() {
        return Ok(LensCatalog { lenses: Vec::new() });
    }
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|err| CliError::usage(format!("parse lens catalog {}: {err}", path.display())))
}

fn read_budget(path: &Path) -> CliResult<PanelResourceBudget> {
    let bytes = fs::read(path)?;
    let budget: PanelResourceBudget = serde_json::from_slice(&bytes)
        .map_err(|err| CliError::usage(format!("parse panel budget {}: {err}", path.display())))?;
    pack_panel_by_density(&[], budget).map_err(|error| {
        CliError::usage(format!(
            "invalid panel budget {}: {}: {}",
            path.display(),
            error.code,
            error.message
        ))
    })?;
    Ok(budget)
}

fn budget_usage(budget: PanelResourceBudget) -> ResourceUsage {
    ResourceUsage {
        vram_mb: budget.max_vram_mb,
        ram_mb: budget.max_ram_mb,
        ms_per_input: budget.max_ms_per_input,
    }
}

fn mib(bytes: u64) -> f32 {
    bytes as f32 / (1024.0 * 1024.0)
}
