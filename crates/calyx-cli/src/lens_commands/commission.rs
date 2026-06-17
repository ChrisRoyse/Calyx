use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{Input, Lens, Modality, QuantPolicy, SlotShape};
use calyx_registry::{DEFAULT_TEI_ENDPOINT, LensForgeManifest, NormPolicy, TeiHttpLens};
use serde::{Deserialize, Serialize};
use serde_json::json;

mod artifact;
mod log;

use artifact::{
    Artifact, FileReport, add_optional, artifact, artifact_set_sha256, file_report, find_preferred,
    manifest_files, read_hidden_size, require_named, require_named_fallback,
};
use log::{ConversionLog, run_command, write_json_file};

use super::catalog::{AddReport, add_manifest_to_catalog};
use super::flags::value;
use super::support::validate_vector_contract;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_TEI_DIM: u32 = 768;
const MANIFEST_NAME: &str = "lensforge.manifest.json";
const CONVERSION_LOG_NAME: &str = "conversion-log.jsonl";

#[derive(Clone, Copy)]
enum CommissionRuntime {
    OnnxInt8,
    CandleFp16,
    Tei,
}

impl CommissionRuntime {
    fn parse(raw: &str) -> CliResult<Self> {
        match raw {
            "onnx-int8" => Ok(Self::OnnxInt8),
            "candle-fp16" => Ok(Self::CandleFp16),
            "tei" | "tei-http" | "tei_http" => Ok(Self::Tei),
            other => Err(CliError::usage(format!(
                "unsupported --runtime {other}; expected onnx-int8, candle-fp16, or tei"
            ))),
        }
    }

    const fn manifest_runtime(self) -> &'static str {
        match self {
            Self::OnnxInt8 => "onnx-int8",
            Self::CandleFp16 => "candle-fp16",
            Self::Tei => "tei",
        }
    }

    const fn default_dtype(self) -> &'static str {
        match self {
            Self::OnnxInt8 => "int8",
            Self::CandleFp16 => "f16",
            Self::Tei => "f32",
        }
    }
}

struct CommissionFlags {
    hf: String,
    runtime: CommissionRuntime,
    home: Option<PathBuf>,
    out: Option<PathBuf>,
    name: Option<String>,
    endpoint: Option<String>,
    dim: Option<u32>,
    license: Option<String>,
    non_commercial: bool,
    pooling: String,
    norm: String,
    quant_target: String,
}

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
    let artifacts = match flags.runtime {
        CommissionRuntime::Tei => commission_tei(&flags, &out, &mut log)?,
        CommissionRuntime::CandleFp16 => commission_candle(&flags, &out, &mut log)?,
        CommissionRuntime::OnnxInt8 => commission_onnx_int8(&flags, &out, &mut log)?,
    };
    let manifest_path = write_manifest(&flags, &out, &artifacts, &mut log)?;
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
        files: artifacts.iter().map(file_report).collect(),
        registered,
    })
}

impl CommissionFlags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut hf = None;
        let mut runtime = None;
        let mut home = None;
        let mut out = None;
        let mut name = None;
        let mut endpoint = None;
        let mut dim = None;
        let mut license = None;
        let mut non_commercial = false;
        let mut pooling = "mean".to_string();
        let mut norm = "unit".to_string();
        let mut quant_target = "avx2".to_string();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--hf" => {
                    idx += 1;
                    hf = Some(value(args, idx, "--hf")?.to_string());
                }
                "--runtime" => {
                    idx += 1;
                    runtime = Some(CommissionRuntime::parse(value(args, idx, "--runtime")?)?);
                }
                "--home" => {
                    idx += 1;
                    home = Some(value(args, idx, "--home")?.into());
                }
                "--out" => {
                    idx += 1;
                    out = Some(value(args, idx, "--out")?.into());
                }
                "--name" => {
                    idx += 1;
                    name = Some(value(args, idx, "--name")?.to_string());
                }
                "--endpoint" => {
                    idx += 1;
                    endpoint = Some(value(args, idx, "--endpoint")?.to_string());
                }
                "--dim" => {
                    idx += 1;
                    let raw = value(args, idx, "--dim")?;
                    dim = Some(raw.parse().map_err(|err| {
                        CliError::usage(format!("parse --dim value {raw}: {err}"))
                    })?);
                }
                "--license" => {
                    idx += 1;
                    license = Some(value(args, idx, "--license")?.to_string());
                }
                "--non-commercial" => non_commercial = true,
                "--pooling" => {
                    idx += 1;
                    pooling = value(args, idx, "--pooling")?.to_string();
                }
                "--norm" => {
                    idx += 1;
                    norm = value(args, idx, "--norm")?.to_string();
                }
                "--quant-target" => {
                    idx += 1;
                    quant_target = value(args, idx, "--quant-target")?.to_string();
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected lens commission flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        let hf = require_nonempty(hf, "--hf")?;
        let runtime = runtime.ok_or_else(|| CliError::usage("--runtime is required"))?;
        validate_quant_target(&quant_target)?;
        Ok(Self {
            hf,
            runtime,
            home,
            out,
            name,
            endpoint,
            dim,
            license,
            non_commercial,
            pooling,
            norm,
            quant_target,
        })
    }

    fn output_dir(&self) -> CliResult<PathBuf> {
        if let Some(out) = &self.out {
            return Ok(out.clone());
        }
        let home = match &self.home {
            Some(path) => path.clone(),
            None => env::var_os("CALYX_HOME")
                .map(PathBuf::from)
                .ok_or_else(|| CliError::usage("CALYX_HOME is required or pass --home <dir>"))?,
        };
        Ok(home.join("lenses").join("commissioned").join(format!(
            "{}-{}",
            sanitize_path_token(&self.hf),
            self.runtime.manifest_runtime()
        )))
    }

    fn lens_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            format!(
                "{}-{}",
                sanitize_path_token(&self.hf),
                self.runtime.manifest_runtime()
            )
        })
    }
}

