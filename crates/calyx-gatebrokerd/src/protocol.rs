use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 65_536;
pub const MAX_ARGV_ITEMS: usize = 1_024;
pub const MAX_ENV_ITEMS: usize = 256;
const MAX_ARGV_BYTES: usize = 32_768;
const MAX_ENV_BYTES: usize = 32_768;

#[derive(Debug, Clone, Copy)]
enum StringRule {
    Token,
    Uuid,
    Leaf,
    AbsolutePath,
    RelativePath,
    Text,
    Unit,
    EnvName,
    Hex32,
    Hex64,
}

fn validate_string(value: &str, max: usize, rule: StringRule) -> Result<(), String> {
    if value.is_empty() {
        return Err("value must not be empty".into());
    }
    if value.len() > max {
        return Err(format!("value exceeds the {max}-byte limit"));
    }
    if value.bytes().any(|byte| matches!(byte, 0 | b'\r' | b'\n')) {
        return Err("value contains a forbidden control byte".into());
    }
    match rule {
        StringRule::Token => {
            if !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            {
                return Err("token contains a forbidden character".into());
            }
        }
        StringRule::Uuid => {
            if value.len() != 36
                || value.bytes().enumerate().any(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte != b'-'
                    } else {
                        !matches!(byte, b'0'..=b'9' | b'a'..=b'f')
                    }
                })
            {
                return Err("request id must be a canonical lowercase UUID".into());
            }
        }
        StringRule::Leaf => {
            if matches!(value, "." | "..") || value.as_bytes().contains(&b'/') {
                return Err("leaf must be exactly one non-special path component".into());
            }
        }
        StringRule::AbsolutePath => {
            if !value.starts_with('/')
                || (value.len() > 1 && value.ends_with('/'))
                || value.contains("//")
                || value
                    .split('/')
                    .skip(1)
                    .any(|part| matches!(part, "." | ".."))
            {
                return Err("path must be a normalized absolute path without traversal".into());
            }
        }
        StringRule::RelativePath => {
            if value != "."
                && (value.starts_with('/')
                    || value.ends_with('/')
                    || value.contains("//")
                    || value
                        .split('/')
                        .any(|part| part.is_empty() || matches!(part, "." | "..")))
            {
                return Err(
                    "path must be normalized, descriptor-relative, and traversal-free".into(),
                );
            }
        }
        StringRule::EnvName => {
            let mut bytes = value.bytes();
            let Some(first) = bytes.next() else {
                return Err("environment name must not be empty".into());
            };
            if !(first.is_ascii_alphabetic() || first == b'_')
                || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err("environment name is not a portable identifier".into());
            }
        }
        StringRule::Hex64 => {
            if value.len() != 64
                || !value
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(
                    "token must contain exactly 64 lowercase hexadecimal characters".into(),
                );
            }
        }
        StringRule::Hex32 => {
            if value.len() != 32
                || !value
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err("id must contain exactly 32 lowercase hexadecimal characters".into());
            }
        }
        StringRule::Text => {}
        StringRule::Unit => {
            if !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-' | b'.' | b'@')
            }) {
                return Err("unit name contains a forbidden character".into());
            }
        }
    }
    Ok(())
}

macro_rules! bounded_string {
    ($name:ident, $max:expr, $rule:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ProtocolError> {
                let value = value.into();
                validate_string(&value, $max, $rule).map_err(|reason| {
                    ProtocolError::InvalidField {
                        field: stringify!($name),
                        reason,
                    }
                })?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                validate_string(&value, $max, $rule).map_err(serde::de::Error::custom)?;
                Ok(Self(value))
            }
        }
    };
}

bounded_string!(RequestId, 36, StringRule::Uuid);
bounded_string!(RunId, 32, StringRule::Hex32);
bounded_string!(RunToken, 64, StringRule::Hex64);
bounded_string!(ObjectId, 32, StringRule::Hex32);
bounded_string!(StageId, 32, StringRule::Hex32);
bounded_string!(InvocationId, 32, StringRule::Hex32);
bounded_string!(UnitName, 255, StringRule::Unit);
bounded_string!(ProfileName, 64, StringRule::Token);
bounded_string!(RoleName, 64, StringRule::Token);
bounded_string!(RootAlias, 64, StringRule::Token);
// A read-only execution-cwd capability. This is intentionally distinct from
// RootAlias, which grants mutable object-namespace authority.
bounded_string!(ExecutionRootAlias, 64, StringRule::Token);
bounded_string!(LeafName, 240, StringRule::Leaf);
bounded_string!(StageLabel, 96, StringRule::Token);
bounded_string!(CwdPath, 4_096, StringRule::AbsolutePath);
bounded_string!(RelativePath, 4_095, StringRule::RelativePath);
bounded_string!(AbsolutePath, 4_096, StringRule::AbsolutePath);
bounded_string!(ArgValue, 4_096, StringRule::Text);
bounded_string!(EnvName, 128, StringRule::EnvName);
bounded_string!(EnvValue, 8_192, StringRule::Text);
bounded_string!(ReasonText, 1_024, StringRule::Text);
bounded_string!(ErrorText, 2_048, StringRule::Text);
bounded_string!(ContextKey, 96, StringRule::Token);
bounded_string!(ContextValue, 1_024, StringRule::Text);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestEnvelope {
    pub version: u16,
    pub request: Request,
}

impl RequestEnvelope {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion {
                expected: PROTOCOL_VERSION,
                actual: self.version,
            });
        }
        self.request.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "verb", content = "params", rename_all = "snake_case")]
