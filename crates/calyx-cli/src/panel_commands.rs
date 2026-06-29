use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

mod a38_bundle;
mod resident;
mod template_cards;
mod template_model;
mod template_store;
mod templates;
mod warm;

use calyx_assay::{PanelResourceBudget, ResourceDensity, ResourceUsage, pack_panel_by_density};
use calyx_core::{CalyxError, Input, LensCost, LensId, Modality, Panel, Placement, SlotState};
use calyx_registry::{
    LensHealth, PanelSlotListing, Registry, RegistryBatchLimitChange, RegistryBatchLimitUpdate,
    RegistrySnapshotMeasureStats, VaultRegistrySnapshot, apply_registry_snapshot_batch_limits,
    lens_spec_from_manifest_path, list_panel, load_vault_panel_state,
    measure_registry_snapshot_lens_batch_with_stats, set_vault_registry_batch_limits,
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

#[derive(Serialize)]
struct BatchLimitReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: PathBuf,
    manifest_seq: u64,
    durable_seq: u64,
    registry_ref: String,
    wrote_manifest: bool,
    requested_count: usize,
    changed_count: usize,
    preflight_count: usize,
    changes: Vec<BatchLimitChangeReport>,
}

#[derive(Serialize)]
struct BatchLimitChangeReport {
    lens_id: String,
    name: String,
    before: Option<usize>,
    after: usize,
    changed: bool,
    active_slot_count: usize,
    reloaded_max_batch: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    preflight: Option<BatchLimitPreflightReport>,
}

