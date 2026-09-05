//! Online mistake-closure heads for resolved forecast errors (issue #106).

use std::path::{Path, PathBuf};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};
pub use crate::mistake_closure_types::*;

pub fn run_mistake_closure_report(
    request: &MistakeClosureRequest,
    output_root: &Path,
) -> Result<MistakeClosureRun> {
    let report = compute_mistake_closure_report(request)?;
    let report_path = write_mistake_closure_report(output_root, &report)?;
    let readback = read_mistake_closure_report(&report_path)?;
    if serde_json::to_value(&readback).ok() != serde_json::to_value(&report).ok() {
        return Err(PolyError::diagnostics(
            ERR_MISTAKE_CLOSURE_READBACK_MISMATCH,
            format!(
                "mistake closure report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(MistakeClosureRun {
        report_path,
        report: readback,
    })
}

pub fn compute_mistake_closure_report(
    request: &MistakeClosureRequest,
) -> Result<MistakeClosureReport> {
    validate_request(request)?;
    let mistakes = mistake_rows(request);
    let aggregate_effect = measured_effect(&mistakes);
    let evidence = evidence_links(&mistakes);
    let proposals =
        if aggregate_effect.brier_improvement >= request.thresholds.min_brier_improvement {
            build_proposals(request, &mistakes, &evidence, &aggregate_effect)
        } else {
            Vec::new()
        };
    let status = if proposals.is_empty() {
        MistakeClosureStatus::NoProposal
    } else {
        MistakeClosureStatus::Proposed
    };
    let report_hash = report_hash(request, &proposals, &aggregate_effect);
    let artifact_version = format!(
        "{}:{}:{}:{}",
        request.domain,
        request.horizon_bucket,
        request.generated_at,
        &report_hash[..12]
    );

    Ok(MistakeClosureReport {
        schema_version: MISTAKE_CLOSURE_SCHEMA_VERSION.to_string(),
        artifact_kind: MISTAKE_CLOSURE_ARTIFACT_KIND.to_string(),
        artifact_version,
        domain: request.domain.clone(),
        horizon_bucket: request.horizon_bucket.clone(),
        scored_history_artifact: request.scored_history_artifact.clone(),
        source_snapshot_artifact: request.source_snapshot_artifact.clone(),
        outcome_anchor_artifact: request.outcome_anchor_artifact.clone(),
        generated_at: request.generated_at,
        status,
        scored_count: request.rows.len(),
        mistake_count: mistakes.len(),
        proposal_count: proposals.len(),
        aggregate_effect,
        proposals,
        rollback_artifact: request.rollback_artifact.clone(),
        report_hash,
    })
}

pub fn require_mistake_closure_proposed(report: &MistakeClosureReport) -> Result<()> {
    if report.status == MistakeClosureStatus::Proposed && !report.proposals.is_empty() {
        return Ok(());
    }
    Err(PolyError::diagnostics(
        ERR_MISTAKE_CLOSURE_NO_PROPOSAL,
        "mistake closure produced no corrective proposal",
    ))
}

pub fn write_mistake_closure_report(dir: &Path, report: &MistakeClosureReport) -> Result<PathBuf> {
    write_json(dir, MISTAKE_CLOSURE_REPORT_FILE, report)
}

pub fn read_mistake_closure_report(path: &Path) -> Result<MistakeClosureReport> {
    read_json(path)
}

fn build_proposals(
    request: &MistakeClosureRequest,
    mistakes: &[&MistakeClosureScoreRow],
    evidence: &[MistakeClosureEvidenceLink],
    effect: &MistakeClosureEffect,
) -> Vec<MistakeClosureProposal> {
    let mut proposals = Vec::new();
    if mistakes.iter().any(|row| row.missing_evidence_count > 0) {
        proposals.push(proposal(
            request,
            "missing_evidence_lens",
            MistakeClosureHeadKind::Lens,
            "add source-derived lens for the missing pre-forecast evidence family",
            "missing_evidence",
            evidence,
            effect,
        ));
    }
    if mistakes.iter().any(|row| {
        row.weak_association_count > 0
            || row.association_recall_ratio < request.thresholds.min_association_recall_ratio
    }) {
        proposals.push(proposal(
            request,
            "weak_association_repair",
            MistakeClosureHeadKind::Association,
            "promote measured association repair for the weak recalled neighborhood",
            "weak_association",
            evidence,
            effect,
        ));
    }
    if mistakes.iter().any(|row| row.prompt_pattern_count > 0) {
        proposals.push(proposal(
            request,
            "prompt_pattern_repair",
            MistakeClosureHeadKind::Prompt,
            "revise local forecast prompt/output pattern that overstates unsupported confidence",
            "bad_prompt_or_output_pattern",
            evidence,
            effect,
        ));
    }
    if mistakes
        .iter()
        .any(|row| row.calibration_abs_error > request.thresholds.max_calibration_abs_error)
    {
        proposals.push(proposal(
            request,
            "admission_calibration_tighten",
            MistakeClosureHeadKind::Admission,
            "tighten admission for this domain until calibration residuals close",
            "calibration_drift",
            evidence,
            effect,
        ));
    }
    proposals
}

fn proposal(
    request: &MistakeClosureRequest,
    suffix: &str,
    kind: MistakeClosureHeadKind,
    proposed_change: &str,
    trigger: &str,
    evidence: &[MistakeClosureEvidenceLink],
    effect: &MistakeClosureEffect,
) -> MistakeClosureProposal {
    MistakeClosureProposal {
        head_id: format!("{}:{}:{suffix}", request.domain, request.horizon_bucket),
        kind,
        proposed_change: proposed_change.to_string(),
        trigger: trigger.to_string(),
        affected_forecast_ids: evidence
            .iter()
            .map(|item| item.forecast_id.clone())
            .collect(),
        evidence: evidence.to_vec(),
        measured_effect: effect.clone(),
        rollback_artifact_path: request.rollback_artifact.path.clone(),
        rollback_artifact_hash: request.rollback_artifact.blake3.clone(),
    }
}

fn mistake_rows(request: &MistakeClosureRequest) -> Vec<&MistakeClosureScoreRow> {
    request
        .rows
        .iter()
        .filter(|row| {
            let outcome = row.actual_win.expect("validated outcome");
            brier(row.probability, outcome) >= request.thresholds.min_error_brier
                || ((row.probability >= 0.5) != outcome)
        })
        .collect()
}

fn measured_effect(rows: &[&MistakeClosureScoreRow]) -> MistakeClosureEffect {
    if rows.is_empty() {
        return MistakeClosureEffect {
            affected_count: 0,
            baseline_mean_brier: 0.0,
            closure_mean_brier: 0.0,
            brier_improvement: 0.0,
            baseline_calibration_abs_error: 0.0,
            closure_calibration_abs_error: 0.0,
            calibration_abs_error_improvement: 0.0,
            baseline_sufficiency_bits: 0.0,
            closure_sufficiency_bits: 0.0,
            sufficiency_bits_improvement: 0.0,
            baseline_association_recall_ratio: 0.0,
            closure_association_recall_ratio: 0.0,
            association_recall_improvement: 0.0,
        };
    }
    let n = rows.len() as f64;
    let baseline_brier = rows
        .iter()
        .map(|row| brier(row.probability, row.actual_win.expect("validated outcome")))
        .sum::<f64>()
        / n;
    let closure_brier = rows
        .iter()
        .map(|row| {
            brier(
                row.closure_probability,
                row.actual_win.expect("validated outcome"),
            )
        })
        .sum::<f64>()
        / n;
    let baseline_calibration = mean(rows, |row| row.calibration_abs_error);
    let closure_calibration = mean(rows, |row| row.closure_calibration_abs_error);
    let baseline_sufficiency = mean(rows, |row| row.sufficiency_bits);
    let closure_sufficiency = mean(rows, |row| row.closure_sufficiency_bits);
    let baseline_recall = mean(rows, |row| row.association_recall_ratio);
    let closure_recall = mean(rows, |row| row.closure_association_recall_ratio);
    MistakeClosureEffect {
        affected_count: rows.len(),
        baseline_mean_brier: baseline_brier,
        closure_mean_brier: closure_brier,
        brier_improvement: baseline_brier - closure_brier,
        baseline_calibration_abs_error: baseline_calibration,
        closure_calibration_abs_error: closure_calibration,
        calibration_abs_error_improvement: baseline_calibration - closure_calibration,
        baseline_sufficiency_bits: baseline_sufficiency,
        closure_sufficiency_bits: closure_sufficiency,
        sufficiency_bits_improvement: closure_sufficiency - baseline_sufficiency,
        baseline_association_recall_ratio: baseline_recall,
        closure_association_recall_ratio: closure_recall,
        association_recall_improvement: closure_recall - baseline_recall,
    }
}

fn evidence_links(rows: &[&MistakeClosureScoreRow]) -> Vec<MistakeClosureEvidenceLink> {
    rows.iter()
        .map(|row| MistakeClosureEvidenceLink {
            forecast_id: row.forecast_id.clone(),
            forecast_artifact_hash: row.forecast_artifact.blake3.clone(),
            outcome_anchor_hash: row
                .outcome_anchor
                .as_ref()
                .expect("validated outcome")
                .blake3
                .clone(),
            source_snapshot_hash: row.source_snapshot.blake3.clone(),
            score_artifact_hash: row.score_artifact.blake3.clone(),
            prompt_artifact_hash: row.prompt_artifact.blake3.clone(),
        })
        .collect()
}

fn validate_request(request: &MistakeClosureRequest) -> Result<()> {
    for (field, value) in [
        ("domain", &request.domain),
        ("horizon_bucket", &request.horizon_bucket),
        ("scored_history_artifact", &request.scored_history_artifact),
        (
            "source_snapshot_artifact",
            &request.source_snapshot_artifact,
        ),
        ("outcome_anchor_artifact", &request.outcome_anchor_artifact),
    ] {
        validate_label(field, value)?;
        reject_forbidden(field, value)?;
    }
    validate_thresholds(request.thresholds)?;
    validate_artifact("rollback_artifact", &request.rollback_artifact)?;
    if request.rows.len() < request.thresholds.min_sample_size {
        return Err(PolyError::diagnostics(
            ERR_MISTAKE_CLOSURE_INSUFFICIENT_SAMPLE,
            format!(
                "mistake closure needs >= {} scored rows, got {}",
                request.thresholds.min_sample_size,
                request.rows.len()
            ),
        ));
    }
    for row in &request.rows {
        validate_row(row, request.generated_at)?;
    }
    Ok(())
}

fn validate_thresholds(thresholds: MistakeClosureThresholds) -> Result<()> {
    if thresholds.min_sample_size < MISTAKE_CLOSURE_MIN_ROWS {
        return invalid("min_sample_size is below the mistake-closure proof floor");
    }
    for (name, value) in [
        ("min_error_brier", thresholds.min_error_brier),
        ("min_brier_improvement", thresholds.min_brier_improvement),
        (
            "max_calibration_abs_error",
            thresholds.max_calibration_abs_error,
        ),
        (
            "min_association_recall_ratio",
            thresholds.min_association_recall_ratio,
        ),
    ] {
        if !value.is_finite() || value < 0.0 {
            return invalid(format!("{name} must be finite and non-negative"));
        }
    }
    Ok(())
}

fn validate_row(row: &MistakeClosureScoreRow, generated_at: u64) -> Result<()> {
    validate_label("forecast_id", &row.forecast_id)?;
    if row.actual_win.is_none() || row.outcome_anchor.is_none() || row.resolved_ts.is_none() {
        return Err(PolyError::diagnostics(
            ERR_MISTAKE_CLOSURE_MISSING_OUTCOME,
            format!(
                "forecast {} has no resolved outcome anchor",
                row.forecast_id
            ),
        ));
    }
    let resolved_ts = row.resolved_ts.expect("checked above");
    if row.source_snapshot_ts > row.forecast_ts
        || row.forecast_ts >= resolved_ts
        || resolved_ts > row.scored_ts
        || row.scored_ts > generated_at
    {
        return Err(PolyError::diagnostics(
            ERR_MISTAKE_CLOSURE_LOOKAHEAD,
            format!(
                "forecast {} has non-causal source/forecast/outcome/score timing",
                row.forecast_id
            ),
        ));
    }
    for (name, value) in numeric_fields(row) {
        if !value.is_finite() {
            return invalid(format!("{name} must be finite for {}", row.forecast_id));
        }
    }
    for (name, value) in [
        ("probability", row.probability),
        ("closure_probability", row.closure_probability),
        ("association_recall_ratio", row.association_recall_ratio),
        (
            "closure_association_recall_ratio",
            row.closure_association_recall_ratio,
        ),
    ] {
        if !(0.0..=1.0).contains(&value) {
            return invalid(format!("{name} must be in [0,1] for {}", row.forecast_id));
        }
    }
    validate_artifact("forecast_artifact", &row.forecast_artifact)?;
    validate_artifact(
        "outcome_anchor",
        row.outcome_anchor.as_ref().expect("checked above"),
    )?;
    validate_artifact("source_snapshot", &row.source_snapshot)?;
    validate_artifact("score_artifact", &row.score_artifact)?;
    validate_artifact("prompt_artifact", &row.prompt_artifact)
}

fn numeric_fields(row: &MistakeClosureScoreRow) -> [(&'static str, f64); 8] {
    [
        ("probability", row.probability),
        ("closure_probability", row.closure_probability),
        ("sufficiency_bits", row.sufficiency_bits),
        ("closure_sufficiency_bits", row.closure_sufficiency_bits),
        ("association_recall_ratio", row.association_recall_ratio),
        (
            "closure_association_recall_ratio",
            row.closure_association_recall_ratio,
        ),
        ("calibration_abs_error", row.calibration_abs_error),
        (
            "closure_calibration_abs_error",
            row.closure_calibration_abs_error,
        ),
    ]
}

fn validate_artifact(label: &str, artifact: &MistakeClosureArtifactRef) -> Result<()> {
    validate_label(label, &artifact.path)?;
    if artifact.blake3.len() != 64 || !artifact.blake3.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return invalid(format!("{label} BLAKE3 must be 64 hex characters"));
    }
    let bytes = std::fs::read(&artifact.path).map_err(|err| {
        PolyError::diagnostics(
            ERR_MISTAKE_CLOSURE_MISSING_ARTIFACT,
            format!("read {label} {}: {err}", artifact.path),
        )
    })?;
    let observed = blake3::hash(&bytes).to_hex().to_string();
    if observed != artifact.blake3 {
        return Err(PolyError::diagnostics(
            ERR_MISTAKE_CLOSURE_MISSING_ARTIFACT,
            format!("{label} {} hash mismatch", artifact.path),
        ));
    }
    Ok(())
}

fn report_hash(
    request: &MistakeClosureRequest,
    proposals: &[MistakeClosureProposal],
    effect: &MistakeClosureEffect,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(request.domain.as_bytes());
    hasher.update(request.horizon_bucket.as_bytes());
    hasher.update(&request.generated_at.to_le_bytes());
    hasher.update(request.rollback_artifact.blake3.as_bytes());
    hasher.update(&effect.brier_improvement.to_le_bytes());
    for row in &request.rows {
        hasher.update(row.forecast_id.as_bytes());
        hasher.update(row.forecast_artifact.blake3.as_bytes());
        hasher.update(row.score_artifact.blake3.as_bytes());
    }
    for proposal in proposals {
        hasher.update(proposal.head_id.as_bytes());
        hasher.update(format!("{:?}", proposal.kind).as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn mean(rows: &[&MistakeClosureScoreRow], f: impl Fn(&MistakeClosureScoreRow) -> f64) -> f64 {
    rows.iter().map(|row| f(row)).sum::<f64>() / rows.len() as f64
}

fn brier(p: f64, outcome: bool) -> f64 {
    let y = if outcome { 1.0 } else { 0.0 };
    (p - y) * (p - y)
}

fn validate_label(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return invalid(format!("{field} must not be empty"));
    }
    Ok(())
}

fn reject_forbidden(field: &str, value: &str) -> Result<()> {
    let lower = value.to_ascii_lowercase();
    let tokens = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if [
        "pnl",
        "profit",
        "profits",
        "stake",
        "stakes",
        "bet",
        "bets",
        "betting",
        "execution",
        "capital",
        "bankroll",
    ]
    .iter()
    .any(|term| tokens.iter().any(|token| token == term))
    {
        return Err(PolyError::diagnostics(
            ERR_MISTAKE_CLOSURE_FORBIDDEN_SEMANTIC,
            format!("{field} contains forbidden trading semantics"),
        ));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_MISTAKE_CLOSURE_INVALID_REQUEST,
        message.into(),
    ))
}
