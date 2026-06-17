use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::{Input, Lens, SlotVector};
use calyx_registry::{
    CandleLens, LensRuntime, LensSpec, OnnxLens, StaticLookupLens, TeiHttpLens,
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
    dim: u32,
    rows: Option<u32>,
    norm: f32,
    norm_ok: bool,
    first_values: Vec<f32>,
    total_ms: f32,
    ms_per_input: f32,
    vram_bytes: u64,
    vram_mb: f32,
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
        .ok_or_else(|| CliError::usage("calyx lens explain requires --manifest <path>"))?;
    let repeat = flags.repeat.unwrap_or(1);
    if repeat == 0 {
        return Err(CliError::usage("--repeat must be > 0"));
    }
    let spec = lens_spec_from_manifest_path(&manifest)?;
    let input = flags
        .input
        .unwrap_or_else(|| "Calyx lens explain probe".to_string());
    let probe = Input::new(spec.modality, input.into_bytes());
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
        dim: dim(spec.output),
        rows: measurement.rows,
        norm,
        norm_ok: true,
        first_values: slot_prefix(&measurement.vector, 4),
        total_ms,
        ms_per_input: total_ms / repeat as f32,
        vram_bytes: measurement.vram_bytes,
        vram_mb: measurement.vram_bytes as f32 / (1024.0 * 1024.0),
    })
}

fn measure_runtime(spec: &LensSpec, probe: &Input, repeat: usize) -> CliResult<Measurement> {
    match &spec.runtime {
        LensRuntime::StaticLookup { .. } => measure_static_lookup(spec, probe, repeat),
        LensRuntime::TeiHttp { endpoint } => measure_tei(spec, endpoint, probe, repeat),
        LensRuntime::CandleLocal { .. } => measure_candle(spec, probe, repeat),
        LensRuntime::Onnx { .. } => measure_onnx(spec, probe, repeat),
        other => Err(CliError::usage(format!(
            "calyx lens explain does not support {} runtime measurement",
            runtime_name(other)
        ))),
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
