//! Grounded-resolution feedback controller (issue #233).
//!
//! This is an orchestration layer: it joins resolved markets to pending forecasts, writes score
//! artifacts, preflights proxy→resolved backfills, runs the selected learning/adaptation reports,
//! and appends the meta-learning ledger. It deliberately reuses the existing primitives instead of
//! reimplementing scoring or tuning math.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{Anchor, Clock};
use calyx_ledger::{LedgerAppender, LedgerCfStore};
use serde::{Deserialize, Serialize};

use crate::blend_relearning::{
    BlendRelearningReport, BlendRelearningRequest, run_blend_relearning,
};
use crate::calibration_refit::{
    CalibrationRefitReport, CalibrationRefitRequest, run_calibration_refit,
};
use crate::diagnostics_store::{read_json, write_json};
use crate::grounding::{TrustTransition, promote_on_resolution};
use crate::meta_learning_ledger::{
    META_LEARNING_LEDGER_FILE, MetaLearningEffect, MetaLearningLedgerEntry,
    MetaLearningLedgerRequest, append_meta_learning_ledger_entry,
    read_meta_learning_ledger_entries,
};
use crate::model::Resolution;
use crate::pending_forecast_register::{
    PendingForecastLedgerStore, PendingForecastRegister, ResolutionJoinResult,
    join_resolution_to_pending_forecasts,
};
use crate::score::{ForecastScoreManifest, ForecastScoreRequest, write_forecast_score_artifacts};
use crate::self_evolution_guardrails::{
    SelfEvolutionGuardrailReport, SelfEvolutionGuardrailRequest, SelfEvolutionStatus,
    require_self_evolution_approved, run_self_evolution_guardrail,
};
use crate::{PolyError, Result};

pub const FEEDBACK_CONTROLLER_SCHEMA_VERSION: &str = "poly.feedback_controller.v1";
pub const FEEDBACK_CONTROLLER_ARTIFACT_KIND: &str = "poly_feedback_controller_cycle";
pub const FEEDBACK_CONTROLLER_REPORT_FILE: &str = "feedback_controller_report.json";

pub const ERR_FEEDBACK_INVALID_REQUEST: &str = "CALYX_POLY_FEEDBACK_INVALID_REQUEST";
pub const ERR_FEEDBACK_MISSING_SCORE: &str = "CALYX_POLY_FEEDBACK_MISSING_SCORE_REQUEST";
pub const ERR_FEEDBACK_SCORE_MISMATCH: &str = "CALYX_POLY_FEEDBACK_SCORE_MISMATCH";
pub const ERR_FEEDBACK_RESOLUTION_NOT_FINAL: &str = "CALYX_POLY_FEEDBACK_RESOLUTION_NOT_FINAL";
pub const ERR_FEEDBACK_READBACK_MISMATCH: &str = "CALYX_POLY_FEEDBACK_READBACK_MISMATCH";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackResolutionInput {
    pub resolution: Resolution,
    pub voided: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackBackfillInput {
    pub name: String,
    pub proxy: Anchor,
    pub resolved: Anchor,
}

pub struct FeedbackLearningRequest<'a> {
    pub blend: BlendRelearningRequest<'a>,
    pub calibration: CalibrationRefitRequest<'a>,
    pub guardrail: SelfEvolutionGuardrailRequest<'a>,
    pub meta: FeedbackMetaLearningRequest<'a>,
}

pub struct FeedbackMetaLearningRequest<'a> {
    pub ledger_dir: &'a Path,
    pub change_id: &'a str,
    pub changed_surface: &'a str,
    pub rationale: &'a str,
    pub responsible_actor: &'a str,
    pub effect: MetaLearningEffect,
    pub rollback_artifact_path: &'a Path,
    pub fsv_artifact_path: &'a Path,
}

