use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::crypto_ingestor::CryptoIngestorConfig;
use crate::pending_forecast_register::{PendingForecastEntry, ResolutionJoinResult};

pub const CRYPTO_CAPTURE_HARNESS_SCHEMA_VERSION: &str = "poly.crypto_capture_harness.v1";
pub const CRYPTO_CAPTURE_STATE_FILE: &str = "crypto-capture-harness-state.json";
pub const CRYPTO_CAPTURE_REPORT_FILE: &str = "crypto-capture-harness-report.json";
pub const CRYPTO_PRE_RESOLUTION_CORPUS_FILE: &str = "crypto-pre-resolution-corpus.json";
pub const ERR_CRYPTO_CAPTURE_INVALID_CONFIG: &str = "CALYX_POLY_CRYPTO_CAPTURE_INVALID_CONFIG";
pub const ERR_CRYPTO_CAPTURE_READBACK: &str = "CALYX_POLY_CRYPTO_CAPTURE_READBACK";
pub const ERR_CRYPTO_CAPTURE_PENDING_ENTRY: &str = "CALYX_POLY_CRYPTO_CAPTURE_PENDING_ENTRY";
pub const ERR_CRYPTO_CAPTURE_LOOKAHEAD: &str = "CALYX_POLY_CRYPTO_CAPTURE_LOOKAHEAD";
pub const ERR_CRYPTO_CAPTURE_NO_MATURED_PAIR: &str = "CALYX_POLY_CRYPTO_CAPTURE_NO_MATURED_PAIR";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoCaptureHarnessConfig {
    pub interval_secs: u64,
    pub ingestor_config: CryptoIngestorConfig,
}

impl Default for CryptoCaptureHarnessConfig {
    fn default() -> Self {
        Self {
            interval_secs: 60,
            ingestor_config: CryptoIngestorConfig::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CryptoCaptureHarnessRequest<'a> {
    pub vault_id: calyx_core::VaultId,
    pub vault_salt: &'a [u8],
    pub output_root: &'a Path,
    pub config: CryptoCaptureHarnessConfig,
    pub now_ts: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CryptoCaptureDecisionKind {
    Captured,
    SkippedDuplicateInterval,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoCapturedSnapshotRef {
    pub cx_id: String,
    pub token_id: String,
    pub forecast_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forecast_artifact_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forecast_artifact_blake3: Option<String>,
    pub outcome_index: u32,
    pub forecast_ts: u64,
    pub pending_entry: PendingForecastEntry,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoCaptureRecord {
    pub capture_id: String,
    pub due_slot: u64,
    pub captured_ts: u64,
    pub market_id: String,
    pub condition_id: String,
    pub token_count: usize,
    pub run_hash_blake3: String,
    pub snapshots: Vec<CryptoCapturedSnapshotRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoPreResolutionPair {
    pub condition_id: String,
    pub token_id: String,
    pub outcome_index: u32,
    pub snapshot_cx_id: String,
    pub forecast_id: String,
    pub forecast_ts: u64,
    pub p_model: f64,
    pub confidence: f64,
    pub resolution_id: String,
    pub resolved_ts: u64,
    pub actual_win: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoMaturedResolutionRecord {
    pub resolution_id: String,
    pub condition_id: String,
    pub resolved_ts: u64,
    pub voided: bool,
    pub idempotent_replay: bool,
    pub work_item_count: usize,
    pub join_ledger_seq: Option<u64>,
    pub pairs: Vec<CryptoPreResolutionPair>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoCaptureHarnessState {
    pub schema_version: String,
    pub domain: String,
    pub interval_secs: u64,
    pub captures: Vec<CryptoCaptureRecord>,
    pub matured_resolutions: Vec<CryptoMaturedResolutionRecord>,
}

impl Default for CryptoCaptureHarnessState {
    fn default() -> Self {
        Self {
            schema_version: CRYPTO_CAPTURE_HARNESS_SCHEMA_VERSION.to_string(),
            domain: "crypto".to_string(),
            interval_secs: 0,
            captures: Vec::new(),
            matured_resolutions: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoCaptureHarnessReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub state_path: String,
    pub decision: CryptoCaptureDecisionKind,
    pub due_slot: u64,
    pub captured_record: Option<CryptoCaptureRecord>,
    pub capture_count_after: usize,
    pub matured_pair_count_after: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CryptoCaptureHarnessRun {
    pub state_path: PathBuf,
    pub report_path: PathBuf,
    pub state: CryptoCaptureHarnessState,
    pub report: CryptoCaptureHarnessReport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CryptoCaptureResolutionRun {
    pub state_path: PathBuf,
    pub corpus_path: PathBuf,
    pub state: CryptoCaptureHarnessState,
    pub join: ResolutionJoinResult,
    pub record: CryptoMaturedResolutionRecord,
}
