use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::{Input, Lens, LensCost, Placement};
use calyx_registry::{
    LensHealth, LensRuntime, LensSpec, PlacementBudget, StaticLookupLens, choose_placement,
    lens_spec_from_manifest_path, lens_spec_metadata_from_manifest_path,
};
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
    let plan = choose_placement(&spec.runtime, cost, budget)?;
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
        placement: plan.resource.placement,
    })
}

fn estimate_lens_cost(spec: &LensSpec) -> CliResult<LensCost> {
    match &spec.runtime {
        LensRuntime::Algorithmic { .. }
        | LensRuntime::ExternalCmd { .. }
        | LensRuntime::TeiHttp { .. } => Ok(LensCost::zero()),
        LensRuntime::MultimodalAdapter { files, .. } => {
            let bytes = files_size(files)?;
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
    Ok(PlacementBudget {
        vram_soft_cap_bytes: env_u64("CALYX_PANEL_VRAM_SOFT_CAP_BYTES", 32 * gib())?,
        tei_reserved_bytes: env_u64("CALYX_TEI_RESERVED_BYTES", 20 * gib())?,
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
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn catalog_replacement_removes_prior_name_manifest_or_id() {
        let mut catalog = LensCatalog {
            lenses: vec![
                entry("same-id", "old", "old.json"),
                entry("other-id", "same-name", "other.json"),
                entry("third-id", "third", "same-path.json"),
                entry("keep-id", "keep", "keep.json"),
            ],
        };

        retain_unrelated_entries(
            &mut catalog,
            "same-id",
            "same-name",
            Path::new("same-path.json"),
        );

        assert_eq!(catalog.lenses.len(), 1);
        assert_eq!(catalog.lenses[0].lens_id, "keep-id");
    }

    #[test]
    fn multimodal_adapter_cost_counts_files_as_cpu_ram() {
        let root = temp_root("multimodal-cost");
        let model = root.join("model.onnx");
        let adapter = root.join("adapter.json");
        fs::write(&model, [1_u8; 11]).unwrap();
        fs::write(&adapter, [2_u8; 7]).unwrap();
        let spec = LensSpec {
            name: "fixture-image-adapter".to_string(),
            runtime: LensRuntime::MultimodalAdapter {
                axis: "image".to_string(),
                model_id: "fixture/model".to_string(),
                adapter_config: Some(adapter.clone()),
                files: vec![model, adapter],
            },
            output: calyx_core::SlotShape::Dense(16),
            modality: calyx_core::Modality::Image,
            weights_sha256: [1_u8; 32],
            corpus_hash: [2_u8; 32],
            norm_policy: calyx_registry::NormPolicy::unit(),
            max_batch: None,
            axis: Some("image:fixture/model".to_string()),
            asymmetry: calyx_core::Asymmetry::None,
            quant_default: calyx_core::QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: 0.02,
            retrieval_only: false,
            excluded_from_dedup: false,
        };

        let cost = estimate_lens_cost(&spec).unwrap();

        assert_eq!(cost.vram_bytes, 0);
        assert_eq!(cost.ram_bytes, 18);
        assert_eq!(cost.batch_ceiling, u32::MAX);
        let _ = fs::remove_dir_all(root);
    }

    fn entry(lens_id: &str, name: &str, manifest: &str) -> LensCatalogEntry {
        LensCatalogEntry {
            lens_id: lens_id.to_string(),
            name: name.to_string(),
            modality: "text".to_string(),
            runtime: "onnx_colbert".to_string(),
            dim: 384,
            retrieval_only: false,
            excluded_from_dedup: false,
            weights_sha256: "00".repeat(32),
            manifest: PathBuf::from(manifest),
            cost: LensCost::zero(),
            placement: Placement::Cpu,
        }
    }

    #[test]
    fn list_health_uses_metadata_without_reading_missing_artifact() {
        let root = temp_root("list-health-metadata");
        let manifest = root.join("manifest.json");
        fs::write(
            &manifest,
            r#"{
  "name": "missing-artifact",
  "modality": "text",
  "runtime": "onnx-int8",
  "dim": 384,
  "shape": {"kind": "dense", "dim": 384},
  "dtype": "int8",
  "weights_sha256": "1111111111111111111111111111111111111111111111111111111111111111",
  "artifact_set_sha256": null,
  "files": [
    {"role": "model", "path": "missing.onnx", "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "bytes": 123}
  ],
  "pooling": "mean",
  "norm": "unit",
  "source_hf_id": "fixture/missing",
  "license": "apache-2.0",
  "non_commercial": false,
  "quant_default": {"turbo_quant": {"bits_per_channel_x2": 7}},
  "truncate_dim": null,
  "recall_delta": 0.02
}"#,
        )
        .unwrap();

        assert_eq!(health_from_manifest(&manifest), LensHealth::Cold);
        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "calyx-catalog-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
