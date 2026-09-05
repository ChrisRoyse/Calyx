use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::{Input, Lens, SlotShape, SlotVector};
use calyx_registry::{
    CandleLens, FastembedBgem3Lens, FastembedRerankerLens, FastembedSparseLens, LensRuntime,
    LensSpec, MultimodalAdapterLens, OnnxColbertLens, OnnxLens, StaticLookupLens,
    TeiEndpointIdentity, TeiHttpLens, TeiSegmentedLens, algorithmic_lens_from_spec,
    lens_spec_from_manifest_path, tei_endpoint_identity,
};
use serde::Serialize;

use super::explain_vector::{
    SegmentationReport, SparseEntryReport, full_sparse, full_vector, norm, segment_count,
    segment_norms, segmentation_report, sparse_entry_count, sparse_top, token_count, vector_kind,
    vector_sha256,
};
use super::flags::Flags;
use super::support::{dim, runtime_name, slot_prefix, validate_vector_contract};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Serialize)]
struct ExplainReport {
    manifest: PathBuf,
    lens_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_lens_id: Option<String>,
    name: String,
    runtime: String,
    runtime_detail: String,
    vector_kind: &'static str,
    dtype: String,
    shape: ShapeReport,
    dim: u32,
    retrieval_only: bool,
    excluded_from_dedup: bool,
    rows: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    segment_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    segmentation: Option<SegmentationReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    norm: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    segment_norms: Option<Vec<f32>>,
    norm_ok: bool,
    vector_sha256: String,
    first_values: Vec<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sparse_entries: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sparse_top: Option<Vec<SparseEntryReport>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_vector: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_sparse: Option<Vec<SparseEntryReport>>,
    total_ms: f32,
    ms_per_input: f32,
    vram_bytes: u64,
    vram_mb: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tei_endpoint_identity: Option<TeiEndpointIdentity>,
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
    let (runtime_lens_id, tei_endpoint_identity) = match &spec.runtime {
        LensRuntime::TeiHttp { endpoint } => {
            let lens = TeiHttpLens::new(&spec.name, endpoint, spec.modality, dim(spec.output));
            (
                Some(lens.id().to_string()),
                Some(tei_endpoint_identity(endpoint)?),
            )
        }
        LensRuntime::TeiHttpSegmented { endpoint, policy } => {
            let lens = TeiSegmentedLens::new(
                &spec.name,
                endpoint,
                spec.modality,
                dim(spec.output),
                *policy,
            )?;
            (
                Some(lens.id().to_string()),
                Some(tei_endpoint_identity(endpoint)?),
            )
        }
        LensRuntime::StaticLookup { .. } => (
            Some(StaticLookupLens::from_lens_spec(&spec)?.id().to_string()),
            None,
        ),
        LensRuntime::Algorithmic { .. } => (
            Some(algorithmic_lens_from_spec(&spec)?.id().to_string()),
            None,
        ),
        _ => (None, None),
    };
    let started = Instant::now();
    let measurement = measure_runtime(&spec, &probe, repeat)?;
    let total_ms = started.elapsed().as_secs_f64() as f32 * 1000.0;
    validate_vector_contract(&measurement.vector, spec.output, spec.norm_policy)?;
    print_json(&ExplainReport {
        manifest,
        lens_id: spec.lens_id().to_string(),
        runtime_lens_id,
        name: spec.name,
        runtime: runtime_name(&spec.runtime).to_string(),
        runtime_detail: measurement.runtime_detail,
        vector_kind: vector_kind(&measurement.vector),
        dtype: measurement.dtype,
        shape: shape_report(spec.output),
        dim: dim(spec.output),
        retrieval_only: spec.retrieval_only,
        excluded_from_dedup: spec.excluded_from_dedup,
        rows: measurement.rows,
        token_count: token_count(&measurement.vector),
        segment_count: segment_count(&measurement.vector),
        segmentation: segmentation_report(&measurement.vector),
        norm: norm(&measurement.vector),
        segment_norms: segment_norms(&measurement.vector),
        norm_ok: true,
        vector_sha256: vector_sha256(&measurement.vector),
        first_values: slot_prefix(&measurement.vector, 4),
        sparse_entries: sparse_entry_count(&measurement.vector),
        sparse_top: sparse_top(&measurement.vector, 8),
        full_vector: full_vector(&measurement.vector, flags.full_vector)?,
        full_sparse: full_sparse(&measurement.vector, flags.full_vector)?,
        total_ms,
        ms_per_input: total_ms / repeat as f32,
        vram_bytes: measurement.vram_bytes,
        vram_mb: measurement.vram_bytes as f32 / (1024.0 * 1024.0),
        tei_endpoint_identity,
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

fn measure_runtime(spec: &LensSpec, probe: &Input, repeat: usize) -> CliResult<Measurement> {
    match &spec.runtime {
        LensRuntime::Algorithmic { .. } => measure_algorithmic(spec, probe, repeat),
        LensRuntime::StaticLookup { .. } => measure_static_lookup(spec, probe, repeat),
        LensRuntime::TeiHttp { endpoint } => measure_tei(spec, endpoint, probe, repeat),
        LensRuntime::TeiHttpSegmented { endpoint, policy } => {
            measure_tei_segmented(spec, endpoint, *policy, probe, repeat)
        }
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

fn measure_algorithmic(spec: &LensSpec, probe: &Input, repeat: usize) -> CliResult<Measurement> {
    let lens = algorithmic_lens_from_spec(spec)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    let LensRuntime::Algorithmic { kind } = &spec.runtime else {
        unreachable!();
    };
    Ok(Measurement {
        vector,
        dtype: "f32".to_string(),
        rows: None,
        vram_bytes: 0,
        runtime_detail: format!("algorithmic;{kind};frozen={}", lens.id()),
    })
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
    let lens = TeiHttpLens::new(&spec.name, endpoint, spec.modality, dim(spec.output))
        .with_declared_max_batch(spec.max_batch)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: "f32".to_string(),
        rows: None,
        vram_bytes: 0,
        runtime_detail: endpoint.to_string(),
    })
}

fn measure_tei_segmented(
    spec: &LensSpec,
    endpoint: &str,
    policy: calyx_registry::TeiSegmentationPolicy,
    probe: &Input,
    repeat: usize,
) -> CliResult<Measurement> {
    let lens = TeiSegmentedLens::new(
        &spec.name,
        endpoint,
        spec.modality,
        dim(spec.output),
        policy,
    )?
    .with_declared_max_batch(spec.max_batch)?;
    let vector = measure_repeated(&lens, probe, repeat)?;
    Ok(Measurement {
        vector,
        dtype: "f32".to_string(),
        rows: None,
        vram_bytes: 0,
        runtime_detail: format!("{endpoint};{}", policy.fingerprint()),
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
    let expected = lens.measure(probe)?;
    let expected_sha256 = vector_sha256(&expected);
    for attempt in 2..=repeat {
        let observed = lens.measure(probe)?;
        if observed != expected {
            return Err(calyx_core::CalyxError::lens_frozen_violation(format!(
                "lens {} returned non-deterministic typed slot bytes for identical input at \
                 repeat {attempt}: expected_sha256={expected_sha256} observed_sha256={}",
                lens.id(),
                vector_sha256(&observed)
            ))
            .into());
        }
    }
    Ok(expected)
}

fn files_size(files: &[PathBuf]) -> CliResult<u64> {
    files
        .iter()
        .try_fold(0_u64, |acc, path| Ok(acc.saturating_add(path_size(path)?)))
}

fn path_size(path: &Path) -> CliResult<u64> {
    Ok(fs::metadata(path)?.len())
}