pub struct FeedbackControllerCycleRequest<'a> {
    pub cycle_id: &'a str,
    pub report_dir: &'a Path,
    pub score_root: &'a Path,
    pub resolutions: Vec<FeedbackResolutionInput>,
    pub score_requests: Vec<ForecastScoreRequest>,
    pub backfills: Vec<FeedbackBackfillInput>,
    pub learning: Option<FeedbackLearningRequest<'a>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackBackfillResult {
    pub name: String,
    pub transition: TrustTransition,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackLearningResult {
    pub blend_report: BlendRelearningReport,
    pub calibration_report: CalibrationRefitReport,
    pub guardrail_report: SelfEvolutionGuardrailReport,
    pub promoted: bool,
    pub rejection_code: Option<String>,
    pub meta_learning_appended: bool,
    pub meta_learning_entry: Option<MetaLearningLedgerEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackControllerReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub cycle_id: String,
    pub join_results: Vec<ResolutionJoinResult>,
    pub score_manifests: Vec<ForecastScoreManifest>,
    pub skipped_existing_score_ids: Vec<String>,
    pub backfills: Vec<FeedbackBackfillResult>,
    pub learning: Option<FeedbackLearningResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackControllerRun {
    pub report_path: PathBuf,
    pub report: FeedbackControllerReport,
}

pub fn run_feedback_controller_cycle<P, S, C>(
    request: &FeedbackControllerCycleRequest<'_>,
    pending_store: &P,
    register: &mut PendingForecastRegister,
    score_ledger: &mut LedgerAppender<S, C>,
) -> Result<FeedbackControllerRun>
where
    P: PendingForecastLedgerStore,
    S: LedgerCfStore,
    C: Clock,
{
    validate_cycle_request(request)?;
    let score_by_forecast = score_request_map(&request.score_requests)?;
    let mut join_results = Vec::new();
    for input in &request.resolutions {
        if input.resolution.disputed && !input.voided {
            return Err(PolyError::diagnostics(
                ERR_FEEDBACK_RESOLUTION_NOT_FINAL,
                "disputed resolutions must not drive score/backfill until finalized",
            ));
        }
        join_results.push(join_resolution_to_pending_forecasts(
            pending_store,
            register,
            &input.resolution,
            input.voided,
        )?);
    }

    let scored_work = join_results
        .iter()
        .flat_map(|join| {
            join.work_items
                .iter()
                .filter(move |item| !join.voided && item.actual_win.is_some())
        })
        .collect::<Vec<_>>();
    let has_scored_work = !scored_work.is_empty();
    let backfills = if has_scored_work {
        preflight_backfills(&request.backfills)?
    } else {
        Vec::new()
    };

    let mut score_manifests = Vec::new();
    let mut skipped = Vec::new();
    for work in scored_work {
        let score_request = score_by_forecast
            .get(work.forecast_id.as_str())
            .ok_or_else(|| {
                PolyError::diagnostics(
                    ERR_FEEDBACK_MISSING_SCORE,
                    format!("no score request supplied for {}", work.forecast_id),
                )
            })?;
        validate_score_matches_work(score_request, work.actual_win.expect("scored work"))?;
        if score_artifact_exists(request.score_root, &score_request.score_id) {
            skipped.push(score_request.score_id.clone());
        } else {
            score_manifests.push(write_forecast_score_artifacts(
                request.score_root,
                score_ledger,
                score_request,
            )?);
        }
    }

    let learning = match (&request.learning, has_scored_work) {
        (Some(learning), true) => Some(run_learning(learning)?),
        _ => None,
    };
    let report = FeedbackControllerReport {
        schema_version: FEEDBACK_CONTROLLER_SCHEMA_VERSION.to_string(),
        artifact_kind: FEEDBACK_CONTROLLER_ARTIFACT_KIND.to_string(),
        cycle_id: request.cycle_id.to_string(),
        join_results,
        score_manifests,
        skipped_existing_score_ids: skipped,
        backfills,
        learning,
    };
    let report_path = write_json(request.report_dir, FEEDBACK_CONTROLLER_REPORT_FILE, &report)?;
    let expected = serde_json::to_vec_pretty(&report).map_err(|err| {
        PolyError::diagnostics(
            ERR_FEEDBACK_READBACK_MISMATCH,
            format!("encode feedback report for readback check: {err}"),
        )
    })?;
    let actual = fs::read(&report_path).map_err(|err| {
        PolyError::diagnostics(
            ERR_FEEDBACK_READBACK_MISMATCH,
            format!("read feedback report {}: {err}", report_path.display()),
        )
    })?;
    if actual != expected {
        return Err(PolyError::diagnostics(
            ERR_FEEDBACK_READBACK_MISMATCH,
            format!(
                "feedback report changed during readback from {}",
                report_path.display()
            ),
        ));
    }
    Ok(FeedbackControllerRun {
        report_path,
        report,
    })
}

pub fn read_feedback_controller_report(path: &Path) -> Result<FeedbackControllerReport> {
    read_json(path)
}

fn validate_cycle_request(request: &FeedbackControllerCycleRequest<'_>) -> Result<()> {
    if request.cycle_id.trim().is_empty() || request.resolutions.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_FEEDBACK_INVALID_REQUEST,
            "cycle_id and at least one resolution are required",
        ));
    }
    Ok(())
}

