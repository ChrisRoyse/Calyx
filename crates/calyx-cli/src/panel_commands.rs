use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{LensCost, Placement};
use calyx_registry::{PanelSlotListing, list_panel, load_vault_panel_state};
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
    lenses: Vec<PanelLensStatus>,
}

#[derive(Serialize)]
struct VaultPanelStatusReport {
    vault: PathBuf,
    panel_version: u32,
    slot_count: usize,
    registry_lens_count: usize,
    panel_ref: Option<String>,
    slots: Vec<PanelSlotListing>,
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
}

pub(crate) fn run(topic: &str, rest: &[String]) -> CliResult {
    match topic {
        "status" => status(rest),
        other => Err(CliError::usage(format!(
            "unknown panel subcommand {other}; expected status"
        ))),
    }
}

fn status(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    if let Some(vault) = flags.vault {
        return status_vault(vault);
    }
    let catalog_path = catalog_path(flags.home.as_deref())?;
    let catalog = read_catalog(&catalog_path)?;
    let lenses = catalog
        .lenses
        .into_iter()
        .map(status_from_entry)
        .collect::<Vec<_>>();
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
        lenses,
    })
}

fn status_vault(vault: PathBuf) -> CliResult {
    let state = load_vault_panel_state(&vault)?;
    let slots = list_panel(&state.panel, &state.registry);
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
        slots,
    })
}

fn status_from_entry(entry: LensCatalogEntry) -> PanelLensStatus {
    PanelLensStatus {
        lens_id: entry.lens_id,
        name: entry.name,
        runtime: entry.runtime,
        placement: entry.placement,
        ram_mb: mib(entry.cost.ram_bytes),
        vram_mb: mib(entry.cost.vram_bytes),
        batch_ceiling: entry.cost.batch_ceiling,
        cost: entry.cost,
        manifest: entry.manifest,
    }
}

#[derive(Default)]
struct Flags {
    home: Option<PathBuf>,
    vault: Option<PathBuf>,
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

fn mib(bytes: u64) -> f32 {
    bytes as f32 / (1024.0 * 1024.0)
}
