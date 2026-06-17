use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use calyx_core::{CalyxError, Result};
use serde::Deserialize;

use super::axis::MultimodalAxis;

const ADAPTER_SCHEMA: &str = "calyx-multimodal-adapter-v2";
const ENGINE_ONNX_EXTERNAL: &str = "onnx-external";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

#[derive(Clone, Debug)]
pub struct MultimodalAdapterConfig {
    pub path: PathBuf,
    pub axis: MultimodalAxis,
    pub model_id: String,
    pub processor_model_id: String,
    pub dim: u32,
    pub command: String,
    pub helper: PathBuf,
    pub model_file: PathBuf,
    pub provider: String,
    pub timeout: Duration,
}

#[derive(Deserialize)]
struct RawAdapterConfig {
    schema: String,
    engine: String,
    axis: String,
    model_id: String,
    #[serde(default)]
    processor_model_id: Option<String>,
    dim: u32,
    #[serde(default)]
    python: Option<String>,
    helper: PathBuf,
    model_file: PathBuf,
    #[serde(default = "default_provider")]
    provider: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

pub fn load_adapter_config(
    path: &Path,
    expected_axis: MultimodalAxis,
    expected_model_id: &str,
    expected_dim: Option<u32>,
) -> Result<MultimodalAdapterConfig> {
    let bytes = fs::read(path).map_err(|err| {
        config_invalid(format!(
            "read multimodal adapter config {} failed: {err}",
            path.display()
        ))
    })?;
    let raw: RawAdapterConfig = serde_json::from_slice(&bytes).map_err(|err| {
        config_invalid(format!(
            "parse multimodal adapter config {} failed: {err}",
            path.display()
        ))
    })?;
    if raw.schema != ADAPTER_SCHEMA {
        return Err(config_invalid(format!(
            "unsupported multimodal adapter schema {}",
            raw.schema
        )));
    }
    if raw.engine != ENGINE_ONNX_EXTERNAL {
        return Err(config_invalid(format!(
            "unsupported multimodal adapter engine {}",
            raw.engine
        )));
    }
    let axis = MultimodalAxis::parse(&raw.axis)?;
    if axis != expected_axis {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "multimodal adapter config axis {} != expected {}",
            axis.as_str(),
            expected_axis.as_str()
        )));
    }
    if raw.model_id != expected_model_id {
        return Err(CalyxError::lens_frozen_violation(format!(
            "multimodal adapter config model {} != expected {}",
            raw.model_id, expected_model_id
        )));
    }
    if raw.dim == 0 {
        return Err(config_invalid("multimodal adapter config dim must be > 0"));
    }
    if let Some(expected) = expected_dim
        && raw.dim != expected
    {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "multimodal adapter config dim {} != expected {}",
            raw.dim, expected
        )));
    }
    if raw.provider != "cpu_explicit" {
        return Err(config_invalid(format!(
            "unsupported multimodal adapter provider {}",
            raw.provider
        )));
    }
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let helper = resolve_path(base, raw.helper);
    let model_file = resolve_path(base, raw.model_file);
    ensure_file("helper", &helper)?;
    ensure_file("model", &model_file)?;
    Ok(MultimodalAdapterConfig {
        path: path.to_path_buf(),
        axis,
        model_id: raw.model_id,
        processor_model_id: raw
            .processor_model_id
            .unwrap_or_else(|| expected_model_id.to_string()),
        dim: raw.dim,
        command: raw.python.unwrap_or_else(|| "python3".to_string()),
        helper,
        model_file,
        provider: raw.provider,
        timeout: Duration::from_millis(raw.timeout_ms),
    })
}

impl MultimodalAdapterConfig {
    pub fn contract_paths(&self) -> Vec<PathBuf> {
        vec![
            self.model_file.clone(),
            self.path.clone(),
            self.helper.clone(),
        ]
    }
}

fn default_provider() -> String {
    "cpu_explicit".to_string()
}

const fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn resolve_path(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn ensure_file(label: &str, path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }
    Err(config_invalid(format!(
        "multimodal adapter {label} file {} is missing",
        path.display()
    )))
}

pub fn config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix the multimodal adapter lens spec",
    }
}
