use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::{Input, Lens, SlotShape, SlotVector};
use calyx_registry::{
    CandleLens, FastembedBgem3Lens, FastembedRerankerLens, FastembedSparseLens, LensRuntime,
    LensSpec, MultimodalAdapterLens, OnnxColbertLens, OnnxLens, StaticLookupLens, TeiHttpLens,
    lens_spec_from_manifest_path,
};
use serde::Serialize;

use super::flags::Flags;
use super::support::{dim, runtime_name, slot_norm, slot_prefix, validate_vector_contract};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Serialize)]
struct ExplainReport {
    manifest: PathBuf,
    lens_id: String,
    name: String,
    runtime: String,
    runtime_detail: String,
    dtype: String,
    shape: ShapeReport,
    dim: u32,
    retrieval_only: bool,
    excluded_from_dedup: bool,
    rows: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_count: Option<usize>,
    norm: f32,
    norm_ok: bool,
    first_values: Vec<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_vector: Option<Vec<f32>>,
    total_ms: f32,
    ms_per_input: f32,
    vram_bytes: u64,
    vram_mb: f32,
}

#[derive(Serialize)]
struct ShapeReport {
    kind: &'static str,
    dim: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_dim: Option<u32>,
}

struct Measurement {
    vector: SlotVector,
    dtype: String,
    rows: Option<u32>,
    vram_bytes: u64,
    runtime_detail: String,
}

pub(crate) fn explain(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    let manifest = flags
        .manifest
        .clone()
        .ok_or_else(|| CliError::usage("calyx lens explain requires --manifest <path>"))?;
    let repeat = flags.repeat.unwrap_or(1);
    if repeat == 0 {
        return Err(CliError::usage("--repeat must be > 0"));
    }
    let spec = lens_spec_from_manifest_path(&manifest)?;
    let input = input_bytes(&flags)?;
    let probe = Input::new(spec.modality, input);
    let started = Instant::now();
    let measurement = measure_runtime(&spec, &probe, repeat)?;
    let total_ms = started.elapsed().as_secs_f64() as f32 * 1000.0;
    validate_vector_contract(&measurement.vector, spec.output, spec.norm_policy)?;
    let norm = slot_norm(&measurement.vector);
    print_json(&ExplainReport {
        manifest,
        lens_id: spec.lens_id().to_string(),
        name: spec.name,
        runtime: runtime_name(&spec.runtime).to_string(),
        runtime_detail: measurement.runtime_detail,
        dtype: measurement.dtype,
        shape: shape_report(spec.output),
        dim: dim(spec.output),
        retrieval_only: spec.retrieval_only,
        excluded_from_dedup: spec.excluded_from_dedup,
        rows: measurement.rows,
        token_count: token_count(&measurement.vector),
        norm,
        norm_ok: true,
        first_values: slot_prefix(&measurement.vector, 4),
        full_vector: full_vector(&measurement.vector, flags.full_vector)?,
        total_ms,
        ms_per_input: total_ms / repeat as f32,
        vram_bytes: measurement.vram_bytes,
        vram_mb: measurement.vram_bytes as f32 / (1024.0 * 1024.0),
    })
}

fn shape_report(shape: SlotShape) -> ShapeReport {
    match shape {
        SlotShape::Dense(dim) => ShapeReport {
            kind: "dense",
            dim,
            token_dim: None,
        },
        SlotShape::Sparse(dim) => ShapeReport {
            kind: "sparse",
            dim,
            token_dim: None,
        },
        SlotShape::Multi { token_dim } => ShapeReport {
            kind: "multi",
            dim: token_dim,
            token_dim: Some(token_dim),
        },
    }
}

fn token_count(vector: &SlotVector) -> Option<usize> {
    match vector {
        SlotVector::Multi { tokens, .. } => Some(tokens.len()),
        _ => None,
    }
}

fn measure_runtime(spec: &LensSpec, probe: &Input, repeat: usize) -> CliResult<Measurement> {
    match &spec.runtime {
        LensRuntime::StaticLookup { .. } => measure_static_lookup(spec, probe, repeat),
        LensRuntime::TeiHttp { endpoint } => measure_tei(spec, endpoint, probe, repeat),
        LensRuntime::CandleLocal { .. } => measure_candle(spec, probe, repeat),
        LensRuntime::Onnx { .. } => measure_onnx(spec, probe, repeat),
        LensRuntime::OnnxColbert { .. } => measure_onnx_colbert(spec, probe, repeat),
        LensRuntime::FastembedSparse { .. } => measure_fastembed_sparse(spec, probe, repeat),
        LensRuntime::FastembedBgem3 { .. } => measure_fastembed_bgem3(spec, probe, repeat),
        LensRuntime::FastembedReranker { .. } => measure_fastembed_reranker(spec, probe, repeat),
        LensRuntime::FastembedQwen3 { .. } => measure_fastembed_qwen3(spec, probe, repeat),
        LensRuntime::MultimodalAdapter { .. } => measure_multimodal(spec, probe, repeat),
        other => Err(CliError::usage(format!(
            "calyx lens explain does not support {} runtime measurement",
            runtime_name(other)
        ))),
    }
}

fn full_vector(vector: &SlotVector, enabled: bool) -> CliResult<Option<Vec<f32>>> {
    if !enabled {
        return Ok(None);
    }
    match vector {
        SlotVector::Dense { data, .. } => Ok(Some(data.clone())),
        SlotVector::Sparse { .. } | SlotVector::Multi { .. } | SlotVector::Absent { .. } => Err(
            CliError::usage("--full-vector is supported only for dense lens explain output"),
        ),
    }
}

