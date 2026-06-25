use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{Input, Lens, Modality, QuantPolicy, SlotShape};
use calyx_registry::{
    DEFAULT_TEI_ENDPOINT, LensForgeManifest, LensForgeShape, NormPolicy, TeiHttpLens,
};
use serde::Serialize;
use serde_json::json;

mod artifact;
mod fastembed;
mod fastembed_special;
mod log;
mod onnx_colbert;
mod options;
mod tei;

use artifact::{
    Artifact, FileReport, add_optional, artifact, artifact_set_sha256, file_report, find_preferred,
    manifest_files, read_hidden_size, require_named, require_named_fallback,
};
use log::{ConversionLog, run_command, write_json_file};
use options::{CommissionFlags, CommissionRuntime};

use super::catalog::{AddReport, add_manifest_to_catalog};
use super::support::validate_vector_contract;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_TEI_DIM: u32 = 768;
const MANIFEST_NAME: &str = "lensforge.manifest.json";
const CONVERSION_LOG_NAME: &str = "conversion-log.jsonl";

#[derive(Serialize)]
struct CommissionReport {
    hf: String,
    runtime: String,
    output_dir: PathBuf,
    manifest: PathBuf,
    conversion_log: PathBuf,
    files: Vec<FileReport>,
    registered: AddReport,
}

struct CommissionOutput {
    artifacts: Vec<Artifact>,
    dim_override: Option<u32>,
    source_hf_id: Option<String>,
}

impl CommissionOutput {
    fn new(artifacts: Vec<Artifact>) -> Self {
        Self {
            artifacts,
            dim_override: None,
            source_hf_id: None,
        }
    }

    fn with_dim(artifacts: Vec<Artifact>, dim: u32) -> Self {
        Self {
            artifacts,
            dim_override: Some(dim),
            source_hf_id: None,
        }
    }

    fn with_source_hf_id(artifacts: Vec<Artifact>, source_hf_id: String) -> Self {
        Self {
            artifacts,
            dim_override: None,
            source_hf_id: Some(source_hf_id),
        }
    }
}

pub(crate) fn commission(args: &[String]) -> CliResult {
    let flags = CommissionFlags::parse(args)?;
    let out = flags.output_dir()?;
    fs::create_dir_all(&out)?;
    let mut log = ConversionLog::create(out.join(CONVERSION_LOG_NAME))?;
    log.event(json!({
        "event": "commission_start",
        "hf": flags.hf,
        "runtime": flags.runtime.manifest_runtime(),
        "output_dir": out,
    }))?;
    let output = match flags.runtime {
        CommissionRuntime::Tei => {
            let commissioned = commission_tei(&flags, &out, &mut log)?;
            CommissionOutput::with_source_hf_id(commissioned.artifacts, commissioned.source_hf_id)
        }
        CommissionRuntime::CandleFp16 => {
            CommissionOutput::new(commission_candle(&flags, &out, &mut log)?)
        }
        CommissionRuntime::OnnxInt8 => {
            CommissionOutput::new(commission_onnx_int8(&flags, &out, &mut log)?)
        }
        CommissionRuntime::OnnxFp32 => {
            CommissionOutput::new(commission_onnx_fp32(&flags, &out, &mut log)?)
        }
        CommissionRuntime::FastembedOnnx => {
            let commissioned = fastembed::commission(&flags, &out, &mut log)?;
            CommissionOutput::with_dim(commissioned.artifacts, commissioned.dim)
        }
        CommissionRuntime::OnnxColbert => {
            let commissioned = onnx_colbert::commission(&flags, &out, &mut log)?;
            CommissionOutput::with_dim(commissioned.artifacts, commissioned.dim)
        }
        CommissionRuntime::FastembedSparse
        | CommissionRuntime::FastembedBgem3Dense
        | CommissionRuntime::FastembedBgem3Sparse
        | CommissionRuntime::FastembedBgem3Colbert
        | CommissionRuntime::FastembedReranker
        | CommissionRuntime::FastembedQwen3 => {
            let commissioned = fastembed_special::commission(&flags, &out, &mut log)?;
            CommissionOutput::with_dim(commissioned.artifacts, commissioned.dim)
        }
    };
    let manifest_path = write_manifest(
        &flags,
        &out,
        &output.artifacts,
        output.dim_override,
        output.source_hf_id.as_deref(),
        &mut log,
    )?;
    let registered = add_manifest_to_catalog(flags.home.as_deref(), manifest_path.clone())?;
    log.event(json!({
        "event": "registered",
        "catalog": registered.catalog,
        "lens_id": registered.lens_id,
    }))?;
    print_json(&CommissionReport {
        hf: flags.hf,
        runtime: flags.runtime.manifest_runtime().to_string(),
        output_dir: out,
        manifest: manifest_path,
        conversion_log: log.path,
        files: output.artifacts.iter().map(file_report).collect(),
        registered,
    })
}

