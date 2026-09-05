//! Local forecast strategy learning from held-out score history (issue #108).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

pub const STRATEGY_LEARNING_SCHEMA_VERSION: &str = "poly.strategy_learning.v1";
pub const STRATEGY_LEARNING_ARTIFACT_KIND: &str = "poly_strategy_learning";
pub const STRATEGY_LEARNING_REPORT_FILE: &str = "strategy_learning_report.json";
pub const STRATEGY_LEARNING_MIN_HELDOUT_ROWS: usize = 4;

pub const ERR_STRATEGY_LEARNING_INVALID_REQUEST: &str =
    "CALYX_POLY_STRATEGY_LEARNING_INVALID_REQUEST";
pub const ERR_STRATEGY_LEARNING_LOOKAHEAD: &str = "CALYX_POLY_STRATEGY_LEARNING_LOOKAHEAD";
pub const ERR_STRATEGY_LEARNING_FORBIDDEN_OBJECTIVE: &str =
    "CALYX_POLY_STRATEGY_LEARNING_FORBIDDEN_OBJECTIVE";
pub const ERR_STRATEGY_LEARNING_NO_PROMOTION: &str = "CALYX_POLY_STRATEGY_LEARNING_NO_PROMOTION";
pub const ERR_STRATEGY_LEARNING_READBACK_MISMATCH: &str =
    "CALYX_POLY_STRATEGY_LEARNING_READBACK_MISMATCH";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyChangeKind {
    Lens,
    Association,
    Prompt,
    CalibrationFeature,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrategyComponentChange {
    pub kind: StrategyChangeKind,
    pub key: String,
    pub expected_metric: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrategyCandidateArtifact {
    pub candidate_id: String,
    pub artifact_version: String,
    pub artifact_path: String,
    pub artifact_hash: String,
    pub rollback_artifact_path: String,
    pub rollback_hash: String,
    pub created_at: u64,
    pub effective_at: u64,
    pub provenance: Vec<String>,
    pub objective_notes: Vec<String>,
    pub components: Vec<StrategyComponentChange>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrategyScoreRow {
    pub forecast_id: String,
    pub forecast_ts: u64,
    pub resolved_ts: u64,
    pub scored_ts: u64,
    pub outcome: bool,
    pub baseline_p: f64,
    pub candidate_p: f64,
    pub baseline_sufficiency_bits: f64,
    pub candidate_sufficiency_bits: f64,
    pub baseline_recall_ratio: f64,
    pub candidate_recall_ratio: f64,
    pub baseline_attribution_bits: f64,
    pub candidate_attribution_bits: f64,
    pub baseline_drift_score: f64,
    pub candidate_drift_score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrategyLearningRequest {
    pub domain: String,
    pub horizon_bucket: String,
    pub score_history_artifact: String,
    pub candidate: StrategyCandidateArtifact,
    pub heldout_rows: Vec<StrategyScoreRow>,
    pub min_heldout_rows: usize,
    pub min_brier_improvement: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyLearningStatus {
    Promoted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrategyMetricDelta {
    pub metric: String,
    pub baseline: f64,
    pub candidate: f64,
    pub improvement: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrategyLearningReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub horizon_bucket: String,
    pub score_history_artifact: String,
    pub candidate: StrategyCandidateArtifact,
    pub heldout_count: usize,
    pub positive_count: usize,
    pub status: StrategyLearningStatus,
    pub promotion_code: String,
    pub metric_deltas: Vec<StrategyMetricDelta>,
    pub promoted_change_hash: Option<String>,
    pub report_hash: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrategyLearningRun {
    pub report_path: PathBuf,
    pub report: StrategyLearningReport,
}

pub fn run_strategy_learning_report(
    request: &StrategyLearningRequest,
    output_root: &Path,
) -> Result<StrategyLearningRun> {
    let report = compute_strategy_learning_report(request)?;
    let report_path = write_strategy_learning_report(output_root, &report)?;
    let readback = read_strategy_learning_report(&report_path)?;
    if serde_json::to_value(&readback).ok() != serde_json::to_value(&report).ok() {
        return Err(PolyError::diagnostics(
            ERR_STRATEGY_LEARNING_READBACK_MISMATCH,
            format!(
                "strategy learning report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(StrategyLearningRun {
        report_path,
        report: readback,
    })
}

pub fn compute_strategy_learning_report(
    request: &StrategyLearningRequest,
) -> Result<StrategyLearningReport> {
    validate_request(request)?;
    let deltas = metric_deltas(&request.heldout_rows);
    let (status, code) = promotion_status(request, &deltas);
    let promoted_change_hash = (status == StrategyLearningStatus::Promoted)
        .then(|| promoted_change_hash(request, &deltas));
    let report_hash = report_hash(request, &deltas, status);
    let positive_count = request
        .heldout_rows
        .iter()
        .filter(|row| row.outcome)
        .count();

    Ok(StrategyLearningReport {
        schema_version: STRATEGY_LEARNING_SCHEMA_VERSION.to_string(),
        artifact_kind: STRATEGY_LEARNING_ARTIFACT_KIND.to_string(),
        domain: request.domain.clone(),
        horizon_bucket: request.horizon_bucket.clone(),
        score_history_artifact: request.score_history_artifact.clone(),
        candidate: request.candidate.clone(),
        heldout_count: request.heldout_rows.len(),
        positive_count,
        status,
        promotion_code: code.to_string(),
        metric_deltas: deltas,
        promoted_change_hash,
        report_hash,
    })
}

pub fn require_strategy_learning_promoted(report: &StrategyLearningReport) -> Result<()> {
    if report.status != StrategyLearningStatus::Promoted {
        return Err(PolyError::diagnostics(
            ERR_STRATEGY_LEARNING_NO_PROMOTION,
            format!(
                "strategy learning refused promotion: {}",
                report.promotion_code
            ),
        ));
    }
    Ok(())
}

pub fn write_strategy_learning_report(
    dir: &Path,
    report: &StrategyLearningReport,
) -> Result<PathBuf> {
    write_json(dir, STRATEGY_LEARNING_REPORT_FILE, report)
}

pub fn read_strategy_learning_report(path: &Path) -> Result<StrategyLearningReport> {
    read_json(path)
}

fn promotion_status(
    request: &StrategyLearningRequest,
    deltas: &[StrategyMetricDelta],
) -> (StrategyLearningStatus, &'static str) {
    let brier = improvement(deltas, "brier");
    let calibration = improvement(deltas, "calibration_abs_error");
    let sufficiency = improvement(deltas, "sufficiency_bits");
    let recall = improvement(deltas, "recall_ratio");
    let attribution = improvement(deltas, "attribution_bits");
    let drift = improvement(deltas, "drift_score");
    if brier < request.min_brier_improvement {
        return (StrategyLearningStatus::Rejected, "brier_not_improved");
    }
    if calibration < 0.0 {
        return (StrategyLearningStatus::Rejected, "degraded_calibration");
    }
    if sufficiency < 0.0 || recall < 0.0 || attribution < 0.0 || drift < 0.0 {
        return (
            StrategyLearningStatus::Rejected,
            "forecast_quality_metric_regressed",
        );
    }
    (StrategyLearningStatus::Promoted, "promoted")
}

fn metric_deltas(rows: &[StrategyScoreRow]) -> Vec<StrategyMetricDelta> {
    let n = rows.len() as f64;
    let brier_base = rows
        .iter()
        .map(|row| brier(row.baseline_p, row.outcome))
        .sum::<f64>()
        / n;
    let brier_candidate = rows
        .iter()
        .map(|row| brier(row.candidate_p, row.outcome))
        .sum::<f64>()
        / n;
    let calib_base = rows
        .iter()
        .map(|row| abs_error(row.baseline_p, row.outcome))
        .sum::<f64>()
        / n;
    let calib_candidate = rows
        .iter()
        .map(|row| abs_error(row.candidate_p, row.outcome))
        .sum::<f64>()
        / n;
    vec![
        delta("brier", brier_base, brier_candidate, true),
        delta("calibration_abs_error", calib_base, calib_candidate, true),
        delta(
            "sufficiency_bits",
            mean(rows, |row| row.baseline_sufficiency_bits),
            mean(rows, |row| row.candidate_sufficiency_bits),
            false,
        ),
        delta(
            "recall_ratio",
            mean(rows, |row| row.baseline_recall_ratio),
            mean(rows, |row| row.candidate_recall_ratio),
            false,
        ),
        delta(
            "attribution_bits",
            mean(rows, |row| row.baseline_attribution_bits),
            mean(rows, |row| row.candidate_attribution_bits),
            false,
        ),
        delta(
            "drift_score",
            mean(rows, |row| row.baseline_drift_score),
            mean(rows, |row| row.candidate_drift_score),
            true,
        ),
    ]
}

fn delta(
    metric: &str,
    baseline: f64,
    candidate: f64,
    lower_is_better: bool,
) -> StrategyMetricDelta {
    let improvement = if lower_is_better {
        baseline - candidate
    } else {
        candidate - baseline
    };
    StrategyMetricDelta {
        metric: metric.to_string(),
        baseline,
        candidate,
        improvement,
    }
}

fn improvement(deltas: &[StrategyMetricDelta], metric: &str) -> f64 {
    deltas
        .iter()
        .find(|delta| delta.metric == metric)
        .map(|delta| delta.improvement)
        .unwrap_or(f64::NEG_INFINITY)
}

fn validate_request(request: &StrategyLearningRequest) -> Result<()> {
    validate_label("domain", &request.domain)?;
    validate_label("horizon_bucket", &request.horizon_bucket)?;
    validate_label("score_history_artifact", &request.score_history_artifact)?;
    validate_candidate(&request.candidate)?;
    if request.min_heldout_rows < STRATEGY_LEARNING_MIN_HELDOUT_ROWS
        || request.heldout_rows.len() < request.min_heldout_rows
    {
        return invalid(format!(
            "strategy learning needs >= {} held-out rows, got {}",
            request.min_heldout_rows,
            request.heldout_rows.len()
        ));
    }
    if !request.min_brier_improvement.is_finite() || request.min_brier_improvement <= 0.0 {
        return invalid("min_brier_improvement must be finite and positive");
    }
    let positives = request
        .heldout_rows
        .iter()
        .filter(|row| row.outcome)
        .count();
    if positives == 0 || positives == request.heldout_rows.len() {
        return invalid("held-out rows must contain both outcome classes");
    }
    for row in &request.heldout_rows {
        validate_row(row, request.candidate.effective_at)?;
    }
    Ok(())
}

fn validate_candidate(candidate: &StrategyCandidateArtifact) -> Result<()> {
    for (field, value) in [
        ("candidate_id", &candidate.candidate_id),
        ("artifact_version", &candidate.artifact_version),
        ("artifact_path", &candidate.artifact_path),
        ("artifact_hash", &candidate.artifact_hash),
        ("rollback_artifact_path", &candidate.rollback_artifact_path),
        ("rollback_hash", &candidate.rollback_hash),
    ] {
        validate_label(field, value)?;
    }
    if candidate.created_at > candidate.effective_at {
        return invalid("candidate created_at must be <= effective_at");
    }
    if candidate.provenance.is_empty() || candidate.components.is_empty() {
        return invalid("candidate requires provenance and component changes");
    }
    for note in &candidate.objective_notes {
        if forbidden_objective(note) {
            return Err(PolyError::diagnostics(
                ERR_STRATEGY_LEARNING_FORBIDDEN_OBJECTIVE,
                "strategy learning objectives must not mention betting, stake, execution, PnL, or capital growth",
            ));
        }
    }
    for component in &candidate.components {
        validate_label("component.key", &component.key)?;
        validate_label("component.expected_metric", &component.expected_metric)?;
    }
    Ok(())
}

fn validate_row(row: &StrategyScoreRow, candidate_effective_at: u64) -> Result<()> {
    validate_label("forecast_id", &row.forecast_id)?;
    if candidate_effective_at > row.forecast_ts {
        return Err(PolyError::diagnostics(
            ERR_STRATEGY_LEARNING_LOOKAHEAD,
            format!(
                "candidate effective_at {} is after forecast_ts {} for {}",
                candidate_effective_at, row.forecast_ts, row.forecast_id
            ),
        ));
    }
    if row.forecast_ts >= row.resolved_ts || row.resolved_ts > row.scored_ts {
        return Err(PolyError::diagnostics(
            ERR_STRATEGY_LEARNING_LOOKAHEAD,
            format!(
                "forecast/outcome/score timestamps are not causal for {}",
                row.forecast_id
            ),
        ));
    }
    for (name, value) in numeric_fields(row) {
        if !value.is_finite() {
            return invalid(format!("{name} must be finite for {}", row.forecast_id));
        }
    }
    for (name, p) in [
        ("baseline_p", row.baseline_p),
        ("candidate_p", row.candidate_p),
    ] {
        if !(0.0..=1.0).contains(&p) {
            return invalid(format!("{name} must be in [0,1] for {}", row.forecast_id));
        }
    }
    Ok(())
}

fn numeric_fields(row: &StrategyScoreRow) -> [(&'static str, f64); 10] {
    [
        ("baseline_p", row.baseline_p),
        ("candidate_p", row.candidate_p),
        ("baseline_sufficiency_bits", row.baseline_sufficiency_bits),
        ("candidate_sufficiency_bits", row.candidate_sufficiency_bits),
        ("baseline_recall_ratio", row.baseline_recall_ratio),
        ("candidate_recall_ratio", row.candidate_recall_ratio),
        ("baseline_attribution_bits", row.baseline_attribution_bits),
        ("candidate_attribution_bits", row.candidate_attribution_bits),
        ("baseline_drift_score", row.baseline_drift_score),
        ("candidate_drift_score", row.candidate_drift_score),
    ]
}

fn brier(p: f64, outcome: bool) -> f64 {
    let y = if outcome { 1.0 } else { 0.0 };
    (p - y) * (p - y)
}

fn abs_error(p: f64, outcome: bool) -> f64 {
    let y = if outcome { 1.0 } else { 0.0 };
    (p - y).abs()
}

fn mean(rows: &[StrategyScoreRow], f: impl Fn(&StrategyScoreRow) -> f64) -> f64 {
    rows.iter().map(f).sum::<f64>() / rows.len() as f64
}

fn promoted_change_hash(
    request: &StrategyLearningRequest,
    deltas: &[StrategyMetricDelta],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(request.candidate.candidate_id.as_bytes());
    hasher.update(request.candidate.artifact_hash.as_bytes());
    hasher.update(request.candidate.rollback_hash.as_bytes());
    for delta in deltas {
        hasher.update(delta.metric.as_bytes());
        hasher.update(&delta.improvement.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn report_hash(
    request: &StrategyLearningRequest,
    deltas: &[StrategyMetricDelta],
    status: StrategyLearningStatus,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(request.domain.as_bytes());
    hasher.update(request.horizon_bucket.as_bytes());
    hasher.update(format!("{:?}", status).as_bytes());
    for row in &request.heldout_rows {
        hasher.update(row.forecast_id.as_bytes());
        hasher.update(&row.baseline_p.to_le_bytes());
        hasher.update(&row.candidate_p.to_le_bytes());
        hasher.update(&[u8::from(row.outcome)]);
    }
    for delta in deltas {
        hasher.update(delta.metric.as_bytes());
        hasher.update(&delta.baseline.to_le_bytes());
        hasher.update(&delta.candidate.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn forbidden_objective(note: &str) -> bool {
    let lower = note.to_ascii_lowercase();
    [
        "pnl",
        "profit",
        "stake",
        "bet",
        "execution",
        "capital",
        "bankroll",
    ]
    .iter()
    .any(|term| lower.contains(term))
}

fn validate_label(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return invalid(format!("{field} must not be empty"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_STRATEGY_LEARNING_INVALID_REQUEST,
        message.into(),
    ))
}
