use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use calyx_core::{Input, Lens, Placement, SlotShape, SlotVector};
use calyx_registry::{LensSpec as RegistryLensSpec, lens_spec_from_manifest_path};
use serde::{Deserialize, Serialize};

use super::data::BuildRows;
use super::request::CorpusBuildRequest;
use crate::assay_bits_validation::cost::LensCost;
use crate::lens_commands::support::{dim, validate_vector_contract};
mod algorithmic;
mod progress;
pub(crate) mod projection;
mod runtime;
mod stream;
use projection::{project_multi, project_sparse};
use runtime::build_lens;
pub(crate) use stream::measure_text_batch;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct MeasuredLens {
    pub(crate) name: String,
    pub(crate) manifest: PathBuf,
    pub(crate) runtime: String,
    pub(crate) output: SlotShape,
    pub(crate) max_batch: Option<usize>,
    pub(crate) assay_projection: String,
    pub(crate) vectors: Vec<Vec<f32>>,
    pub(crate) cost: LensCost,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worker_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worker_report_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worker_stderr_path: Option<PathBuf>,
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
    load_unique_specs(request)?
        .into_iter()
        .map(|(manifest, spec)| build_lens(manifest, spec))
        .collect()
}

pub(crate) fn measure_requested_lenses(
    request: &CorpusBuildRequest,
    rows: &BuildRows,
) -> Result<Vec<MeasuredLens>, String> {
    let specs = load_unique_specs(request)?;
    let overrides = load_cost_overrides(request)?;
    let mut measured = Vec::with_capacity(specs.len());
    for (manifest, spec) in specs {
        let lens = build_lens(manifest, spec)?;
        measured.push(measure_one_lens(request, rows, lens, &overrides)?);
    }
    Ok(measured)
}

fn load_unique_specs(
    request: &CorpusBuildRequest,
) -> Result<Vec<(PathBuf, RegistryLensSpec)>, String> {
    let mut names = BTreeMap::new();
    let mut specs = Vec::with_capacity(request.manifests.len());
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
        specs.push((manifest.clone(), spec));
    }
    Ok(specs)
}

fn measure_one_lens(
    request: &CorpusBuildRequest,
    rows: &BuildRows,
    lens: BuildLens,
    overrides: &BTreeMap<String, CostOverride>,
) -> Result<MeasuredLens, String> {
    let inputs = rows
        .rows
        .iter()
        .map(|row| row.input_for(lens.spec.modality))
        .collect::<Vec<_>>();
    progress::emit_start(request, rows, &lens);
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
    let (vectors, assay_projection) = assay_vectors(&lens, slots)?;
    let cost = lens_cost(&lens, overrides.get(&lens.name).copied(), ms_per_input)?;
    progress::emit_finish(request, rows, &lens, total_ms, &cost);
    Ok(MeasuredLens {
        name: lens.name,
        manifest: lens.manifest,
        runtime: lens.runtime_name,
        output: lens.spec.output,
        max_batch: lens.spec.max_batch,
        assay_projection: assay_projection.to_string(),
        vectors,
        cost,
        worker_pid: None,
        worker_report_path: None,
        worker_stderr_path: None,
    })
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

fn assay_vectors(
    lens: &BuildLens,
    slots: Vec<SlotVector>,
) -> Result<(Vec<Vec<f32>>, &'static str), String> {
    let mut vectors = Vec::with_capacity(slots.len());
    let mut projection = None;
    for (idx, slot) in slots.into_iter().enumerate() {
        match slot {
            SlotVector::Dense { dim: got, data } if got == dim(lens.spec.output) => {
                projection.get_or_insert("native_dense");
                vectors.push(data);
            }
            SlotVector::Dense { dim: got, data: _ } => {
                return Err(format!(
                    "CALYX_FSV_ASSAY_CORPUS_BUILD_DIM_MISMATCH: lens={} row={idx} dim={got} expected={}",
                    lens.name,
                    dim(lens.spec.output)
                ));
            }
            SlotVector::Sparse { dim: got, entries } if got == dim(lens.spec.output) => {
                projection.get_or_insert(projection::slot_projection_name(lens.spec.output));
                vectors.push(project_sparse(&lens.name, idx, got, entries)?);
            }
            SlotVector::Sparse {
                dim: got,
                entries: _,
            } => {
                return Err(format!(
                    "CALYX_FSV_ASSAY_CORPUS_BUILD_DIM_MISMATCH: lens={} row={idx} dim={got} expected={}",
                    lens.name,
                    dim(lens.spec.output)
                ));
            }
            SlotVector::Multi { token_dim, tokens } if token_dim == dim(lens.spec.output) => {
                projection.get_or_insert("multi_mean_dense");
                vectors.push(project_multi(&lens.name, idx, token_dim, tokens)?);
            }
            SlotVector::Multi {
                token_dim,
                tokens: _,
            } => {
                return Err(format!(
                    "CALYX_FSV_ASSAY_CORPUS_BUILD_DIM_MISMATCH: lens={} row={idx} token_dim={token_dim} expected={}",
                    lens.name,
                    dim(lens.spec.output)
                ));
            }
            other => {
                return Err(format!(
                    "CALYX_FSV_ASSAY_CORPUS_BUILD_UNSUPPORTED_VECTOR_SHAPE: lens={} row={idx} shape {:?} must be dense, sparse, or multi",
                    lens.name, other
                ));
            }
        }
    }
    Ok((vectors, projection.unwrap_or("native_dense")))
}

fn lens_cost(
    lens: &BuildLens,
    override_cost: Option<CostOverride>,
    ms_per_input: f32,
) -> Result<LensCost, String> {
    let basis = match override_cost {
        Some(cost) => {
            validate_override_compatible(&lens.name, &lens.runtime_name, lens.placement, cost)?;
            RuntimeCostBasis {
                name: &lens.name,
                runtime: &lens.runtime_name,
                placement: cost.placement,
                vram_mb: cost.vram_mb,
                ram_mb: cost.ram_mb,
            }
        }
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

fn validate_override_compatible(
    lens_name: &str,
    runtime_name: &str,
    default_placement: Placement,
    cost: CostOverride,
) -> Result<(), String> {
    if default_placement == Placement::Gpu && cost.placement != Placement::Gpu {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_GPU_OVERRIDE_PLACEMENT: lens={lens_name} runtime={runtime_name} override placement={:?} but GPU runtimes must remain gpu",
            cost.placement
        ));
    }
    Ok(())
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

fn lens_error(error: calyx_core::CalyxError) -> String {
    format!("{}: {}", error.code, error.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_runtime_override_cannot_downgrade_to_cpu() {
        let cost = CostOverride {
            placement: Placement::Cpu,
            vram_mb: 0.0,
            ram_mb: 128.0,
        };

        let error = validate_override_compatible("semantic-bge", "onnx", Placement::Gpu, cost)
            .expect_err("GPU runtime override must not become CPU");

        assert!(error.contains("CALYX_FSV_ASSAY_CORPUS_BUILD_GPU_OVERRIDE_PLACEMENT"));
        assert!(error.contains("runtime=onnx"));
    }

    #[test]
    fn cpu_runtime_override_can_remain_cpu() {
        let cost = CostOverride {
            placement: Placement::Cpu,
            vram_mb: 0.0,
            ram_mb: 128.0,
        };

        validate_override_compatible("semantic-potion", "static_lookup", Placement::Cpu, cost)
            .expect("CPU-native override can remain CPU");
    }
}