fn score_request_map(
    requests: &[ForecastScoreRequest],
) -> Result<BTreeMap<&str, &ForecastScoreRequest>> {
    let mut map = BTreeMap::new();
    for request in requests {
        if map.insert(request.forecast_id.as_str(), request).is_some() {
            return Err(PolyError::diagnostics(
                ERR_FEEDBACK_INVALID_REQUEST,
                format!("duplicate score request for {}", request.forecast_id),
            ));
        }
    }
    Ok(map)
}

fn validate_score_matches_work(request: &ForecastScoreRequest, actual_win: bool) -> Result<()> {
    if request.outcome.actual_win != actual_win {
        return Err(PolyError::diagnostics(
            ERR_FEEDBACK_SCORE_MISMATCH,
            format!(
                "score request {} actual_win={} does not match joined outcome {actual_win}",
                request.score_id, request.outcome.actual_win
            ),
        ));
    }
    Ok(())
}

fn preflight_backfills(backfills: &[FeedbackBackfillInput]) -> Result<Vec<FeedbackBackfillResult>> {
    backfills
        .iter()
        .map(|input| {
            Ok(FeedbackBackfillResult {
                name: input.name.clone(),
                transition: promote_on_resolution(&input.proxy, &input.resolved)?,
            })
        })
        .collect()
}

fn run_learning(request: &FeedbackLearningRequest<'_>) -> Result<FeedbackLearningResult> {
    let blend = run_blend_relearning(&request.blend)?.report;
    let calibration = run_calibration_refit(&request.calibration)?.report;
    let guardrail_run = run_self_evolution_guardrail(&request.guardrail)?;
    let promoted = guardrail_run.report.status == SelfEvolutionStatus::Approved
        && require_self_evolution_approved(&guardrail_run.report).is_ok();
    let rejection_code = (!promoted).then(|| {
        require_self_evolution_approved(&guardrail_run.report)
            .unwrap_err()
            .code()
            .to_string()
    });
    let (meta_learning_appended, meta_learning_entry) =
        append_or_reuse_meta_entry(&request.meta, &guardrail_run.report_path)?;
    Ok(FeedbackLearningResult {
        blend_report: blend,
        calibration_report: calibration,
        guardrail_report: guardrail_run.report,
        promoted,
        rejection_code,
        meta_learning_appended,
        meta_learning_entry,
    })
}

fn append_or_reuse_meta_entry(
    request: &FeedbackMetaLearningRequest<'_>,
    guardrail_report_path: &Path,
) -> Result<(bool, Option<MetaLearningLedgerEntry>)> {
    let ledger_path = request.ledger_dir.join(META_LEARNING_LEDGER_FILE);
    let existing = read_meta_learning_ledger_entries(&ledger_path)?;
    if let Some(entry) = existing
        .into_iter()
        .find(|entry| entry.change_id == request.change_id)
    {
        return Ok((false, Some(entry)));
    }
    let run = append_meta_learning_ledger_entry(&MetaLearningLedgerRequest {
        ledger_dir: request.ledger_dir,
        change_id: request.change_id,
        changed_surface: request.changed_surface,
        rationale: request.rationale,
        responsible_actor: request.responsible_actor,
        effect: request.effect,
        guardrail_report_path,
        rollback_artifact_path: request.rollback_artifact_path,
        fsv_artifact_path: request.fsv_artifact_path,
    })?;
    Ok((true, Some(run.appended)))
}

fn score_artifact_exists(root: &Path, score_id: &str) -> bool {
    root.join(score_id).join("manifest.json").exists()
}