#[derive(Clone, Debug, Serialize)]
struct BatchLimitPreflightReport {
    input_count: usize,
    runtime_batch_limit: Option<usize>,
    effective_chunk_size: usize,
    chunk_count: usize,
    runtime_load_ms: u128,
    measure_ms: u128,
    total_ms: u128,
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
        "batch-limit" => batch_limit(rest),
        "status" => status(rest),
        "template" => templates::run(rest),
        "resident" => resident::run(rest),
        "warm" => warm::run(rest),
        other => Err(CliError::usage(format!(
            "unknown panel subcommand {other}; expected a38-bundle, batch-limit, status, template, resident, or warm"
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

fn batch_limit(args: &[String]) -> CliResult {
    let flags = BatchLimitFlags::parse(args)?;
    let state = load_vault_panel_state(&flags.vault)?;
    let snapshot = state.registry_snapshot.as_ref().ok_or_else(|| {
        CliError::from(CalyxError::aster_corrupt_shard(
            "vault has no persisted registry snapshot; cannot update lens batch limits",
        ))
    })?;
    let updates = resolve_batch_limit_updates(snapshot, &flags.sets)?;
    let mut preview_snapshot = snapshot.clone();
    let preview_changes = apply_registry_snapshot_batch_limits(&mut preview_snapshot, &updates)?;
    let preflight = preflight_batch_limit_changes(&preview_snapshot, &preview_changes, &flags)?;
    let write = set_vault_registry_batch_limits(&flags.vault, &updates)?;
    let reloaded = load_vault_panel_state(&flags.vault)?;
    let changes = verify_batch_limit_write(&flags.vault, &reloaded, &write.changes, &preflight)?;
    print_json(&BatchLimitReport {
        status: "batch_limits_updated",
        source_of_truth: "vault MANIFEST registry_ref plus manifest-backed registry asset reloaded via load_vault_panel_state",
        vault: flags.vault,
        manifest_seq: write.manifest_seq,
        durable_seq: write.durable_seq,
        registry_ref: write.registry_ref.logical_path,
        wrote_manifest: write.wrote_manifest,
        requested_count: updates.len(),
        changed_count: write.changes.iter().filter(|change| change.changed).count(),
        preflight_count: preflight.len(),
        changes,
    })
}

fn resolve_batch_limit_updates(
    snapshot: &VaultRegistrySnapshot,
    sets: &[BatchLimitSet],
) -> CliResult<Vec<RegistryBatchLimitUpdate>> {
    let mut updates = Vec::with_capacity(sets.len());
    for set in sets {
        let lens_id = resolve_batch_limit_selector(snapshot, &set.selector)?;
        updates.push(RegistryBatchLimitUpdate {
            lens_id,
            max_batch: set.max_batch,
        });
    }
    Ok(updates)
}

fn resolve_batch_limit_selector(
    snapshot: &VaultRegistrySnapshot,
    selector: &str,
) -> CliResult<LensId> {
    if let Ok(lens_id) = LensId::from_str(selector) {
        return Ok(lens_id);
    }
    let matches = snapshot
        .lenses
        .iter()
        .filter_map(|lens| {
            lens.spec
                .as_ref()
                .filter(|spec| spec.name == selector)
                .map(|_| lens.lens_id)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [lens_id] => Ok(*lens_id),
        [] => Err(CliError::usage(format!(
            "batch-limit selector {selector} did not match a persisted lens name or LensId"
        ))),
        _ => Err(CliError::usage(format!(
            "batch-limit selector {selector} matched multiple persisted lenses; use LensId"
        ))),
    }
}

fn preflight_batch_limit_changes(
    snapshot: &VaultRegistrySnapshot,
    changes: &[RegistryBatchLimitChange],
    flags: &BatchLimitFlags,
) -> CliResult<Vec<(LensId, BatchLimitPreflightReport)>> {
    let mut reports = Vec::new();
    for change in changes.iter().filter(|change| change.changed) {
        let lens = snapshot
            .lenses
            .iter()
            .find(|lens| lens.lens_id == change.lens_id)
            .ok_or_else(|| {
                CliError::from(CalyxError::lens_unreachable(format!(
                    "preflight lens {} missing from preview registry snapshot",
                    change.lens_id
                )))
            })?;
        let modality = lens.contract.modality();
        if modality != Modality::Text {
            return Err(CliError::from(CalyxError {
                code: "CALYX_REGISTRY_BATCH_LIMIT_PREFLIGHT_UNSUPPORTED",
                message: format!(
                    "batch-limit preflight currently supports Text lenses, but {} ({}) is {:?}",
                    change.name, change.lens_id, modality
                ),
                remediation: "add a modality-specific preflight input generator before changing this non-text lens batch limit",
            }));
        }
        let input_count = flags.preflight_repeat.unwrap_or(change.after).max(1);
        let inputs = (0..input_count)
            .map(|idx| {
                Input::new(
                    Modality::Text,
                    format!("{} #{idx}", flags.preflight_text).into_bytes(),
                )
            })
            .collect::<Vec<_>>();
        let (_, stats) =
            measure_registry_snapshot_lens_batch_with_stats(lens, &inputs, Some(change.after))?;
        reports.push((change.lens_id, BatchLimitPreflightReport::from(stats)));
    }
    Ok(reports)
}

fn verify_batch_limit_write(
    vault: &Path,
    reloaded: &calyx_registry::VaultPanelState,
    changes: &[RegistryBatchLimitChange],
    preflight: &[(LensId, BatchLimitPreflightReport)],
) -> CliResult<Vec<BatchLimitChangeReport>> {
    let snapshot = reloaded.registry_snapshot.as_ref().ok_or_else(|| {
        CliError::from(CalyxError::aster_corrupt_shard(format!(
            "vault {} lost registry snapshot after batch-limit write",
            vault.display()
        )))
    })?;
    let mut reports = Vec::with_capacity(changes.len());
    for change in changes {
        let lens = snapshot
            .lenses
            .iter()
            .find(|lens| lens.lens_id == change.lens_id)
            .ok_or_else(|| {
                CliError::from(CalyxError::aster_corrupt_shard(format!(
                    "vault {} reloaded registry is missing changed lens {}",
                    vault.display(),
                    change.lens_id
                )))
            })?;
        let reloaded_max_batch = lens
            .spec
            .as_ref()
            .and_then(|spec| spec.max_batch)
            .ok_or_else(|| {
                CliError::from(CalyxError::aster_corrupt_shard(format!(
                    "vault {} reloaded lens {} has no max_batch after batch-limit write",
                    vault.display(),
                    change.lens_id
                )))
            })?;
        if reloaded_max_batch != change.after {
            return Err(CliError::from(CalyxError {
                code: "CALYX_REGISTRY_BATCH_LIMIT_VERIFY_FAILED",
                message: format!(
                    "vault {} reloaded lens {} max_batch={}, expected {}",
                    vault.display(),
                    change.lens_id,
                    reloaded_max_batch,
                    change.after
                ),
                remediation: "inspect the vault MANIFEST registry_ref and retry the batch-limit command after repairing registry persistence",
            }));
        }
        let active_slot_count = reloaded
            .panel
            .slots
            .iter()
            .filter(|slot| slot.lens_id == change.lens_id && slot.state == SlotState::Active)
            .count();
        reports.push(BatchLimitChangeReport {
            lens_id: change.lens_id.to_string(),
            name: change.name.clone(),
            before: change.before,
            after: change.after,
            changed: change.changed,
            active_slot_count,
            reloaded_max_batch,
            preflight: preflight
                .iter()
                .find(|(lens_id, _)| *lens_id == change.lens_id)
                .map(|(_, report)| report.clone()),
        });
    }
    Ok(reports)
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

struct BatchLimitFlags {
    vault: PathBuf,
    sets: Vec<BatchLimitSet>,
    preflight_text: String,
    preflight_repeat: Option<usize>,
}

struct BatchLimitSet {
    selector: String,
    max_batch: usize,
}

impl BatchLimitFlags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut sets = Vec::new();
        let mut preflight_text = "calyx batch-limit preflight".to_string();
        let mut preflight_repeat = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => {
                    idx += 1;
                    vault = Some(value(args, idx, "--vault")?.into());
                }
                "--set" => {
                    idx += 1;
                    sets.push(parse_batch_limit_set(value(args, idx, "--set")?)?);
                }
                "--preflight-text" => {
                    idx += 1;
                    preflight_text = value(args, idx, "--preflight-text")?.to_string();
                    if preflight_text.is_empty() {
                        return Err(CliError::usage("--preflight-text must not be empty"));
                    }
                }
                "--preflight-repeat" => {
                    idx += 1;
                    let raw = value(args, idx, "--preflight-repeat")?;
                    let parsed = raw.parse::<usize>().map_err(|error| {
                        CliError::usage(format!("parse --preflight-repeat {raw}: {error}"))
                    })?;
                    if parsed == 0 {
                        return Err(CliError::usage("--preflight-repeat must be > 0"));
                    }
                    preflight_repeat = Some(parsed);
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected panel batch-limit flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        let vault = vault.ok_or_else(|| CliError::usage("panel batch-limit requires --vault"))?;
        if sets.is_empty() {
            return Err(CliError::usage(
                "panel batch-limit requires at least one --set <name-or-id>=<max_batch>",
            ));
        }
        Ok(Self {
            vault,
            sets,
            preflight_text,
            preflight_repeat,
        })
    }
}

fn parse_batch_limit_set(raw: &str) -> CliResult<BatchLimitSet> {
    let (selector, max_batch) = raw
        .split_once('=')
        .ok_or_else(|| CliError::usage("--set must use <name-or-id>=<max_batch>"))?;
    if selector.is_empty() {
        return Err(CliError::usage("--set selector must not be empty"));
    }
    let max_batch = max_batch
        .parse::<usize>()
        .map_err(|error| CliError::usage(format!("parse batch limit {raw}: {error}")))?;
    if max_batch == 0 {
        return Err(CliError::usage("--set max_batch must be > 0"));
    }
    Ok(BatchLimitSet {
        selector: selector.to_string(),
        max_batch,
    })
}

impl From<RegistrySnapshotMeasureStats> for BatchLimitPreflightReport {
    fn from(stats: RegistrySnapshotMeasureStats) -> Self {
        Self {
            input_count: stats.input_count,
            runtime_batch_limit: stats.runtime_batch_limit,
            effective_chunk_size: stats.effective_chunk_size,
            chunk_count: stats.chunk_count,
            runtime_load_ms: stats.runtime_load_ms,
            measure_ms: stats.measure_ms,
            total_ms: stats.total_ms,
        }
    }
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