fn commission_tei(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<Vec<Artifact>> {
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
    let descriptor = TeiDescriptor {
        source_hf_id: flags.hf.clone(),
        endpoint,
        modality: "text".to_string(),
        dim,
        norm: "unit".to_string(),
    };
    let path = out.join("tei-descriptor.json");
    write_json_file(&path, &descriptor)?;
    log.event(json!({
        "event": "tei_probe_verified",
        "descriptor": path,
        "dim": dim,
    }))?;
    Ok(vec![artifact("model", path)?])
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
    let export_dir = out.join("onnx-export");
    let quant_dir = out.join("onnx-int8");
    fs::create_dir_all(&export_dir)?;
    fs::create_dir_all(&quant_dir)?;
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
    let tokenizer = require_named_fallback(&quant_dir, &export_dir, "tokenizer.json")?;
    let config = require_named_fallback(&quant_dir, &export_dir, "config.json")?;
    let dim = flags.dim.unwrap_or(read_hidden_size(&config)?);
    log.event(json!({"event": "onnx_int8_artifacts_ready", "dim": dim}))?;
    let mut artifacts = vec![
        artifact("model", model)?,
        artifact("tokenizer", tokenizer)?,
        artifact("config", config)?,
    ];
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

fn write_manifest(
    flags: &CommissionFlags,
    out: &Path,
    artifacts: &[Artifact],
    log: &mut ConversionLog,
) -> CliResult<PathBuf> {
    let model = artifacts
        .iter()
        .find(|item| item.role == "model")
        .ok_or_else(|| CliError::usage("commission produced no model artifact"))?;
    let dim = flags.dim.unwrap_or({
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
        dtype: flags.runtime.default_dtype().to_string(),
        weights_sha256: model.sha256.clone(),
        artifact_set_sha256: Some(artifact_set_sha256(artifacts)?),
        files: manifest_files(out, artifacts)?,
        pooling: flags.pooling.clone(),
        norm: flags.norm.clone(),
        source_hf_id: flags.hf.clone(),
        endpoint: flags.endpoint_for_manifest(),
        license: flags.license.clone(),
        non_commercial: flags.non_commercial,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
    };
    let path = out.join(MANIFEST_NAME);
    write_json_file(&path, &manifest)?;
    log.event(json!({"event": "manifest_written", "path": path}))?;
    Ok(path)
}

impl CommissionFlags {
    fn endpoint_for_manifest(&self) -> Option<String> {
        if matches!(self.runtime, CommissionRuntime::Tei) {
            Some(
                self.endpoint
                    .clone()
                    .unwrap_or_else(|| DEFAULT_TEI_ENDPOINT.to_string()),
            )
        } else {
            None
        }
    }
}

#[derive(Serialize, Deserialize)]
struct TeiDescriptor {
    source_hf_id: String,
    endpoint: String,
    modality: String,
    dim: u32,
    norm: String,
}

fn require_nonempty(value: Option<String>, flag: &str) -> CliResult<String> {
    let value = value.ok_or_else(|| CliError::usage(format!("{flag} is required")))?;
    if value.trim().is_empty() {
        return Err(CliError::usage(format!("{flag} must not be empty")));
    }
    Ok(value)
}

fn validate_quant_target(raw: &str) -> CliResult {
    match raw {
        "arm64" | "avx2" | "avx512" | "avx512_vnni" | "tensorrt" => Ok(()),
        other => Err(CliError::usage(format!(
            "--quant-target {other} is unsupported"
        ))),
    }
}

fn sanitize_path_token(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