fn input_bytes(flags: &Flags) -> CliResult<Vec<u8>> {
    match (&flags.input, &flags.input_file) {
        (Some(_), Some(_)) => Err(CliError::usage(
            "calyx lens explain accepts only one of --input or --input-file",
        )),
        (Some(input), None) => Ok(input.clone().into_bytes()),
        (None, Some(path)) => Ok(fs::read(path)?),
        (None, None) => Ok(b"Calyx lens explain probe".to_vec()),
    }
}

fn measure_static_lookup(spec: &LensSpec, probe: &Input, repeat: usize) -> CliResult<Measurement> {
    let lens = StaticLookupLens::from_lens_spec(spec)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: lens.dtype().as_str().to_string(),
        rows: Some(lens.row_count()),
        vram_bytes: 0,
        runtime_detail: "static_lookup_mmap".to_string(),
    })
}

fn measure_tei(
    spec: &LensSpec,
    endpoint: &str,
    probe: &Input,
    repeat: usize,
) -> CliResult<Measurement> {
    let lens = TeiHttpLens::new(&spec.name, endpoint, spec.modality, dim(spec.output));
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: "f32".to_string(),
        rows: None,
        vram_bytes: 0,
        runtime_detail: endpoint.to_string(),
    })
}

fn measure_candle(spec: &LensSpec, probe: &Input, repeat: usize) -> CliResult<Measurement> {
    let lens = CandleLens::from_lens_spec(spec)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: lens.precision().as_str().to_string(),
        rows: None,
        vram_bytes: files_size(&lens.files().artifact_paths())?,
        runtime_detail: lens.device_policy().as_str().to_string(),
    })
}

fn measure_onnx(spec: &LensSpec, probe: &Input, repeat: usize) -> CliResult<Measurement> {
    let lens = OnnxLens::from_lens_spec(spec)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: "f32".to_string(),
        rows: None,
        vram_bytes: files_size(&lens.files().artifact_paths())?,
        runtime_detail: format!("{};{}", lens.runtime_name(), lens.provider_policy()),
    })
}

fn measure_onnx_colbert(spec: &LensSpec, probe: &Input, repeat: usize) -> CliResult<Measurement> {
    let lens = OnnxColbertLens::from_lens_spec(spec)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: "f32".to_string(),
        rows: None,
        vram_bytes: files_size(&lens.files().artifact_paths())?,
        runtime_detail: format!("onnx-colbert;{}", lens.provider_policy()),
    })
}

fn measure_fastembed_sparse(
    spec: &LensSpec,
    probe: &Input,
    repeat: usize,
) -> CliResult<Measurement> {
    let lens = FastembedSparseLens::from_lens_spec(spec)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: "f32".to_string(),
        rows: None,
        vram_bytes: files_size(&lens.files().artifact_paths())?,
        runtime_detail: format!("fastembed-sparse;{}", lens.provider_policy()),
    })
}

fn measure_fastembed_bgem3(
    spec: &LensSpec,
    probe: &Input,
    repeat: usize,
) -> CliResult<Measurement> {
    let lens = FastembedBgem3Lens::from_lens_spec(spec)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: "f32".to_string(),
        rows: None,
        vram_bytes: files_size(&lens.files().artifact_paths())?,
        runtime_detail: format!("{};{}", lens.runtime_name(), lens.provider_policy()),
    })
}

fn measure_fastembed_reranker(
    spec: &LensSpec,
    probe: &Input,
    repeat: usize,
) -> CliResult<Measurement> {
    let lens = FastembedRerankerLens::from_lens_spec(spec)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: "f32".to_string(),
        rows: None,
        vram_bytes: files_size(&lens.files().artifact_paths())?,
        runtime_detail: format!("fastembed-reranker;{}", lens.provider_policy()),
    })
}

fn measure_fastembed_qwen3(
    spec: &LensSpec,
    probe: &Input,
    repeat: usize,
) -> CliResult<Measurement> {
    let lens = calyx_registry::FastembedQwen3Lens::from_lens_spec(spec)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: lens.precision().as_str().to_string(),
        rows: None,
        vram_bytes: files_size(&lens.files().artifact_paths())?,
        runtime_detail: format!(
            "fastembed-qwen3;{};max_tokens={}",
            lens.device_policy().as_str(),
            lens.max_tokens()
        ),
    })
}

fn measure_multimodal(spec: &LensSpec, probe: &Input, repeat: usize) -> CliResult<Measurement> {
    let lens = MultimodalAdapterLens::from_lens_spec(spec)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    let vram_bytes = match &spec.runtime {
        LensRuntime::MultimodalAdapter { files, .. } if lens.provider().is_gpu() => {
            files_size(files)?
        }
        _ => 0,
    };
    Ok(Measurement {
        vector,
        dtype: "f32".to_string(),
        rows: None,
        vram_bytes,
        runtime_detail: format!(
            "multimodal_adapter_onnx_external;{}",
            lens.provider_detail()
        ),
    })
}

fn measure_repeated(lens: &dyn Lens, probe: &Input, repeat: usize) -> CliResult<SlotVector> {
    let mut last = None;
    for _ in 0..repeat {
        last = Some(lens.measure(probe)?);
    }
    last.ok_or_else(|| CliError::usage("repeat produced no vector"))
}

fn files_size(files: &[PathBuf]) -> CliResult<u64> {
    files
        .iter()
        .try_fold(0_u64, |acc, path| Ok(acc.saturating_add(path_size(path)?)))
}

fn path_size(path: &Path) -> CliResult<u64> {
    Ok(fs::metadata(path)?.len())
}