pub enum Request {
    Health(HealthRequest),
    BeginRun(BeginRunRequest),
    CreateObject(CreateObjectRequest),
    ExecStage(ExecStageRequest),
    DeleteObject(DeleteObjectRequest),
    FinishRun(FinishRunRequest),
    AbortRun(AbortRunRequest),
    Inspect(InspectRequest),
}

impl Request {
    pub fn request_id(&self) -> &RequestId {
        match self {
            Self::Health(request) => &request.request_id,
            Self::BeginRun(request) => &request.request_id,
            Self::CreateObject(request) => &request.request_id,
            Self::ExecStage(request) => &request.request_id,
            Self::DeleteObject(request) => &request.request_id,
            Self::FinishRun(request) => &request.request_id,
            Self::AbortRun(request) => &request.request_id,
            Self::Inspect(request) => &request.request_id,
        }
    }

    fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::BeginRun(request) if request.owner_pid == 0 || request.owner_starttime == 0 => {
                return Err(ProtocolError::InvalidField {
                    field: "owner_pid/owner_starttime",
                    reason: "both values must be nonzero".into(),
                });
            }
            Self::ExecStage(request) => request.validate()?,
            Self::Inspect(request) if request.run_id.is_some() != request.run_token.is_some() => {
                return Err(ProtocolError::InvalidField {
                    field: "run_id/run_token",
                    reason: "inspect must provide both fields or neither field".into(),
                });
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthRequest {
    pub request_id: RequestId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginRunRequest {
    pub request_id: RequestId,
    pub profile: ProfileName,
    pub owner_pid: u32,
    pub owner_starttime: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateObjectRequest {
    pub request_id: RequestId,
    pub run_id: RunId,
    pub run_token: RunToken,
    pub role: RoleName,
    pub root_alias: RootAlias,
    pub leaf: Option<LeafName>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnvEntry {
    pub name: EnvName,
    pub value: EnvValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecStageRequest {
    pub request_id: RequestId,
    pub run_id: RunId,
    pub run_token: RunToken,
    pub label: StageLabel,
    pub cwd_root: ExecutionRootAlias,
    pub cwd: RelativePath,
    pub argv: Vec<ArgValue>,
    #[serde(default)]
    pub env: Vec<EnvEntry>,
}

impl ExecStageRequest {
    fn validate(&self) -> Result<(), ProtocolError> {
        if self.argv.is_empty() || self.argv.len() > MAX_ARGV_ITEMS {
            return Err(ProtocolError::InvalidCollection {
                field: "argv",
                reason: format!("expected 1..={MAX_ARGV_ITEMS} entries"),
            });
        }
        let argv_bytes: usize = self.argv.iter().map(|value| value.as_str().len()).sum();
        if argv_bytes > MAX_ARGV_BYTES {
            return Err(ProtocolError::InvalidCollection {
                field: "argv",
                reason: format!("combined size exceeds {MAX_ARGV_BYTES} bytes"),
            });
        }
        if self.env.len() > MAX_ENV_ITEMS {
            return Err(ProtocolError::InvalidCollection {
                field: "env",
                reason: format!("contains more than {MAX_ENV_ITEMS} entries"),
            });
        }
        let mut names = BTreeSet::new();
        let mut env_bytes = 0;
        for entry in &self.env {
            if !names.insert(entry.name.as_str()) {
                return Err(ProtocolError::InvalidCollection {
                    field: "env",
                    reason: format!("duplicate name {}", entry.name),
                });
            }
            env_bytes += entry.name.as_str().len() + entry.value.as_str().len();
        }
        if env_bytes > MAX_ENV_BYTES {
            return Err(ProtocolError::InvalidCollection {
                field: "env",
                reason: format!("combined size exceeds {MAX_ENV_BYTES} bytes"),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteObjectRequest {
    pub request_id: RequestId,
    pub run_id: RunId,
    pub run_token: RunToken,
    pub object_id: ObjectId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishRunRequest {
    pub request_id: RequestId,
    pub run_id: RunId,
    pub run_token: RunToken,
    pub intended_status: RunStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbortRunRequest {
    pub request_id: RequestId,
    pub run_id: RunId,
    pub run_token: RunToken,
    pub reason: ReasonText,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectRequest {
    pub request_id: RequestId,
    pub run_id: Option<RunId>,
    pub run_token: Option<RunToken>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Succeeded,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectState {
    Prepared,
    Published,
    Quarantined,
    Deleted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticIdentity {
    pub device: u64,
    pub inode: u64,
}

mod response;
pub use response::*;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("frame is empty")]
    EmptyFrame,
    #[error("frame is {actual} bytes; maximum is {maximum}")]
    OversizedFrame { actual: usize, maximum: usize },
    #[error("invalid JSON request: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("protocol version {actual} is unsupported; expected {expected}")]
    UnsupportedVersion { expected: u16, actual: u16 },
    #[error("invalid {field}: {reason}")]
    InvalidField { field: &'static str, reason: String },
    #[error("invalid {field}: {reason}")]
    InvalidCollection { field: &'static str, reason: String },
}

pub fn decode_request(frame: &[u8]) -> Result<RequestEnvelope, ProtocolError> {
    if frame.is_empty() {
        return Err(ProtocolError::EmptyFrame);
    }
    if frame.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::OversizedFrame {
            actual: frame.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }
    let envelope: RequestEnvelope = serde_json::from_slice(frame)?;
    envelope.validate()?;
    Ok(envelope)
}
