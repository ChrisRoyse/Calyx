use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const MISTAKE_CLOSURE_SCHEMA_VERSION: &str = "poly.mistake_closure.v1";
pub const MISTAKE_CLOSURE_ARTIFACT_KIND: &str = "poly_mistake_closure";
pub const MISTAKE_CLOSURE_REPORT_FILE: &str = "mistake_closure_report.json";
pub const MISTAKE_CLOSURE_MIN_ROWS: usize = 4;

pub const ERR_MISTAKE_CLOSURE_INVALID_REQUEST: &str = "CALYX_POLY_MISTAKE_CLOSURE_INVALID_REQUEST";
pub const ERR_MISTAKE_CLOSURE_MISSING_OUTCOME: &str = "CALYX_POLY_MISTAKE_CLOSURE_MISSING_OUTCOME";
pub const ERR_MISTAKE_CLOSURE_INSUFFICIENT_SAMPLE: &str =
    "CALYX_POLY_MISTAKE_CLOSURE_INSUFFICIENT_SAMPLE";
pub const ERR_MISTAKE_CLOSURE_LOOKAHEAD: &str = "CALYX_POLY_MISTAKE_CLOSURE_LOOKAHEAD";
pub const ERR_MISTAKE_CLOSURE_MISSING_ARTIFACT: &str =
    "CALYX_POLY_MISTAKE_CLOSURE_MISSING_ARTIFACT";
pub const ERR_MISTAKE_CLOSURE_FORBIDDEN_SEMANTIC: &str =
    "CALYX_POLY_MISTAKE_CLOSURE_FORBIDDEN_SEMANTIC";
pub const ERR_MISTAKE_CLOSURE_NO_PROPOSAL: &str = "CALYX_POLY_MISTAKE_CLOSURE_NO_PROPOSAL";
pub const ERR_MISTAKE_CLOSURE_READBACK_MISMATCH: &str =
    "CALYX_POLY_MISTAKE_CLOSURE_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeClosureArtifactRef {
    pub path: String,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeClosureScoreRow {
    pub forecast_id: String,
    pub forecast_ts: u64,
    pub resolved_ts: Option<u64>,
    pub scored_ts: u64,
    pub source_snapshot_ts: u64,
    pub actual_win: Option<bool>,
    pub probability: f64,
    pub closure_probability: f64,
    pub sufficiency_bits: f64,
    pub closure_sufficiency_bits: f64,
    pub association_recall_ratio: f64,
    pub closure_association_recall_ratio: f64,
    pub calibration_abs_error: f64,
    pub closure_calibration_abs_error: f64,
    pub missing_evidence_count: usize,
    pub weak_association_count: usize,
    pub prompt_pattern_count: usize,
    pub forecast_artifact: MistakeClosureArtifactRef,
    pub outcome_anchor: Option<MistakeClosureArtifactRef>,
    pub source_snapshot: MistakeClosureArtifactRef,
    pub score_artifact: MistakeClosureArtifactRef,
    pub prompt_artifact: MistakeClosureArtifactRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeClosureThresholds {
    pub min_sample_size: usize,
    pub min_error_brier: f64,
    pub min_brier_improvement: f64,
    pub max_calibration_abs_error: f64,
    pub min_association_recall_ratio: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeClosureRequest {
    pub domain: String,
    pub horizon_bucket: String,
    pub scored_history_artifact: String,
    pub source_snapshot_artifact: String,
    pub outcome_anchor_artifact: String,
    pub generated_at: u64,
    pub rollback_artifact: MistakeClosureArtifactRef,
    pub thresholds: MistakeClosureThresholds,
    pub rows: Vec<MistakeClosureScoreRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MistakeClosureStatus {
    Proposed,
    NoProposal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MistakeClosureHeadKind {
    Lens,
    Association,
    Prompt,
    Admission,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeClosureEffect {
    pub affected_count: usize,
    pub baseline_mean_brier: f64,
    pub closure_mean_brier: f64,
    pub brier_improvement: f64,
    pub baseline_calibration_abs_error: f64,
    pub closure_calibration_abs_error: f64,
    pub calibration_abs_error_improvement: f64,
    pub baseline_sufficiency_bits: f64,
    pub closure_sufficiency_bits: f64,
    pub sufficiency_bits_improvement: f64,
    pub baseline_association_recall_ratio: f64,
    pub closure_association_recall_ratio: f64,
    pub association_recall_improvement: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeClosureEvidenceLink {
    pub forecast_id: String,
    pub forecast_artifact_hash: String,
    pub outcome_anchor_hash: String,
    pub source_snapshot_hash: String,
    pub score_artifact_hash: String,
    pub prompt_artifact_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeClosureProposal {
    pub head_id: String,
    pub kind: MistakeClosureHeadKind,
    pub proposed_change: String,
    pub trigger: String,
    pub affected_forecast_ids: Vec<String>,
    pub evidence: Vec<MistakeClosureEvidenceLink>,
    pub measured_effect: MistakeClosureEffect,
    pub rollback_artifact_path: String,
    pub rollback_artifact_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeClosureReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub artifact_version: String,
    pub domain: String,
    pub horizon_bucket: String,
    pub scored_history_artifact: String,
    pub source_snapshot_artifact: String,
    pub outcome_anchor_artifact: String,
    pub generated_at: u64,
    pub status: MistakeClosureStatus,
    pub scored_count: usize,
    pub mistake_count: usize,
    pub proposal_count: usize,
    pub aggregate_effect: MistakeClosureEffect,
    pub proposals: Vec<MistakeClosureProposal>,
    pub rollback_artifact: MistakeClosureArtifactRef,
    pub report_hash: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MistakeClosureRun {
    pub report_path: PathBuf,
    pub report: MistakeClosureReport,
}
