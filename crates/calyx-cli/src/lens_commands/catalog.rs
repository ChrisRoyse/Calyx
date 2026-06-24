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
use calyxd::vram::{NvmlVramUsage, VramUsage};
use serde::{Deserialize, Serialize};

use super::flags::Flags;
use super::support::{dim, hex_from_bytes, runtime_name};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

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

pub(super) fn catalog_path(home: Option<&Path>) -> CliResult<PathBuf> {
    let root = match home {
        Some(path) => path.to_path_buf(),
        None => env::var_os("CALYX_HOME")
            .map(PathBuf::from)
            .ok_or_else(|| CliError::usage("CALYX_HOME is required or pass --home <dir>"))?,
    };
    Ok(root.join("lenses").join("registry.json"))
}

pub(super) fn read_catalog(path: &Path) -> CliResult<LensCatalog> {
    if !path.exists() {
        return Ok(LensCatalog { lenses: Vec::new() });
    }
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|err| CliError::usage(format!("parse lens catalog {}: {err}", path.display())))
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

pub(super) fn write_catalog(path: &Path, catalog: &LensCatalog) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(catalog)
        .map_err(|err| CliError::usage(format!("serialize lens catalog: {err}")))?;
    fs::write(path, bytes)?;
    Ok(())
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

fn placement_budget_from_catalog(catalog: &LensCatalog) -> CliResult<PlacementBudget> {
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
    // Operator-facing diagnostic: make the GPU placement budget visible so a
    // CPU spill is never silent. cap/reservation source is env-override vs live
    // NVML probe (see resolve_gpu_vram_budget).
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

/// VRAM carved out of the board for non-Calyx GPU users + allocation spikes
/// (a model's declared FP32 cost underestimates its real peak working set). On a
/// 32 GiB board this leaves ~28 GiB usable, matching the operator's ceiling.
/// Override with `CALYX_GPU_HEADROOM_BYTES`.
const DEFAULT_GPU_HEADROOM_BYTES: u64 = 4 * gib();

/// Resolve the GPU VRAM soft cap and co-resident reservation that gate lens
/// placement, from **live device reality** rather than a fixed board assumption.
///
/// Why this exists: a hard-coded 20 GiB TEI reservation (the historic default)
/// starved the placement budget so small ONNX lenses silently spilled to CPU and
/// new GPU commissions failed with `CALYX_VRAM_BUDGET_EXCEEDED` — the root cause
/// of the throughput collapse. The cap now defaults to `board_total - headroom`
/// and the reservation to the VRAM *already in use by every other process on the
/// device* (resident TEI servers, the operator's other GPU apps), read via the
/// same NVML probe the daemon uses. Placement therefore self-adjusts to whatever
/// GPU it runs on, on a fresh checkout, with no machine-specific tuning.
///
/// Explicit env overrides (`CALYX_PANEL_VRAM_SOFT_CAP_BYTES`,
/// `CALYX_TEI_RESERVED_BYTES`) always win. If a needed value has no override and
/// the probe fails, this fails loud with remediation — it never falls back to a
/// guessed budget (doctrine: fail closed, no silent workaround).
///
/// Note: the live `used` reading also counts any Calyx lenses already resident in
/// a running daemon. At commission time the CLI holds no panel, so `used` ≈ the
/// non-Calyx footprint, which is exactly the reservation we want; if a daemon is
/// mid-ingest the reservation is conservatively larger, which fails safe (fewer
/// GPU placements, never an over-commit).
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
                 budgets, or ensure the NVIDIA driver (libnvidia-ml.so.1) is reachable"
            ))
        })?)
    };
    compute_vram_budget(cap_override, tei_override, probe, headroom)
}

/// Pure budget arithmetic, separated from env/NVML IO so the resolution logic is
/// unit-testable on a CPU-only host with hand-set readings.
fn compute_vram_budget(
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

/// Live `(total, used)` device VRAM in bytes via NVML — the same source of truth
/// the daemon's budget enforcer uses, so the CLI and daemon agree on the device.
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

const fn gib() -> u64 {
    1024 * 1024 * 1024
}

#[cfg(test)]
mod tests;
