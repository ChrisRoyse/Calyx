use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::{Input, Lens, LensCost, Placement};
use calyx_registry::{
    CALYX_VRAM_BUDGET_EXCEEDED, LENS_VRAM_REMEDIATION, LensHealth, LensRuntime, LensSpec,
    MultimodalAdapterLens, PlacementBudget, StaticLookupLens, choose_placement,
    lens_spec_from_manifest_path, lens_spec_metadata_from_manifest_path,
};
use serde::{Deserialize, Serialize};

use super::flags::{Flags, value};
use super::support::{dim, hex_from_bytes, runtime_name};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

mod budget;
mod store;

pub(crate) use store::LensCatalogDbReadback;

use budget::placement_budget_from_catalog;

#[cfg(test)]
use budget::compute_vram_budget;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LensCatalog {
    pub(crate) lenses: Vec<LensCatalogEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LensCatalogEntry {
    pub(crate) lens_id: String,
    pub(crate) name: String,
    pub(crate) modality: String,
    pub(crate) runtime: String,
    pub(crate) dim: u32,
    #[serde(default)]
    pub(crate) retrieval_only: bool,
    #[serde(default)]
    pub(crate) excluded_from_dedup: bool,
    pub(crate) weights_sha256: String,
    pub(crate) manifest: PathBuf,
    #[serde(default)]
    pub(crate) cost: LensCost,
    #[serde(default)]
    pub(crate) placement: Placement,
}

#[derive(Serialize)]
pub(crate) struct AddReport {
    pub(crate) catalog: PathBuf,
    pub(crate) lens_id: String,
    pub(crate) name: String,
    pub(crate) manifest: PathBuf,
    pub(crate) cost: LensCost,
    pub(crate) placement: Placement,
    pub(crate) count: usize,
}

#[derive(Serialize)]
struct ListReport {
    catalog: PathBuf,
    count: usize,
    lenses: Vec<ListLensEntry>,
}

#[derive(Serialize)]
struct ListLensEntry {
    #[serde(flatten)]
    entry: LensCatalogEntry,
    health: LensHealth,
}

#[derive(Serialize)]
struct MigrateReport {
    source: PathBuf,
    catalog: PathBuf,
    count: usize,
    readback: LensCatalogDbReadback,
}

#[derive(Default)]
struct MigrateFlags {
    home: Option<PathBuf>,
    from: Option<PathBuf>,
}

pub(crate) fn add(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    flags.reject_measure_flags("calyx lens add")?;
    let manifest = flags
        .manifest
        .ok_or_else(|| CliError::usage("calyx lens add requires --manifest <path>"))?;
    let report = add_manifest_to_catalog(flags.home.as_deref(), manifest)?;
    print_json(&report)
}

pub(crate) fn list(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    flags.reject_measure_flags("calyx lens list")?;
    if flags.manifest.is_some() {
        return Err(CliError::usage(
            "calyx lens list does not accept --manifest",
        ));
    }
    let catalog_path = catalog_path(flags.home.as_deref())?;
    let catalog = read_catalog(&catalog_path)?;
    print_json(&ListReport {
        catalog: catalog_path,
        count: catalog.lenses.len(),
        lenses: catalog.lenses.into_iter().map(list_entry).collect(),
    })
}

pub(crate) fn migrate_catalog(args: &[String]) -> CliResult {
    let flags = MigrateFlags::parse(args)?;
    let catalog_path = catalog_path(flags.home.as_deref())?;
    let source = flags
        .from
        .unwrap_or_else(|| store::legacy_catalog_path(&catalog_path));
    let catalog = read_legacy_catalog(&source)?;
    let readback = write_catalog(&catalog_path, &catalog)?;
    print_json(&MigrateReport {
        source,
        catalog: catalog_path,
        count: catalog.lenses.len(),
        readback,
    })
}

pub(crate) fn add_manifest_to_catalog(
    home: Option<&Path>,
    manifest: PathBuf,
) -> CliResult<AddReport> {
    let spec = lens_spec_from_manifest_path(&manifest)?;
    let catalog_path = catalog_path(home)?;
    let mut catalog = read_catalog(&catalog_path)?;
    let lens_id = spec.lens_id().to_string();
    retain_unrelated_entries(&mut catalog, &lens_id, &spec.name, &manifest);
    let budget = placement_budget_from_catalog(&catalog)?;
    let entry = entry_from_spec(&spec, manifest, budget)?;
    retain_unrelated_entries(&mut catalog, &entry.lens_id, &entry.name, &entry.manifest);
    catalog.lenses.push(entry.clone());
    catalog
        .lenses
        .sort_by(|left, right| left.lens_id.cmp(&right.lens_id));
    write_catalog(&catalog_path, &catalog)?;
    Ok(AddReport {
        catalog: catalog_path,
        lens_id: entry.lens_id,
        name: entry.name,
        manifest: entry.manifest,
        cost: entry.cost,
        placement: entry.placement,
        count: catalog.lenses.len(),
    })
}

fn retain_unrelated_entries(catalog: &mut LensCatalog, lens_id: &str, name: &str, manifest: &Path) {
    catalog
        .lenses
        .retain(|item| !same_catalog_identity(item, lens_id, name, manifest));
}

fn same_catalog_identity(
    entry: &LensCatalogEntry,
    lens_id: &str,
    name: &str,
    manifest: &Path,
) -> bool {
    entry.lens_id == lens_id || entry.name == name || entry.manifest == manifest
}

impl MigrateFlags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut flags = Self::default();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--home" => {
                    idx += 1;
                    flags.home = Some(value(args, idx, "--home")?.into());
                }
                "--from" => {
                    idx += 1;
                    flags.from = Some(value(args, idx, "--from")?.into());
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected lens migrate-catalog flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        Ok(flags)
    }
}

