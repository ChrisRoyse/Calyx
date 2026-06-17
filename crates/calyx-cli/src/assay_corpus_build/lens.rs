use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::{Input, Lens, Placement, SlotVector};
use calyx_registry::{
    CandleLens, LensRuntime, LensSpec as RegistryLensSpec, OnnxLens, StaticLookupLens, TeiHttpLens,
    lens_spec_from_manifest_path,
};
use serde::{Deserialize, Serialize};

use crate::assay_bits_validation::cost::LensCost;
use crate::lens_commands::support::{dim, runtime_name, validate_vector_contract};

use super::data::BuildRows;
use super::request::CorpusBuildRequest;

#[derive(Clone, Debug)]
pub(crate) struct MeasuredLens {
    pub(crate) name: String,
    pub(crate) manifest: PathBuf,
    pub(crate) runtime: String,
    pub(crate) dim: u32,
    pub(crate) vectors: Vec<Vec<f32>>,
    pub(crate) cost: LensCost,
}

pub(crate) struct BuildLens {
    name: String,
    manifest: PathBuf,
    spec: RegistryLensSpec,
    runtime_name: String,
    lens: Box<dyn Lens>,
    placement: Placement,
    default_vram_mb: f32,
    default_ram_mb: f32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct CostOverride {
    placement: Placement,
    vram_mb: f32,
    #[serde(default)]
    ram_mb: f32,
}

#[derive(Clone, Debug, Serialize)]
struct RuntimeCostBasis<'a> {
    name: &'a str,
    runtime: &'a str,
    placement: Placement,
    vram_mb: f32,
    ram_mb: f32,
}

pub(crate) fn load_lenses(request: &CorpusBuildRequest) -> Result<Vec<BuildLens>, String> {
    let mut names = BTreeMap::new();
    let mut lenses = Vec::with_capacity(request.manifests.len());
    for manifest in &request.manifests {
        let spec = lens_spec_from_manifest_path(manifest).map_err(|error| {
            format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_MANIFEST_INVALID: {}: {}",
                manifest.display(),
                error.message
            )
        })?;
        if let Some(previous) = names.insert(spec.name.clone(), manifest.clone()) {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_DUPLICATE_LENS: lens={} manifests={} and {}",
                spec.name,
                previous.display(),
                manifest.display()
            ));
        }
        lenses.push(build_lens(manifest.clone(), spec)?);
    }
    Ok(lenses)
}

pub(crate) fn measure_lenses(
    request: &CorpusBuildRequest,
    rows: &BuildRows,
    lenses: Vec<BuildLens>,
) -> Result<Vec<MeasuredLens>, String> {
    let overrides = load_cost_overrides(request)?;
    let inputs: Vec<Input> = rows
        .rows
        .iter()
        .map(|row| Input::new(calyx_core::Modality::Text, row.text.as_bytes().to_vec()))
        .collect();
    let mut measured = Vec::with_capacity(lenses.len());
    for lens in lenses {
        let started = Instant::now();
        let slots = measure_batches(&lens, &inputs, request.batch_size)?;
        let total_ms = started.elapsed().as_secs_f64() as f32 * 1000.0;
        let ms_per_input = total_ms / inputs.len() as f32;
        if !ms_per_input.is_finite() || ms_per_input <= 0.0 {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_COST: lens={} measured ms_per_input={} must be finite and > 0",
                lens.name, ms_per_input
            ));
        }
        let vectors = dense_vectors(&lens, slots)?;
        let cost = lens_cost(&lens, overrides.get(&lens.name).copied(), ms_per_input)?;
        measured.push(MeasuredLens {
            name: lens.name,
            manifest: lens.manifest,
            runtime: lens.runtime_name,
            dim: dim(lens.spec.output),
            vectors,
            cost,
        });
    }
    Ok(measured)
}

fn build_lens(manifest: PathBuf, spec: RegistryLensSpec) -> Result<BuildLens, String> {
    let runtime = runtime_name(&spec.runtime).to_string();
    match spec.runtime.clone() {
        LensRuntime::Onnx { files, .. } => {
            let lens = OnnxLens::from_lens_spec(&spec).map_err(lens_error)?;
            let ram_mb = paths_mb(&files)?;
            Ok(BuildLens {
                name: spec.name.clone(),
                manifest,
                spec,
                runtime_name: runtime,
                lens: Box::new(lens),
                placement: Placement::Cpu,
                default_vram_mb: 0.0,
                default_ram_mb: ram_mb,
            })
        }
        LensRuntime::StaticLookup {
            embeddings_file,
            tokenizer,
            ..
        } => {
            let lens = StaticLookupLens::from_lens_spec(&spec).map_err(lens_error)?;
            let files = vec![embeddings_file.clone(), tokenizer.clone()];
            let ram_mb = paths_mb(&files)?;
            Ok(BuildLens {
                name: spec.name.clone(),
                manifest,
                spec,
                runtime_name: runtime,
                lens: Box::new(lens),
                placement: Placement::Cpu,
                default_vram_mb: 0.0,
                default_ram_mb: ram_mb,
            })
        }
        LensRuntime::TeiHttp { endpoint } => {
            let lens = TeiHttpLens::new(&spec.name, endpoint, spec.modality, dim(spec.output));
            Ok(BuildLens {
                name: spec.name.clone(),
                manifest,
                spec,
                runtime_name: runtime,
                lens: Box::new(lens),
                placement: Placement::Gpu,
                default_vram_mb: f32::NAN,
                default_ram_mb: 0.0,
            })
        }
        LensRuntime::CandleLocal { files, .. } => {
            let lens = CandleLens::from_lens_spec(&spec).map_err(lens_error)?;
            let vram_mb = paths_mb(&files)?;
            Ok(BuildLens {
                name: spec.name.clone(),
                manifest,
                spec,
                runtime_name: runtime,
                lens: Box::new(lens),
                placement: Placement::Gpu,
                default_vram_mb: vram_mb,
                default_ram_mb: 0.0,
            })
        }
        other => Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_UNSUPPORTED_RUNTIME: lens={} runtime={}",
            spec.name,
            runtime_name(&other)
        )),
    }
}

