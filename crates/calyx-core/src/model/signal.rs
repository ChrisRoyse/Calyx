//! Shared signal, reference, and flag structs.

use serde::{Deserialize, Serialize};

use crate::time::Ts;

/// Confidence interval for an estimated signal.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceInterval {
    /// Lower bound.
    pub low: f32,
    /// Upper bound.
    pub high: f32,
}

/// Assay signal estimate for a slot against an anchor axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    /// Estimated bits above baseline.
    pub bits: f32,
    /// Confidence interval.
    pub ci: ConfidenceInterval,
    /// Effective sample count.
    pub n: usize,
    /// Estimator identifier.
    pub estimator: String,
    /// Timestamp of the estimate.
    pub ts: Ts,
}

/// Input content reference; raw bytes may be absent or redacted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRef {
    /// Hash of the input bytes.
    pub hash: [u8; 32],
    /// Optional pointer to retained bytes.
    pub pointer: Option<String>,
    /// Whether the content was intentionally redacted.
    pub redacted: bool,
}

/// Append-only Ledger reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRef {
    /// Ledger sequence number.
    pub seq: u64,
    /// Hash-chain entry hash.
    pub hash: [u8; 32],
}

/// Constellation-level state flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CxFlags {
    /// No grounded anchor is attached yet.
    pub ungrounded: bool,
    /// At least one measurement path was degraded.
    pub degraded: bool,
    /// Ward marked this as outside a calibrated trusted region.
    pub novel_region: bool,
    /// The input bytes were redacted.
    pub redacted_input: bool,
}
