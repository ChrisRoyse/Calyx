use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const PARAMETER_ADAPTATION_SCHEMA_VERSION: &str = "poly.parameter_adaptation.v1";
pub const PARAMETER_ADAPTATION_ARTIFACT_KIND: &str = "poly_parameter_adaptation";
pub const PARAMETER_ADAPTATION_REPORT_FILE: &str = "parameter_adaptation_report.json";
pub const PARAMETER_ADAPTATION_LEDGER_FILE: &str = "parameter_adaptation_ledger.jsonl";
pub const PARAMETER_ADAPTATION_MIN_ROWS: usize = 8;

pub const ERR_PARAMETER_ADAPTATION_INVALID_REQUEST: &str =
    "CALYX_POLY_PARAMETER_ADAPTATION_INVALID_REQUEST";
pub const ERR_PARAMETER_ADAPTATION_INSUFFICIENT_DATA: &str =
    "CALYX_POLY_PARAMETER_ADAPTATION_INSUFFICIENT_DATA";
pub const ERR_PARAMETER_ADAPTATION_MALFORMED_ROW: &str =
    "CALYX_POLY_PARAMETER_ADAPTATION_MALFORMED_ROW";
pub const ERR_PARAMETER_ADAPTATION_LOOKAHEAD: &str = "CALYX_POLY_PARAMETER_ADAPTATION_LOOKAHEAD";
pub const ERR_PARAMETER_ADAPTATION_MISSING_ARTIFACT: &str =
    "CALYX_POLY_PARAMETER_ADAPTATION_MISSING_ARTIFACT";
pub const ERR_PARAMETER_ADAPTATION_DEGENERATE: &str = "CALYX_POLY_PARAMETER_ADAPTATION_DEGENERATE";
pub const ERR_PARAMETER_ADAPTATION_LEDGER_IO: &str = "CALYX_POLY_PARAMETER_ADAPTATION_LEDGER_IO";
pub const ERR_PARAMETER_ADAPTATION_LEDGER_DECODE: &str =
    "CALYX_POLY_PARAMETER_ADAPTATION_LEDGER_DECODE";
pub const ERR_PARAMETER_ADAPTATION_READBACK_MISMATCH: &str =
    "CALYX_POLY_PARAMETER_ADAPTATION_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterAdaptationArtifactRef {
    pub path: String,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterObservation {
    pub ts: u64,
    pub scalar_value: f64,
    pub heavy_tail_value: f64,
    pub lag_signal: f64,
    pub outcome_yes: bool,
    pub knn_vector: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterSetSnapshot {
    pub version: String,
    pub encoder_sigma: f64,
    pub quantile_edges: Vec<f64>,
    pub te_lag: usize,
    pub knn_k: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterAdaptationSchedule {
    pub previous_run_ts: u64,
    pub scheduled_at_ts: u64,
    pub min_rows: usize,
    pub min_new_rows: usize,
    pub max_te_lag: usize,
    pub candidate_knn_k: Vec<usize>,
    pub min_brier_improvement: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterAdaptationRequest {
    pub domain: String,
    pub horizon_bucket: String,
    pub observations_artifact: ParameterAdaptationArtifactRef,
    pub rollback_artifact: ParameterAdaptationArtifactRef,
    pub ledger_dir: String,
    pub current: ParameterSetSnapshot,
    pub schedule: ParameterAdaptationSchedule,
    pub observations: Vec<ParameterObservation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterAdaptationStatus {
    Promoted,
    NoChange,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterAdaptationMetrics {
    pub current_knn_brier: f64,
    pub selected_knn_brier: f64,
    pub brier_improvement: f64,
    pub selected_te_score: f64,
    pub selected_sigma: f64,
    pub selected_knn_k: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterAdaptationLedgerEntry {
    pub schema_version: String,
    pub sequence: u64,
    pub domain: String,
    pub horizon_bucket: String,
    pub previous_version: String,
    pub new_version: String,
    pub changed_parameters: Vec<String>,
    pub observations_hash: String,
    pub rollback_hash: String,
    pub report_hash: String,
    pub scheduled_at_ts: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParameterAdaptationReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub horizon_bucket: String,
    pub status: ParameterAdaptationStatus,
    pub reason: String,
    pub observation_count: usize,
    pub new_observation_count: usize,
    pub previous: ParameterSetSnapshot,
    pub proposed: ParameterSetSnapshot,
    pub metrics: ParameterAdaptationMetrics,
    pub changed_parameters: Vec<String>,
    pub observations_artifact: ParameterAdaptationArtifactRef,
    pub rollback_artifact: ParameterAdaptationArtifactRef,
    pub ledger_entry: Option<ParameterAdaptationLedgerEntry>,
    pub report_hash: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParameterAdaptationRun {
    pub report_path: PathBuf,
    pub ledger_path: PathBuf,
    pub report: ParameterAdaptationReport,
    pub ledger_entries: Vec<ParameterAdaptationLedgerEntry>,
}
