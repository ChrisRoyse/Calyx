use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

mod a38_bundle;
mod batch_limit;
mod manifest_restore;
mod registry_contract;
mod resident;
mod template_cards;
mod template_model;
mod template_store;
mod templates;
mod warm;

pub(crate) use resident::measure_batch_at as measure_resident_batch_at;
pub(crate) use resident::ready_value_at as resident_ready_value_at;
pub(crate) use resident::{ResidentDiscovery, read_resident_discovery, resident_discovery_path};

use calyx_assay::{PanelResourceBudget, ResourceDensity, ResourceUsage, pack_panel_by_density};
use calyx_core::{CalyxError, Input, LensCost, LensId, Modality, Panel, Placement, SlotState};
use calyx_registry::{
    LensHealth, PanelSlotListing, Registry, RegistryBatchLimitChange, RegistryBatchLimitUpdate,
    RegistrySnapshotMeasureStats, VaultRegistrySnapshot, apply_registry_snapshot_batch_limits,
    lens_spec_from_manifest_path, list_panel, load_vault_panel_state,
    measure_registry_snapshot_lens_batch_with_stats, set_vault_registry_batch_limits,
};
use serde::Serialize;

use crate::error::{CliError, CliResult};
use crate::lens_commands::catalog::{catalog_path, read_catalog};
use crate::output::print_json;

use manifest_restore::manifest_restore;
use registry_contract::{registry_audit, registry_repair};

#[cfg(test)]
pub(crate) use crate::lens_commands::catalog::LensCatalog;
pub(crate) use crate::lens_commands::catalog::LensCatalogEntry;

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

pub(crate) struct SavedTemplatePanelBuild {
    pub template_id: String,
    pub template_name: String,
    pub panel: Panel,
    pub registry: Registry,
    pub content_lens_count: usize,
    pub a37_gate_eligible: bool,
    pub a37_status: String,
    pub registered_lenses_added: usize,
}

pub(crate) fn build_saved_template_panel(
    home: &Path,
    selector: &str,
    now_ms: u64,
) -> CliResult<SavedTemplatePanelBuild> {
    build_saved_template_panel_with_progress(home, selector, now_ms, None)
}

fn build_saved_template_panel_with_progress(
    home: &Path,
    selector: &str,
    now_ms: u64,
    progress: Option<&mut dyn FnMut(template_store::TemplateLensProgress) -> CliResult<()>>,
) -> CliResult<SavedTemplatePanelBuild> {
    let store = template_store::TemplateStore::open(home);
    let mut template = store.load(selector)?;
    template.validate()?;
    let a37 = template.a37_admission();
    let template_id = template_store::id_for_loaded(&template)?;
    let mut registry = Registry::new();
    let registered_lenses_added = template_store::register_template_lenses_with_progress(
        &mut registry,
        &mut template,
        progress,
    )?;
    let panel = template.to_target_panel(now_ms);
    let content_lens_count = a37.content_lens_count.max(panel_content_lens_count(&panel));
    Ok(SavedTemplatePanelBuild {
        template_id,
        template_name: template.name,
        panel,
        registry,
        content_lens_count,
        a37_gate_eligible: a37.gate_eligible,
        a37_status: a37.status,
        registered_lenses_added,
    })
}

pub(crate) fn saved_template_names(home: &Path) -> CliResult<Vec<String>> {
    let store = template_store::TemplateStore::open(home);
    Ok(store
        .list()?
        .into_iter()
        .map(|template| template.name)
        .collect())
}

pub(crate) fn run(topic: &str, rest: &[String]) -> CliResult {
    match topic {
        "a38-bundle" => a38_bundle::run(rest),
        "batch-limit" => batch_limit::batch_limit(rest),
        "manifest-restore" => manifest_restore(rest),
        "registry-audit" => registry_audit(rest),
        "registry-repair" => registry_repair(rest),
        "status" => status(rest),
        "template" => templates::run(rest),
        "resident" => resident::run(rest),
        "warm" => warm::run(rest),
        other => Err(CliError::usage(format!(
            "unknown panel subcommand {other}; expected a38-bundle, batch-limit, manifest-restore, registry-audit, registry-repair, status, template, resident, or warm"
        ))),
    }
}

fn panel_content_lens_count(panel: &Panel) -> usize {
    panel
        .slots
        .iter()
        .filter(|slot| !slot.retrieval_only && !slot.excluded_from_dedup)
        .count()
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