fn measure_batches(
    lens: &BuildLens,
    inputs: &[Input],
    batch_size: usize,
) -> Result<Vec<SlotVector>, String> {
    let mut slots = Vec::with_capacity(inputs.len());
    for batch in inputs.chunks(batch_size) {
        let mut rows = lens.lens.measure_batch(batch).map_err(lens_error)?;
        if rows.len() != batch.len() {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_VECTOR_COUNT_MISMATCH: lens={} returned {} vectors for {} inputs",
                lens.name,
                rows.len(),
                batch.len()
            ));
        }
        for vector in &rows {
            validate_vector_contract(vector, lens.spec.output, lens.spec.norm_policy)
                .map_err(|error| format!("{}: {}", error.code(), error.message()))?;
        }
        slots.append(&mut rows);
    }
    Ok(slots)
}

fn dense_vectors(lens: &BuildLens, slots: Vec<SlotVector>) -> Result<Vec<Vec<f32>>, String> {
    let mut vectors = Vec::with_capacity(slots.len());
    for (idx, slot) in slots.into_iter().enumerate() {
        match slot {
            SlotVector::Dense { dim: got, data } if got == dim(lens.spec.output) => {
                vectors.push(data);
            }
            SlotVector::Dense { dim: got, data: _ } => {
                return Err(format!(
                    "CALYX_FSV_ASSAY_CORPUS_BUILD_DIM_MISMATCH: lens={} row={idx} dim={got} expected={}",
                    lens.name,
                    dim(lens.spec.output)
                ));
            }
            _ => {
                return Err(format!(
                    "CALYX_FSV_ASSAY_CORPUS_BUILD_UNSUPPORTED_VECTOR_SHAPE: lens={} row={idx} shape must be dense",
                    lens.name
                ));
            }
        }
    }
    Ok(vectors)
}

fn lens_cost(
    lens: &BuildLens,
    override_cost: Option<CostOverride>,
    ms_per_input: f32,
) -> Result<LensCost, String> {
    let basis = match override_cost {
        Some(cost) => RuntimeCostBasis {
            name: &lens.name,
            runtime: &lens.runtime_name,
            placement: cost.placement,
            vram_mb: cost.vram_mb,
            ram_mb: cost.ram_mb,
        },
        None if lens.default_vram_mb.is_finite() => RuntimeCostBasis {
            name: &lens.name,
            runtime: &lens.runtime_name,
            placement: lens.placement,
            vram_mb: lens.default_vram_mb,
            ram_mb: lens.default_ram_mb,
        },
        None => {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_MISSING_COST_OVERRIDE: lens={} runtime={} requires --cost-override-json with measured resident resources",
                lens.name, lens.runtime_name
            ));
        }
    };
    validate_cost_basis(&basis)?;
    Ok(LensCost {
        placement: basis.placement,
        vram_mb: basis.vram_mb,
        ram_mb: basis.ram_mb,
        ms_per_input,
    })
}

fn load_cost_overrides(
    request: &CorpusBuildRequest,
) -> Result<BTreeMap<String, CostOverride>, String> {
    let Some(path) = &request.cost_override_json else {
        return Ok(BTreeMap::new());
    };
    let text = fs::read_to_string(path).map_err(|error| {
        format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_COST_OVERRIDE_IO: {}: {error}",
            path.display()
        )
    })?;
    let overrides: BTreeMap<String, CostOverride> =
        serde_json::from_str(&text).map_err(|error| {
            format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_COST_OVERRIDE: {}: {error}",
                path.display()
            )
        })?;
    for (name, cost) in &overrides {
        let basis = RuntimeCostBasis {
            name,
            runtime: "override",
            placement: cost.placement,
            vram_mb: cost.vram_mb,
            ram_mb: cost.ram_mb,
        };
        validate_cost_basis(&basis)?;
    }
    Ok(overrides)
}

fn validate_cost_basis(cost: &RuntimeCostBasis<'_>) -> Result<(), String> {
    if !cost.vram_mb.is_finite() || cost.vram_mb < 0.0 {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_COST: lens={} runtime={} vram_mb={} must be finite and >= 0",
            cost.name, cost.runtime, cost.vram_mb
        ));
    }
    if !cost.ram_mb.is_finite() || cost.ram_mb < 0.0 {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_COST: lens={} runtime={} ram_mb={} must be finite and >= 0",
            cost.name, cost.runtime, cost.ram_mb
        ));
    }
    Ok(())
}

fn paths_mb(paths: &[PathBuf]) -> Result<f32, String> {
    let bytes = paths.iter().try_fold(0_u64, |acc, path| {
        Ok::<u64, String>(acc.saturating_add(file_len(path)?))
    })?;
    Ok(bytes as f32 / (1024.0 * 1024.0))
}

fn file_len(path: &Path) -> Result<u64, String> {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| {
            format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_FILE_IO: {}: {error}",
                path.display()
            )
        })
}

fn lens_error(error: calyx_core::CalyxError) -> String {
    format!("{}: {}", error.code, error.message)
}
