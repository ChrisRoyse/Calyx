use std::env;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use calyx_core::{
    Asymmetry, CalyxError, LensId, Modality, QuantPolicy, Result, SlotShape, content_address,
};
use serde::{Deserialize, Serialize};

use crate::frozen::NormPolicy;

const LENS_UNREACHABLE: &str = "CALYX_LENS_UNREACHABLE";
#[cfg(not(feature = "candle-cuda"))]
const CANDLE_CUDA_FEATURE_MISSING_REASON: &str =
    "candle CUDA requested but calyx-registry was built without feature `candle-cuda`";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FastembedBgem3Output {
    Dense,
    Sparse,
    Colbert,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LensRuntime {
    Algorithmic {
        kind: String,
    },
    TeiHttp {
        endpoint: String,
    },
    CandleLocal {
        model_id: String,
        files: Vec<PathBuf>,
        #[serde(default = "default_candle_dtype")]
        dtype: String,
        #[serde(default = "default_candle_pooling")]
        pooling: String,
    },
    Onnx {
        model_id: String,
        files: Vec<PathBuf>,
    },
    OnnxColbert {
        model_id: String,
        files: Vec<PathBuf>,
    },
    FastembedSparse {
        model_id: String,
        files: Vec<PathBuf>,
    },
    FastembedBgem3 {
        model_id: String,
        files: Vec<PathBuf>,
        output: FastembedBgem3Output,
    },
    FastembedReranker {
        model_id: String,
        files: Vec<PathBuf>,
    },
    FastembedQwen3 {
        model_id: String,
        files: Vec<PathBuf>,
        #[serde(default = "default_qwen3_dtype")]
        dtype: String,
    },
    StaticLookup {
        embeddings_file: PathBuf,
        tokenizer: PathBuf,
        dim: u32,
    },
    MultimodalAdapter {
        axis: String,
        model_id: String,
        #[serde(default)]
        adapter_config: Option<PathBuf>,
        #[serde(default)]
        files: Vec<PathBuf>,
    },
    ExternalCmd {
        cmd: String,
        args: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensSpec {
    pub name: String,
    pub runtime: LensRuntime,
    pub output: SlotShape,
    pub modality: Modality,
    pub weights_sha256: [u8; 32],
    pub corpus_hash: [u8; 32],
    pub norm_policy: NormPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_batch: Option<usize>,
    pub axis: Option<String>,
    pub asymmetry: Asymmetry,
    #[serde(default = "default_quant_default")]
    pub quant_default: QuantPolicy,
    #[serde(default)]
    pub truncate_dim: Option<u32>,
    #[serde(default = "default_recall_delta")]
    pub recall_delta: f32,
    pub retrieval_only: bool,
    pub excluded_from_dedup: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LensHealth {
    Loaded,
    Cold,
    Failing { code: String, reason: String },
}

impl LensSpec {
    pub fn lens_id(&self) -> LensId {
        let output = format!(
            "shape={:?};norm={:?};runtime={:?}",
            self.output, self.norm_policy, self.runtime
        );
        LensId::from_bytes(content_address([
            self.name.as_bytes(),
            &self.weights_sha256,
            &self.corpus_hash,
            output.as_bytes(),
        ]))
    }

    pub fn health(&self) -> LensHealth {
        match &self.runtime {
            LensRuntime::Algorithmic { .. } => LensHealth::Loaded,
            LensRuntime::MultimodalAdapter {
                adapter_config,
                files,
                ..
            } => multimodal_adapter_health(adapter_config.as_ref(), files),
            LensRuntime::TeiHttp { endpoint } => probe_http(endpoint),
            LensRuntime::CandleLocal { files, .. } => candle_local_health(files),
            LensRuntime::Onnx { files, .. }
            | LensRuntime::OnnxColbert { files, .. }
            | LensRuntime::FastembedSparse { files, .. }
            | LensRuntime::FastembedBgem3 { files, .. }
            | LensRuntime::FastembedReranker { files, .. }
            | LensRuntime::FastembedQwen3 { files, .. } => files_runtime_health(files),
            LensRuntime::StaticLookup {
                embeddings_file,
                tokenizer,
                ..
            } => {
                if embeddings_file.is_file() && tokenizer.is_file() {
                    LensHealth::Loaded
                } else {
                    LensHealth::Cold
                }
            }
            LensRuntime::ExternalCmd { cmd, .. } => {
                if command_exists(cmd) {
                    LensHealth::Loaded
                } else {
                    LensHealth::Failing {
                        code: LENS_UNREACHABLE.to_string(),
                        reason: format!("external command {cmd} is not executable"),
                    }
                }
            }
        }
    }

    pub fn health_result(&self) -> Result<LensHealth> {
        let health = self.health();
        match &health {
            LensHealth::Failing { reason, .. } => Err(CalyxError::lens_unreachable(reason)),
            _ => Ok(health),
        }
    }
}

fn default_candle_dtype() -> String {
    "f32".to_string()
}

fn default_candle_pooling() -> String {
    "mean".to_string()
}

fn default_qwen3_dtype() -> String {
    "bf16".to_string()
}

pub const fn default_quant_default() -> QuantPolicy {
    QuantPolicy::turboquant_default()
}

pub const fn default_recall_delta() -> f32 {
    0.02
}

fn candle_local_health(files: &[PathBuf]) -> LensHealth {
    match files_runtime_health(files) {
        LensHealth::Loaded => candle_cuda_runtime_health(),
        health => health,
    }
}

fn files_runtime_health(files: &[PathBuf]) -> LensHealth {
    if files.is_empty() {
        return LensHealth::Cold;
    }
    if files.iter().all(|path| path.exists()) {
        LensHealth::Loaded
    } else {
        LensHealth::Cold
    }
}

fn multimodal_adapter_health(adapter_config: Option<&PathBuf>, files: &[PathBuf]) -> LensHealth {
    let Some(adapter_config) = adapter_config else {
        return LensHealth::Cold;
    };
    if !adapter_config.is_file() {
        return LensHealth::Cold;
    }
    files_runtime_health(files)
}

#[cfg(feature = "candle-cuda")]
fn candle_cuda_runtime_health() -> LensHealth {
    LensHealth::Loaded
}

#[cfg(not(feature = "candle-cuda"))]
fn candle_cuda_runtime_health() -> LensHealth {
    LensHealth::Failing {
        code: LENS_UNREACHABLE.to_string(),
        reason: CANDLE_CUDA_FEATURE_MISSING_REASON.to_string(),
    }
}

fn probe_http(endpoint: &str) -> LensHealth {
    let Some(rest) = endpoint.strip_prefix("http://") else {
        return LensHealth::Failing {
            code: LENS_UNREACHABLE.to_string(),
            reason: "endpoint is not http://".to_string(),
        };
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let (host, port) = authority
        .rsplit_once(':')
        .and_then(|(host, port)| port.parse::<u16>().ok().map(|port| (host, port)))
        .unwrap_or((authority, 80));
    let address = match (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut it| it.next())
    {
        Some(address) => address,
        None => {
            return LensHealth::Failing {
                code: LENS_UNREACHABLE.to_string(),
                reason: format!("{endpoint} resolved no socket address"),
            };
        }
    };
    match TcpStream::connect_timeout(&address, Duration::from_millis(250)) {
        Ok(_) => LensHealth::Loaded,
        Err(err) => LensHealth::Failing {
            code: LENS_UNREACHABLE.to_string(),
            reason: format!("connect {endpoint} failed: {err}"),
        },
    }
}

fn command_exists(cmd: &str) -> bool {
    let path = PathBuf::from(cmd);
    if path.components().count() > 1 {
        return path.is_file();
    }
    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .any(|dir| dir.join(cmd).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{Asymmetry, Modality, QuantPolicy, SlotShape};
    use std::fs;

    fn candle_spec(files: Vec<PathBuf>) -> LensSpec {
        LensSpec {
            name: "fixture-candle".to_string(),
            runtime: LensRuntime::CandleLocal {
                model_id: "fixture/model".to_string(),
                files,
                dtype: "f16".to_string(),
                pooling: "mean".to_string(),
            },
            output: SlotShape::Dense(384),
            modality: Modality::Text,
            weights_sha256: [1_u8; 32],
            corpus_hash: [2_u8; 32],
            norm_policy: NormPolicy::unit(),
            max_batch: None,
            axis: None,
            asymmetry: Asymmetry::None,
            quant_default: QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: default_recall_delta(),
            retrieval_only: false,
            excluded_from_dedup: false,
        }
    }

    #[test]
    fn candle_health_reflects_cuda_feature_availability() {
        let root = std::env::temp_dir().join(format!("calyx-candle-health-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let weights = root.join("model.safetensors");
        let tokenizer = root.join("tokenizer.json");
        let config = root.join("config.json");
        fs::write(&weights, b"weights").unwrap();
        fs::write(&tokenizer, b"tokenizer").unwrap();
        fs::write(&config, b"config").unwrap();

        let health = candle_spec(vec![weights, tokenizer, config]).health();

        #[cfg(feature = "candle-cuda")]
        {
            assert_eq!(health, LensHealth::Loaded);
        }
        #[cfg(not(feature = "candle-cuda"))]
        {
            assert_eq!(
                health,
                LensHealth::Failing {
                    code: LENS_UNREACHABLE.to_string(),
                    reason: CANDLE_CUDA_FEATURE_MISSING_REASON.to_string(),
                }
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn candle_health_reports_cold_before_cuda_feature_check_when_files_missing() {
        let spec = candle_spec(vec![std::env::temp_dir().join("calyx-missing-candle-file")]);

        assert_eq!(spec.health(), LensHealth::Cold);
    }
}