pub(crate) fn catalog_path(home: Option<&Path>) -> CliResult<PathBuf> {
    let root = match home {
        Some(path) => path.to_path_buf(),
        None => env::var_os("CALYX_HOME")
            .map(PathBuf::from)
            .ok_or_else(|| CliError::usage("CALYX_HOME is required or pass --home <dir>"))?,
    };
    Ok(root.join("lenses").join("catalog-db"))
}

pub(crate) fn read_catalog(path: &Path) -> CliResult<LensCatalog> {
    Ok(store::read(path)?)
}

pub(crate) fn read_catalog_with_readback(
    path: &Path,
) -> CliResult<(LensCatalog, LensCatalogDbReadback)> {
    Ok(store::read_with_readback(path)?)
}

fn read_legacy_catalog(path: &Path) -> CliResult<LensCatalog> {
    if !path.exists() {
        return Err(CliError::usage(format!(
            "legacy lens catalog {} does not exist",
            path.display()
        )));
    }
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(|err| {
        CliError::usage(format!(
            "parse legacy lens catalog {}: {err}",
            path.display()
        ))
    })
}

fn list_entry(entry: LensCatalogEntry) -> ListLensEntry {
    let health = health_from_manifest(&entry.manifest);
    ListLensEntry { entry, health }
}

fn health_from_manifest(path: &Path) -> LensHealth {
    match lens_spec_metadata_from_manifest_path(path) {
        Ok(spec) => spec.health(),
        Err(error) => LensHealth::Failing {
            code: error.code.to_string(),
            reason: error.message,
        },
    }
}

pub(crate) fn write_catalog(
    path: &Path,
    catalog: &LensCatalog,
) -> CliResult<LensCatalogDbReadback> {
    Ok(store::write(path, catalog)?)
}

fn entry_from_spec(
    spec: &LensSpec,
    manifest: PathBuf,
    budget: PlacementBudget,
) -> CliResult<LensCatalogEntry> {
    let cost = estimate_lens_cost(spec)?;
    let placement = placement_from_spec(spec, cost, budget)?;
    Ok(LensCatalogEntry {
        lens_id: spec.lens_id().to_string(),
        name: spec.name.clone(),
        modality: format!("{:?}", spec.modality).to_lowercase(),
        runtime: runtime_name(&spec.runtime).to_string(),
        dim: dim(spec.output),
        retrieval_only: spec.retrieval_only,
        excluded_from_dedup: spec.excluded_from_dedup,
        weights_sha256: hex_from_bytes(&spec.weights_sha256),
        manifest,
        cost,
        placement,
    })
}

