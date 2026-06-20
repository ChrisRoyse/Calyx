use std::env;
use std::path::PathBuf;

use crate::error::{CliError, CliResult};
use crate::lens_commands::flags::value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CommissionRuntime {
    OnnxInt8,
    OnnxFp32,
    OnnxColbert,
    FastembedOnnx,
    FastembedSparse,
    FastembedBgem3Dense,
    FastembedBgem3Sparse,
    FastembedBgem3Colbert,
    FastembedReranker,
    FastembedQwen3,
    CandleFp16,
    Tei,
}

impl CommissionRuntime {
    pub(super) fn parse(raw: &str) -> CliResult<Self> {
        match raw {
            "onnx-int8" => Ok(Self::OnnxInt8),
            "onnx-fp32" | "onnx" => Ok(Self::OnnxFp32),
            "onnx-colbert" | "colbert-onnx" | "answerai-colbert" => Ok(Self::OnnxColbert),
            "fastembed-onnx" | "onnx-fastembed" => Ok(Self::FastembedOnnx),
            "fastembed-sparse" => Ok(Self::FastembedSparse),
            "fastembed-bgem3-dense" | "fastembed-bge-m3-dense" => Ok(Self::FastembedBgem3Dense),
            "fastembed-bgem3-sparse" | "fastembed-bge-m3-sparse" => Ok(Self::FastembedBgem3Sparse),
            "fastembed-bgem3-colbert" | "fastembed-bge-m3-colbert" => {
                Ok(Self::FastembedBgem3Colbert)
            }
            "fastembed-reranker" => Ok(Self::FastembedReranker),
            "fastembed-qwen3" | "qwen3" => Ok(Self::FastembedQwen3),
            "candle-fp16" => Ok(Self::CandleFp16),
            "tei" | "tei-http" | "tei_http" => Ok(Self::Tei),
            other => Err(CliError::usage(format!(
                "unsupported --runtime {other}; expected onnx-int8, onnx-fp32, onnx-colbert, fastembed-onnx, fastembed-sparse, fastembed-bgem3-*, fastembed-reranker, fastembed-qwen3, candle-fp16, or tei"
            ))),
        }
    }

    pub(super) const fn manifest_runtime(self) -> &'static str {
        match self {
            Self::OnnxInt8 => "onnx-int8",
            Self::OnnxFp32 => "onnx",
            Self::OnnxColbert => "onnx-colbert",
            Self::FastembedOnnx => "onnx-fastembed",
            Self::FastembedSparse => "fastembed-sparse",
            Self::FastembedBgem3Dense => "fastembed-bgem3-dense",
            Self::FastembedBgem3Sparse => "fastembed-bgem3-sparse",
            Self::FastembedBgem3Colbert => "fastembed-bgem3-colbert",
            Self::FastembedReranker => "fastembed-reranker",
            Self::FastembedQwen3 => "fastembed-qwen3",
            Self::CandleFp16 => "candle-fp16",
            Self::Tei => "tei",
        }
    }

    pub(super) const fn default_dtype(self) -> &'static str {
        match self {
            Self::OnnxInt8 => "int8",
            Self::OnnxFp32 => "f32",
            Self::OnnxColbert => "int8",
            Self::FastembedOnnx
            | Self::FastembedSparse
            | Self::FastembedBgem3Dense
            | Self::FastembedBgem3Sparse
            | Self::FastembedBgem3Colbert
            | Self::FastembedReranker
            | Self::Tei => "f32",
            Self::FastembedQwen3 => "bf16",
            Self::CandleFp16 => "f16",
        }
    }

    pub(super) const fn default_norm(self) -> &'static str {
        match self {
            Self::OnnxColbert
            | Self::FastembedSparse
            | Self::FastembedBgem3Sparse
            | Self::FastembedBgem3Colbert
            | Self::FastembedReranker => "finite",
            _ => "unit",
        }
    }
}

pub(super) struct CommissionFlags {
    pub(super) hf: String,
    pub(super) runtime: CommissionRuntime,
    pub(super) home: Option<PathBuf>,
    pub(super) out: Option<PathBuf>,
    pub(super) name: Option<String>,
    pub(super) endpoint: Option<String>,
    pub(super) dim: Option<u32>,
    pub(super) license: Option<String>,
    pub(super) non_commercial: bool,
    pub(super) pooling: String,
    pub(super) norm: String,
    norm_explicit: bool,
    pub(super) quant_target: String,
    pub(super) max_batch: Option<usize>,
}

impl CommissionFlags {
    pub(super) fn parse(args: &[String]) -> CliResult<Self> {
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
        let mut norm_explicit = false;
        let mut quant_target = "avx2".to_string();
        let mut max_batch = None;
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
                    norm_explicit = true;
                }
                "--quant-target" => {
                    idx += 1;
                    quant_target = value(args, idx, "--quant-target")?.to_string();
                }
                "--max-batch" => {
                    idx += 1;
                    max_batch = Some(parse_positive_usize(
                        value(args, idx, "--max-batch")?,
                        "--max-batch",
                    )?);
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
            norm_explicit,
            quant_target,
            max_batch,
        })
    }

    pub(super) fn output_dir(&self) -> CliResult<PathBuf> {
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

    pub(super) fn lens_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            format!(
                "{}-{}",
                sanitize_path_token(&self.hf),
                self.runtime.manifest_runtime()
            )
        })
    }

    pub(super) fn endpoint_for_manifest(&self) -> Option<String> {
        if matches!(self.runtime, CommissionRuntime::Tei) {
            Some(
                self.endpoint
                    .clone()
                    .unwrap_or_else(|| calyx_registry::DEFAULT_TEI_ENDPOINT.to_string()),
            )
        } else {
            None
        }
    }

    pub(super) fn manifest_norm(&self) -> String {
        if self.norm_explicit {
            self.norm.clone()
        } else {
            self.runtime.default_norm().to_string()
        }
    }
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

fn parse_positive_usize(raw: &str, flag: &str) -> CliResult<usize> {
    let value = raw
        .parse::<usize>()
        .map_err(|err| CliError::usage(format!("{flag} must be an integer: {err}")))?;
    if value == 0 {
        return Err(CliError::usage(format!("{flag} must be > 0")));
    }
    Ok(value)
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
