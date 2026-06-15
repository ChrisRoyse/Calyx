use std::env;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use calyx_core::{
    Asymmetry, CalyxError, LensId, Modality, QuantPolicy, Result, SlotShape, content_address,
};
use serde::{Deserialize, Serialize};

use crate::frozen::NormPolicy;

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
    StaticLookup {
        embeddings_file: PathBuf,
        tokenizer: PathBuf,
        dim: u32,
    },
    MultimodalAdapter {
        axis: String,
        model_id: String,
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
            LensRuntime::MultimodalAdapter { .. } => LensHealth::Loaded,
            LensRuntime::TeiHttp { endpoint } => probe_http(endpoint),
            LensRuntime::CandleLocal { files, .. } | LensRuntime::Onnx { files, .. } => {
                if files.is_empty() {
                    return LensHealth::Cold;
                }
                if files.iter().all(|path| path.exists()) {
                    LensHealth::Loaded
                } else {
                    LensHealth::Cold
                }
            }
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
                        code: "CALYX_LENS_UNREACHABLE".to_string(),
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

pub const fn default_quant_default() -> QuantPolicy {
    QuantPolicy::turboquant_default()
}

pub const fn default_recall_delta() -> f32 {
    0.02
}

fn probe_http(endpoint: &str) -> LensHealth {
    let Some(rest) = endpoint.strip_prefix("http://") else {
        return LensHealth::Failing {
            code: "CALYX_LENS_UNREACHABLE".to_string(),
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
                code: "CALYX_LENS_UNREACHABLE".to_string(),
                reason: format!("{endpoint} resolved no socket address"),
            };
        }
    };
    match TcpStream::connect_timeout(&address, Duration::from_millis(250)) {
        Ok(_) => LensHealth::Loaded,
        Err(err) => LensHealth::Failing {
            code: "CALYX_LENS_UNREACHABLE".to_string(),
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