fn placement_from_spec(
    spec: &LensSpec,
    cost: LensCost,
    budget: PlacementBudget,
) -> CliResult<Placement> {
    if let LensRuntime::MultimodalAdapter { .. } = &spec.runtime {
        let lens = MultimodalAdapterLens::from_lens_spec(spec)?;
        if lens.provider().is_gpu() {
            ensure_vram_available(cost, budget)?;
            return Ok(Placement::Gpu);
        }
    }
    Ok(choose_placement(&spec.runtime, cost, budget)?
        .resource
        .placement)
}

fn ensure_vram_available(cost: LensCost, budget: PlacementBudget) -> CliResult {
    if cost.vram_bytes <= budget.available_vram_bytes() {
        return Ok(());
    }
    Err(calyx_core::CalyxError {
        code: CALYX_VRAM_BUDGET_EXCEEDED,
        message: format!(
            "lens requires {} VRAM bytes, available {} after TEI reservation {} and allocated {}",
            cost.vram_bytes,
            budget.available_vram_bytes(),
            budget.tei_reserved_bytes,
            budget.vram_allocated_bytes
        ),
        remediation: LENS_VRAM_REMEDIATION,
    }
    .into())
}

fn estimate_lens_cost(spec: &LensSpec) -> CliResult<LensCost> {
    match &spec.runtime {
        LensRuntime::Algorithmic { .. }
        | LensRuntime::ExternalCmd { .. }
        | LensRuntime::TeiHttp { .. } => Ok(LensCost::zero()),
        LensRuntime::MultimodalAdapter { files, .. } => {
            let bytes = files_size(files)?;
            let lens = MultimodalAdapterLens::from_lens_spec(spec)?;
            if lens.provider().is_gpu() {
                return Ok(LensCost {
                    total_ms: 0.0,
                    ms_per_input: 0.0,
                    vram_bytes: bytes,
                    ram_bytes: bytes,
                    batch_ceiling: u32::MAX,
                });
            }
            Ok(LensCost {
                total_ms: 0.0,
                ms_per_input: 0.0,
                vram_bytes: 0,
                ram_bytes: bytes,
                batch_ceiling: u32::MAX,
            })
        }
        LensRuntime::StaticLookup {
            embeddings_file,
            tokenizer,
            ..
        } => measure_static_lookup_cost(spec, embeddings_file, tokenizer),
        LensRuntime::CandleLocal { files, .. }
        | LensRuntime::Onnx { files, .. }
        | LensRuntime::OnnxColbert { files, .. }
        | LensRuntime::FastembedSparse { files, .. }
        | LensRuntime::FastembedBgem3 { files, .. }
        | LensRuntime::FastembedReranker { files, .. }
        | LensRuntime::FastembedQwen3 { files, .. } => {
            let bytes = files_size(files)?;
            Ok(LensCost {
                total_ms: 0.0,
                ms_per_input: 0.0,
                vram_bytes: bytes,
                ram_bytes: bytes,
                batch_ceiling: u32::MAX,
            })
        }
    }
}

fn measure_static_lookup_cost(
    spec: &LensSpec,
    embeddings_file: &Path,
    tokenizer: &Path,
) -> CliResult<LensCost> {
    let lens = StaticLookupLens::from_lens_spec(spec)?;
    let probe = Input::new(
        spec.modality,
        b"Calyx lens admission profile probe".to_vec(),
    );
    let started = Instant::now();
    let _vector = lens.measure(&probe)?;
    let total_ms = started.elapsed().as_secs_f64() as f32 * 1000.0;
    Ok(LensCost {
        total_ms,
        ms_per_input: total_ms,
        vram_bytes: 0,
        ram_bytes: path_size(embeddings_file)?.saturating_add(path_size(tokenizer)?),
        batch_ceiling: batch_ceiling(total_ms),
    })
}

fn files_size(files: &[PathBuf]) -> CliResult<u64> {
    files
        .iter()
        .try_fold(0_u64, |acc, path| Ok(acc.saturating_add(path_size(path)?)))
}

fn path_size(path: &Path) -> CliResult<u64> {
    Ok(fs::metadata(path)?.len())
}

fn batch_ceiling(ms_per_input: f32) -> u32 {
    if !ms_per_input.is_finite() || ms_per_input < 0.0 {
        return 1;
    }
    if ms_per_input <= f32::EPSILON {
        return u32::MAX;
    }
    (1_000.0 / ms_per_input).floor().clamp(1.0, u32::MAX as f32) as u32
}

#[cfg(test)]
mod tests;
