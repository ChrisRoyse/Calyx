use std::env;
use std::path::PathBuf;

use crate::error::{CliError, CliResult};
use crate::lens_commands::flags::value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CommissionRuntime {
    OnnxInt8,
    FastembedOnnx,
    CandleFp16,
    Tei,
}

impl CommissionRuntime {
    pub(super) fn parse(raw: &str) -> CliResult<Self> {
        match raw {
            "onnx-int8" => Ok(Self::OnnxInt8),
            "fastembed-onnx" | "onnx-fastembed" => Ok(Self::FastembedOnnx),
            "candle-fp16" => Ok(Self::CandleFp16),
            "tei" | "tei-http" | "tei_http" => Ok(Self::Tei),
            other => Err(CliError::usage(format!(
                "unsupported --runtime {other}; expected onnx-int8, fastembed-onnx, candle-fp16, or tei"
            ))),
        }
    }

    pub(super) const fn manifest_runtime(self) -> &'static str {
        match self {
            Self::OnnxInt8 => "onnx-int8",
            Self::FastembedOnnx => "onnx-fastembed",
            Self::CandleFp16 => "candle-fp16",
            Self::Tei => "tei",
        }
    }

    pub(super) const fn default_dtype(self) -> &'static str {
        match self {
            Self::OnnxInt8 => "int8",
            Self::FastembedOnnx | Self::Tei => "f32",
            Self::CandleFp16 => "f16",
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
    pub(super) quant_target: String,
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
