//! Crate error type.

use calyx_core::CalyxError;
use serde::{Deserialize, Serialize};

/// Convenience result alias for `calyx-poly`.
pub type Result<T> = std::result::Result<T, PolyError>;

/// Errors surfaced by the Polymarket engine. Fail-closed: every variant means "do not proceed".
#[derive(Debug, thiserror::Error)]
pub enum PolyError {
    /// Invalid or missing configuration.
    #[error("config error {code}: {message}")]
    Config {
        /// Stable Poly config error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// A data feed was unavailable or returned an unusable payload.
    #[error("feed error: {0}")]
    Feed(String),

    /// Raw Polymarket source inventory or sample-corpus capture failed.
    #[error("raw source error {code}: {message}")]
    RawSource {
        /// Stable Poly raw-source error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// An encoder received an invalid input (e.g. non-finite value).
    #[error("encode error: {0}")]
    Encode(String),

    /// A record failed schema validation before it could be stored.
    #[error("schema error: {0}")]
    Schema(String),

    /// Forecast admission refused a prediction (carries the human-readable reason).
    #[error("admission refused: {0}")]
    AdmissionRefused(String),

    /// Forecast-agent secret retrieval or validation failed before an LLM call could be made.
    #[error("agent secret error {code}: {message}")]
    AgentSecret {
        /// Stable Poly agent-secret error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// Forecast-agent artifact schema, parsing, or persistence failed.
    #[error("agent artifact error {code}: {message}")]
    AgentArtifact {
        /// Stable Poly agent-artifact error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// Forecast-agent artifact reproduction failed before bit-for-bit proof completed.
    #[error("agent reproduction error {code}: {message}")]
    AgentReproduction {
        /// Stable Poly agent-reproduction error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// Forecast-agent launch, provider call, or ledger persistence failed.
    #[error("agent launch error {code}: {message}")]
    AgentLaunch {
        /// Stable Poly agent-launch error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// A backtest input/report failed validation or did not clear the required baseline.
    #[error("backtest error {code}: {message}")]
    Backtest {
        /// Stable Poly backtest error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// A per-domain kernel recall gate failed before predictions could be trusted.
    #[error("kernel recall error {code}: {message}")]
    KernelRecall {
        /// Stable Poly or Calyx kernel-recall error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// A local file-size lint gate failed before source hygiene could be trusted.
    #[error("file-size lint error {code}: {message}")]
    FileSizeLint {
        /// Stable Poly file-size lint error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// Canonical snapshot identity serialization failed. Fail closed rather than emit empty or
    /// ambiguous identity bytes that could collapse unrelated snapshots onto one content address.
    #[error("snapshot identity error {code}: {message}")]
    SnapshotIdentity {
        /// Stable Poly snapshot-identity error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// A market grounding operation failed before provenance was complete.
    #[error("grounding error {code}: {message}")]
    Grounding {
        /// Stable Poly grounding error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// Runtime local-only policy enforcement failed closed.
    #[error("policy error {code}: {message}")]
    Policy {
        /// Stable Poly policy error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// Forecast outcome scoring failed before durable score provenance completed.
    #[error("score error {code}: {message}")]
    Score {
        /// Stable Poly scoring error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// Structured logging failed before diagnostic evidence could be persisted.
    #[error("structured log error {code}: {message}")]
    StructuredLog {
        /// Stable Poly structured-log error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// A panel information-diagnostic or association-materialization computation failed before its
    /// result could be persisted and read back (issues #207, #208).
    #[error("diagnostics error {code}: {message}")]
    Diagnostics {
        /// Stable Poly diagnostics error code.
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// An error bubbled up from the Calyx engine.
    #[error("calyx error {code}: {message}")]
    Calyx {
        /// Stable Calyx error code.
        code: String,
        /// Human-readable message.
        message: String,
    },
}

impl PolyError {
    /// Builds a fail-closed config error with a stable code.
    pub fn config(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Config {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed raw-source sampling error with a stable code.
    pub fn raw_source(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::RawSource {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed backtest error with a stable code.
    pub fn backtest(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Backtest {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed kernel-recall error with a stable code.
    pub fn kernel_recall(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::KernelRecall {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed file-size lint error with a stable code.
    pub fn file_size_lint(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::FileSizeLint {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed canonical snapshot-identity error with a stable code.
    pub fn snapshot_identity(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::SnapshotIdentity {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed grounding error with a stable code.
    pub fn grounding(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Grounding {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed panel/association diagnostics error with a stable code.
    pub fn diagnostics(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Diagnostics {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed local-only runtime policy error with a stable code.
    pub fn policy(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Policy {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed forecast scoring error with a stable code.
    pub fn score(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Score {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed structured-log error with a stable code.
    pub fn structured_log(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::StructuredLog {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed forecast-agent secret error with a stable code.
    pub fn agent_secret(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::AgentSecret {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed forecast-agent artifact error with a stable code.
    pub fn agent_artifact(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::AgentArtifact {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed forecast-agent reproduction error with a stable code.
    pub fn agent_reproduction(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::AgentReproduction {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Builds a fail-closed forecast-agent launcher error with a stable code.
    pub fn agent_launch(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::AgentLaunch {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Returns the stable machine-readable error code.
    pub fn code(&self) -> &str {
        match self {
            Self::Config { code, .. }
            | Self::RawSource { code, .. }
            | Self::AgentSecret { code, .. }
            | Self::AgentArtifact { code, .. }
            | Self::AgentReproduction { code, .. }
            | Self::AgentLaunch { code, .. }
            | Self::Backtest { code, .. }
            | Self::KernelRecall { code, .. }
            | Self::FileSizeLint { code, .. }
            | Self::SnapshotIdentity { code, .. }
            | Self::Grounding { code, .. }
            | Self::Policy { code, .. }
            | Self::Score { code, .. }
            | Self::StructuredLog { code, .. }
            | Self::Diagnostics { code, .. }
            | Self::Calyx { code, .. } => code,
            Self::Feed(_) => "POLY_FEED_FAILED",
            Self::Encode(_) => "POLY_ENCODE_FAILED",
            Self::Schema(_) => "POLY_SCHEMA_FAILED",
            Self::AdmissionRefused(_) => "POLY_FORECAST_ADMISSION_REFUSED",
        }
    }

    /// Returns the subsystem-level error kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Config { .. } => "config",
            Self::Feed(_) => "feed",
            Self::RawSource { .. } => "raw_source",
            Self::Encode(_) => "encode",
            Self::Schema(_) => "schema",
            Self::AdmissionRefused(_) => "forecast_admission",
            Self::AgentSecret { .. } => "agent_secret",
            Self::AgentArtifact { .. } => "agent_artifact",
            Self::AgentReproduction { .. } => "agent_reproduction",
            Self::AgentLaunch { .. } => "agent_launch",
            Self::Backtest { .. } => "backtest",
            Self::KernelRecall { .. } => "kernel_recall",
            Self::FileSizeLint { .. } => "file_size_lint",
            Self::SnapshotIdentity { .. } => "snapshot_identity",
            Self::Grounding { .. } => "grounding",
            Self::Policy { .. } => "policy",
            Self::Score { .. } => "score",
            Self::StructuredLog { .. } => "structured_log",
            Self::Diagnostics { .. } => "diagnostics",
            Self::Calyx { .. } => "calyx",
        }
    }

    /// Returns the human-readable message without losing the stable code.
    pub fn message(&self) -> String {
        match self {
            Self::Config { message, .. }
            | Self::RawSource { message, .. }
            | Self::AgentSecret { message, .. }
            | Self::AgentArtifact { message, .. }
            | Self::AgentReproduction { message, .. }
            | Self::AgentLaunch { message, .. }
            | Self::Backtest { message, .. }
            | Self::KernelRecall { message, .. }
            | Self::FileSizeLint { message, .. }
            | Self::SnapshotIdentity { message, .. }
            | Self::Grounding { message, .. }
            | Self::Policy { message, .. }
            | Self::Score { message, .. }
            | Self::StructuredLog { message, .. }
            | Self::Diagnostics { message, .. }
            | Self::Calyx { message, .. } => message.clone(),
            Self::Feed(message)
            | Self::Encode(message)
            | Self::Schema(message)
            | Self::AdmissionRefused(message) => message.clone(),
        }
    }

    /// Returns a complete diagnostic record suitable for structured logs and FSV readback.
    pub fn diagnostic(&self) -> PolyErrorDiagnostic {
        PolyErrorDiagnostic {
            kind: self.kind().to_string(),
            code: self.code().to_string(),
            message: self.message(),
            what_failed: self.what_failed().to_string(),
            how_to_fix: self.how_to_fix().to_string(),
        }
    }

    fn what_failed(&self) -> &'static str {
        match self {
            Self::Config { .. } => "configuration loading or validation",
            Self::Feed(_) => "read-only market data feed ingestion",
            Self::RawSource { .. } => "raw Polymarket source inventory and sample-corpus capture",
            Self::Encode(_) => "deterministic signal encoding",
            Self::Schema(_) => "record schema validation",
            Self::AdmissionRefused(_) => "local forecast admission",
            Self::AgentSecret { .. } => "Infisical-backed forecast-agent secret retrieval",
            Self::AgentArtifact { .. } => "forecast-agent artifact parsing or persistence",
            Self::AgentReproduction { .. } => "forecast-agent artifact reproduction",
            Self::AgentLaunch { .. } => "Calyx-controlled forecast-agent launch",
            Self::Backtest { .. } => "forecast backtest validation",
            Self::KernelRecall { .. } => "kernel recall gate validation",
            Self::FileSizeLint { .. } => "local Rust file-size lint gate",
            Self::SnapshotIdentity { .. } => "canonical snapshot identity content-addressing",
            Self::Grounding { .. } => "market grounding and provenance write",
            Self::Policy { .. } => "local-only no-trading policy enforcement",
            Self::Score { .. } => "forecast outcome scoring persistence",
            Self::StructuredLog { .. } => "structured diagnostic log persistence",
            Self::Diagnostics { .. } => {
                "panel higher-order information diagnostics or association-materialization gating"
            }
            Self::Calyx { .. } => "underlying Calyx engine operation",
        }
    }

    fn how_to_fix(&self) -> &'static str {
        match self {
            Self::Config { .. } => {
                "fix the Poly config file or explicit POLY_CONFIG_* override named in the message, then re-run validation"
            }
            Self::Feed(_) => {
                "inspect the feed URL, payload schema, and captured source snapshot; retry only after the read-only source is healthy"
            }
            Self::RawSource { .. } => {
                "inspect the named public endpoint, raw body, metadata, and readback hashes before deriving database schema"
            }
            Self::Encode(_) => {
                "reject or normalize non-finite/out-of-domain numeric input before invoking the frozen encoder"
            }
            Self::Schema(_) => {
                "fix the record to satisfy the persisted schema before writing it to a vault, file, or ledger"
            }
            Self::AdmissionRefused(_) => {
                "persist the refusal and collect stronger grounding evidence before attempting forecast admission again"
            }
            Self::AgentSecret { .. } => {
                "verify the Infisical project, environment, path, and secret names with the strict verifier; do not use plaintext fallbacks"
            }
            Self::AgentArtifact { .. } => {
                "fix the prompt, response, parsed forecast, or artifact path named in the message, then read back the persisted files"
            }
            Self::AgentReproduction { .. } => {
                "compare the manifest hashes against the physical artifacts and regenerate only from the recorded source snapshots"
            }
            Self::AgentLaunch { .. } => {
                "inspect the launch request, DeepSeek response contract, and ledger write named in the message before relaunching"
            }
            Self::Backtest { .. } => {
                "repair the backtest observation set or threshold named in the message and rerun persisted report readback"
            }
            Self::KernelRecall { .. } => {
                "fix the per-domain corpus, frozen lens output, or threshold named in the message before trusting forecasts"
            }
            Self::FileSizeLint { .. } => {
                "inspect the failing root or Rust file named in the lint report, fix it, and read back a fresh report"
            }
            Self::SnapshotIdentity { .. } => {
                "reject or normalize the non-finite/unserializable snapshot field named in the message before content-addressing; never fall back to empty identity bytes"
            }
            Self::Grounding { .. } => {
                "verify the market snapshot, resolution anchor, and local ledger/vault write before proceeding"
            }
            Self::Policy { .. } => {
                "remove the forbidden trading/order action and keep the operation local-only before retrying"
            }
            Self::Score { .. } => {
                "fix the forecast artifact, resolved outcome, or score manifest named in the message and read back the persisted score"
            }
            Self::StructuredLog { .. } => {
                "fix the log event fields or writable JSONL path named in the message, then read back the log file"
            }
            Self::Diagnostics { .. } => {
                "supply the required paired-sample floor and finite slot/outcome series named in the message; below floor a diagnostic is Provisional, never a confident emission, and non-finite input fails closed"
            }
            Self::Calyx { .. } => {
                "inspect the wrapped Calyx code/message and verify the affected local source of truth before retrying"
            }
        }
    }
}

/// Stable diagnostic payload carried into structured logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolyErrorDiagnostic {
    pub kind: String,
    pub code: String,
    pub message: String,
    pub what_failed: String,
    pub how_to_fix: String,
}

impl From<CalyxError> for PolyError {
    fn from(err: CalyxError) -> Self {
        PolyError::Calyx {
            code: err.code.to_string(),
            message: err.message,
        }
    }
}