fn commission_tei(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<tei::CommissionedTei> {
    let endpoint = flags
        .endpoint
        .as_deref()
        .unwrap_or(DEFAULT_TEI_ENDPOINT)
        .to_string();
    let dim = flags.dim.unwrap_or(DEFAULT_TEI_DIM);
    let lens = TeiHttpLens::new(flags.lens_name(), &endpoint, Modality::Text, dim);
    let probe = Input::new(Modality::Text, b"Calyx TEI commission probe".to_vec());
    let vector = lens.measure(&probe)?;
    validate_vector_contract(&vector, SlotShape::Dense(dim), NormPolicy::unit())?;
    let commissioned = tei::write_descriptor(&flags.hf, endpoint, dim, out)?;
    log.event(json!({
        "event": "tei_probe_verified",
        "descriptor": commissioned.descriptor_path,
        "source_hf_id": commissioned.source_hf_id,
        "requested_hf_id": commissioned.requested_hf_id,
        "dim": dim,
    }))?;
    Ok(commissioned)
}

fn commission_candle(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<Vec<Artifact>> {
    let artifact_dir = out.join("hf-candle");
    fs::create_dir_all(&artifact_dir)?;
    run_command(
        log,
        "hf",
        &[
            "download",
            &flags.hf,
            "--local-dir",
            &artifact_dir.display().to_string(),
            "--include",
            "config.json",
            "--include",
            "tokenizer.json",
            "--include",
            "tokenizer_config.json",
            "--include",
            "special_tokens_map.json",
            "--include",
            "*.safetensors",
        ],
    )?;
    let weights = find_preferred(&artifact_dir, &["model.safetensors"], "safetensors")?;
    let tokenizer = require_named(&artifact_dir, "tokenizer.json")?;
    let config = require_named(&artifact_dir, "config.json")?;
    let dim = flags.dim.unwrap_or(read_hidden_size(&config)?);
    log.event(json!({"event": "candle_artifacts_ready", "dim": dim}))?;
    let mut artifacts = vec![
        artifact("model", weights)?,
        artifact("tokenizer", tokenizer)?,
        artifact("config", config)?,
    ];
    add_optional(
        &mut artifacts,
        "tokenizer_config",
        artifact_dir.join("tokenizer_config.json"),
    )?;
    add_optional(
        &mut artifacts,
        "special_tokens_map",
        artifact_dir.join("special_tokens_map.json"),
    )?;
    Ok(artifacts)
}

fn commission_onnx_int8(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<Vec<Artifact>> {
    let export_dir = export_onnx(flags, out, log)?;
    let quant_dir = out.join("onnx-int8");
    fs::create_dir_all(&quant_dir)?;
    let target_flag = format!("--{}", flags.quant_target);
    run_command(
        log,
        "optimum-cli",
        &[
            "onnxruntime",
            "quantize",
            "--onnx_model",
            &export_dir.display().to_string(),
            "-o",
            &quant_dir.display().to_string(),
            &target_flag,
        ],
    )?;
    let model = find_preferred(&quant_dir, &["model_quantized.onnx", "model.onnx"], "onnx")?;
    // See commission_onnx_fp32: register the external-data weights sidecar so it
    // is hashed and counted toward VRAM cost, never a silent multi-GB undercount.
    let model_data = model.with_extension("onnx_data");
    let tokenizer = require_named_fallback(&quant_dir, &export_dir, "tokenizer.json")?;
    let config = require_named_fallback(&quant_dir, &export_dir, "config.json")?;
    let dim = flags.dim.unwrap_or(read_hidden_size(&config)?);
    log.event(json!({"event": "onnx_int8_artifacts_ready", "dim": dim}))?;
    let mut artifacts = vec![
        artifact("model", model)?,
        artifact("tokenizer", tokenizer)?,
        artifact("config", config)?,
    ];
    add_optional(&mut artifacts, "model_data", model_data)?;
    add_optional(
        &mut artifacts,
        "tokenizer_config",
        export_dir.join("tokenizer_config.json"),
    )?;
    add_optional(
        &mut artifacts,
        "special_tokens_map",
        export_dir.join("special_tokens_map.json"),
    )?;
    Ok(artifacts)
}

fn commission_onnx_fp32(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<Vec<Artifact>> {
    let export_dir = export_onnx(flags, out, log)?;
    let model = find_preferred(&export_dir, &["model.onnx"], "onnx")?;
    // Large models export weights to an external-data sidecar (`model.onnx_data`)
    // that the .onnx graph references at load. It must be in the manifest so its
    // bytes are hashed/verified AND counted toward the lens VRAM cost — otherwise
    // a multi-GB model is admitted as a few-MB graph and over-commits the GPU.
    let model_data = model.with_extension("onnx_data");
    let tokenizer = require_named(&export_dir, "tokenizer.json")?;
    let config = require_named(&export_dir, "config.json")?;
    let dim = flags.dim.unwrap_or(read_hidden_size(&config)?);
    log.event(json!({"event": "onnx_fp32_artifacts_ready", "dim": dim}))?;
    let mut artifacts = vec![
        artifact("model", model)?,
        artifact("tokenizer", tokenizer)?,
        artifact("config", config)?,
    ];
    add_optional(&mut artifacts, "model_data", model_data)?;
    add_optional(
        &mut artifacts,
        "tokenizer_config",
        export_dir.join("tokenizer_config.json"),
    )?;
    add_optional(
        &mut artifacts,
        "special_tokens_map",
        export_dir.join("special_tokens_map.json"),
    )?;
    Ok(artifacts)
}

fn export_onnx(flags: &CommissionFlags, out: &Path, log: &mut ConversionLog) -> CliResult<PathBuf> {
    let export_dir = out.join("onnx-export");
    fs::create_dir_all(&export_dir)?;
    run_command(
        log,
        "optimum-cli",
        &[
            "export",
            "onnx",
            "--model",
            &flags.hf,
            "--task",
            "feature-extraction",
            &export_dir.display().to_string(),
        ],
    )?;
    Ok(export_dir)
}

fn write_manifest(
    flags: &CommissionFlags,
    out: &Path,
    artifacts: &[Artifact],
    dim_override: Option<u32>,
    source_hf_id: Option<&str>,
    log: &mut ConversionLog,
) -> CliResult<PathBuf> {
    let model = artifacts
        .iter()
        .find(|item| item.role == "model")
        .ok_or_else(|| CliError::usage("commission produced no model artifact"))?;
    let dim = dim_override.or(flags.dim).unwrap_or({
        if matches!(flags.runtime, CommissionRuntime::Tei) {
            DEFAULT_TEI_DIM
        } else {
            0
        }
    });
    let inferred_dim = if dim == 0 {
        read_hidden_size(
            &artifacts
                .iter()
                .find(|item| item.role == "config")
                .map(|item| item.path.clone())
                .ok_or_else(|| CliError::usage("commission requires --dim or config.json"))?,
        )?
    } else {
        dim
    };
    let manifest = LensForgeManifest {
        name: flags.lens_name(),
        modality: Modality::Text,
        runtime: flags.runtime.manifest_runtime().to_string(),
        dim: inferred_dim,
        shape: Some(LensForgeShape::from_slot_shape(manifest_shape(
            flags.runtime,
            inferred_dim,
        ))),
        dtype: flags.runtime.default_dtype().to_string(),
        weights_sha256: model.sha256.clone(),
        artifact_set_sha256: Some(artifact_set_sha256(artifacts)?),
        files: manifest_files(out, artifacts)?,
        pooling: manifest_pooling(flags),
        norm: flags.manifest_norm(),
        source_hf_id: source_hf_id.unwrap_or(&flags.hf).to_string(),
        endpoint: flags.endpoint_for_manifest(),
        license: flags.license.clone(),
        non_commercial: flags.non_commercial,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        max_batch: flags.max_batch,
    };
    let path = out.join(MANIFEST_NAME);
    write_json_file(&path, &manifest)?;
    log.event(json!({"event": "manifest_written", "path": path}))?;
    Ok(path)
}

fn manifest_shape(runtime: CommissionRuntime, dim: u32) -> SlotShape {
    match runtime {
        CommissionRuntime::FastembedSparse | CommissionRuntime::FastembedBgem3Sparse => {
            SlotShape::Sparse(dim)
        }
        CommissionRuntime::OnnxColbert | CommissionRuntime::FastembedBgem3Colbert => {
            SlotShape::Multi { token_dim: dim }
        }
        _ => SlotShape::Dense(dim),
    }
}

fn manifest_pooling(flags: &CommissionFlags) -> String {
    if matches!(
        flags.runtime,
        CommissionRuntime::OnnxColbert | CommissionRuntime::FastembedBgem3Colbert
    ) {
        "late-interaction".to_string()
    } else {
        flags.pooling.clone()
    }
}
