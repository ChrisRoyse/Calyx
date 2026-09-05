//! Ward conformal calibration provenance for Poly forecast admission (issue #91).
//!
//! This module composes the real `calyx-ward` conformal calibrator and guard, then persists the
//! calibration metadata together with the forecast-admission ledger fields that used it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use calyx_core::{Clock, SlotId};
use calyx_ward::{
    CalibrationInput, CalibrationMeta, GuardId, GuardPolicy, GuardProfile, GuardVerdict,
    NoveltyAction, ProducedSlots, SlotKind, WardError, calibrate, guard,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::admission::{AdmissionInputs, AdmissionParams, evaluate_admission};
use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

pub const POLY_WARD_CALIBRATION_SCHEMA_VERSION: &str = "poly.ward_calibration.v1";
pub const POLY_WARD_CALIBRATION_ARTIFACT_KIND: &str = "poly_ward_calibration";
pub const POLY_WARD_ADMISSION_LEDGER_SCHEMA_VERSION: &str = "poly.ward_admission_ledger.v1";

pub const ERR_WARD_CALIBRATION_INVALID_REQUEST: &str =
    "CALYX_POLY_WARD_CALIBRATION_INVALID_REQUEST";
pub const ERR_WARD_CALIBRATION_INSUFFICIENT_ANCHORS: &str =
    "CALYX_POLY_WARD_CALIBRATION_INSUFFICIENT_ANCHORS";
pub const ERR_WARD_CALIBRATION_STALE: &str = "CALYX_POLY_WARD_CALIBRATION_STALE";
pub const ERR_WARD_CALIBRATION_MALFORMED_RESIDUAL: &str =
    "CALYX_POLY_WARD_CALIBRATION_MALFORMED_RESIDUAL";
pub const ERR_WARD_CALIBRATION_READBACK_MISMATCH: &str =
    "CALYX_POLY_WARD_CALIBRATION_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WardResidualClass {
    KnownGood,
    KnownBad,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WardCalibrationResidual {
    pub slot: SlotId,
    pub class: WardResidualClass,
    pub score: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WardCalibrationRequest {
    pub calibration_version: String,
    pub domain: String,
    pub horizon_bucket: String,
    pub panel_version: u64,
    pub guard_id: String,
    pub slot: SlotId,
    pub slot_kind: SlotKind,
    pub target_far: f32,
    pub alpha: f32,
    pub min_anchor_count: usize,
    pub max_age_seconds: i64,
    pub now_ts: i64,
    pub candidate_score: f32,
    pub residuals: Vec<WardCalibrationResidual>,
    pub admission_params: AdmissionParams,
    pub admission_inputs: AdmissionInputs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WardCalibrationMetaSummary {
    pub corpus_hash: String,
    pub estimator: String,
    pub far: f32,
    pub frr: f32,
    pub confidence: f32,
    pub ts: i64,
    pub per_slot_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WardAdmissionLedger {
    pub schema_version: String,
    pub calibration_version: String,
    pub domain: String,
    pub horizon_bucket: String,
    pub guard_calibrated: bool,
    pub guard_pass: bool,
    pub grounding_anchor_count: u32,
    pub p_win: f64,
    pub confidence: f64,
    pub admitted: bool,
    pub code: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WardCalibrationReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub calibration_version: String,
    pub domain: String,
    pub horizon_bucket: String,
    pub panel_version: u64,
    pub guard_id: String,
    pub slot: SlotId,
    pub slot_kind: SlotKind,
    pub target_far: f32,
    pub alpha: f32,
    pub min_anchor_count: usize,
    pub anchor_count: usize,
    pub good_anchor_count: usize,
    pub bad_anchor_count: usize,
    pub calibration_ts: i64,
    pub stale_after_ts: i64,
    pub now_ts: i64,
    pub stale: bool,
    pub guard_calibrated: bool,
    pub guard_pass: bool,
    pub candidate_score: f32,
    pub residual_evidence_hash: String,
    pub residual_evidence: Vec<WardCalibrationResidual>,
    pub calibration_meta: WardCalibrationMetaSummary,
    pub profile: GuardProfile,
    pub guard_verdict: GuardVerdict,
    pub admission_ledger: WardAdmissionLedger,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WardCalibrationRun {
    pub report_path: PathBuf,
    pub report: WardCalibrationReport,
}

pub fn run_ward_calibration_report(
    request: &WardCalibrationRequest,
    output_root: &Path,
    clock: &dyn Clock,
) -> Result<WardCalibrationRun> {
    let report = compute_ward_calibration_report(request, clock)?;
    let report_path = write_ward_calibration_report(output_root, &report)?;
    let readback = read_ward_calibration_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_WARD_CALIBRATION_READBACK_MISMATCH,
            format!(
                "Ward calibration report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(WardCalibrationRun {
        report_path,
        report: readback,
    })
}

pub fn compute_ward_calibration_report(
    request: &WardCalibrationRequest,
    clock: &dyn Clock,
) -> Result<WardCalibrationReport> {
    validate_request(request)?;
    let guard_id = parse_guard_id(&request.guard_id)?;
    let (good_scores, bad_scores) = split_scores(&request.residuals);
    let profile = calibrate(
        profile_template(request, guard_id),
        vec![CalibrationInput {
            slot: request.slot,
            good_scores,
            bad_scores,
            slot_kind: request.slot_kind,
            target_far: request.target_far,
        }],
        request.alpha,
        clock,
    )
    .map_err(map_ward_error)?;
    let calibration = profile.calibration.as_ref().ok_or_else(|| {
        PolyError::diagnostics(
            ERR_WARD_CALIBRATION_INVALID_REQUEST,
            "Ward calibrate returned a profile without calibration metadata",
        )
    })?;
    let stale_after_ts = calibration.ts.saturating_add(request.max_age_seconds);
    let stale = request.now_ts > stale_after_ts;
    if stale {
        return Err(PolyError::diagnostics(
            ERR_WARD_CALIBRATION_STALE,
            format!(
                "Ward calibration {} stale: now_ts={} stale_after_ts={}",
                request.calibration_version, request.now_ts, stale_after_ts
            ),
        ));
    }

    let verdict = run_guard(&profile, request)?;
    let guard_pass = verdict.overall_pass && !verdict.provisional;
    let anchor_count = request.residuals.len();
    let mut admission_inputs = request.admission_inputs.clone();
    admission_inputs.guard_calibrated = profile.is_calibrated();
    admission_inputs.guard_pass = guard_pass;
    admission_inputs.grounding_anchor_count = saturating_u32(anchor_count);
    let decision = evaluate_admission(&request.admission_params, &admission_inputs);
    let ledger = WardAdmissionLedger {
        schema_version: POLY_WARD_ADMISSION_LEDGER_SCHEMA_VERSION.to_string(),
        calibration_version: request.calibration_version.clone(),
        domain: request.domain.clone(),
        horizon_bucket: request.horizon_bucket.clone(),
        guard_calibrated: admission_inputs.guard_calibrated,
        guard_pass,
        grounding_anchor_count: admission_inputs.grounding_anchor_count,
        p_win: admission_inputs.p_win,
        confidence: admission_inputs.confidence,
        admitted: decision.admitted,
        code: decision.code,
        reason: decision.reason,
    };

    Ok(WardCalibrationReport {
        schema_version: POLY_WARD_CALIBRATION_SCHEMA_VERSION.to_string(),
        artifact_kind: POLY_WARD_CALIBRATION_ARTIFACT_KIND.to_string(),
        calibration_version: request.calibration_version.clone(),
        domain: request.domain.clone(),
        horizon_bucket: request.horizon_bucket.clone(),
        panel_version: request.panel_version,
        guard_id: request.guard_id.clone(),
        slot: request.slot,
        slot_kind: request.slot_kind,
        target_far: request.target_far,
        alpha: request.alpha,
        min_anchor_count: request.min_anchor_count,
        anchor_count,
        good_anchor_count: count_class(&request.residuals, WardResidualClass::KnownGood),
        bad_anchor_count: count_class(&request.residuals, WardResidualClass::KnownBad),
        calibration_ts: calibration.ts,
        stale_after_ts,
        now_ts: request.now_ts,
        stale,
        guard_calibrated: profile.is_calibrated(),
        guard_pass,
        candidate_score: request.candidate_score,
        residual_evidence_hash: residual_hash(&request.residuals),
        residual_evidence: request.residuals.clone(),
        calibration_meta: summarize_calibration(calibration),
        profile,
        guard_verdict: verdict,
        admission_ledger: ledger,
    })
}

pub fn apply_ward_calibration_to_admission(
    inputs: &mut AdmissionInputs,
    report: &WardCalibrationReport,
) {
    inputs.guard_calibrated = report.guard_calibrated && !report.stale;
    inputs.guard_pass = report.guard_pass;
    inputs.grounding_anchor_count = saturating_u32(report.anchor_count);
}

pub fn write_ward_calibration_report(
    dir: &Path,
    report: &WardCalibrationReport,
) -> Result<PathBuf> {
    let name = format!(
        "ward_calibration_{}_{}_{}.json",
        sanitize(&report.domain),
        sanitize(&report.horizon_bucket),
        sanitize(&report.calibration_version)
    );
    write_json(dir, &name, report)
}

pub fn read_ward_calibration_report(path: &Path) -> Result<WardCalibrationReport> {
    read_json(path)
}

fn validate_request(request: &WardCalibrationRequest) -> Result<()> {
    if request.calibration_version.trim().is_empty()
        || request.domain.trim().is_empty()
        || request.horizon_bucket.trim().is_empty()
        || request.guard_id.trim().is_empty()
    {
        return invalid_request(
            "calibration_version, domain, horizon_bucket, and guard_id are required",
        );
    }
    if request.min_anchor_count < calyx_ward::MIN_BAD_SCORES {
        return invalid_request(format!(
            "min_anchor_count {} below Ward conformal floor {}",
            request.min_anchor_count,
            calyx_ward::MIN_BAD_SCORES
        ));
    }
    if request.max_age_seconds < 0 || request.now_ts < 0 {
        return invalid_request("max_age_seconds and now_ts must be non-negative");
    }
    if !request.target_far.is_finite() || !(0.0..=1.0).contains(&request.target_far) {
        return invalid_request("target_far must be finite in [0,1]");
    }
    if !request.alpha.is_finite() || !(0.0..=1.0).contains(&request.alpha) {
        return invalid_request("alpha must be finite in [0,1]");
    }
    if !request.candidate_score.is_finite() || !(-1.0..=1.0).contains(&request.candidate_score) {
        return invalid_request("candidate_score must be a finite cosine value in [-1,1]");
    }
    for residual in &request.residuals {
        if residual.slot != request.slot {
            return invalid_request("all residual evidence rows must use the calibrated slot");
        }
        if !residual.score.is_finite() || !(-1.0..=1.0).contains(&residual.score) {
            return Err(PolyError::diagnostics(
                ERR_WARD_CALIBRATION_MALFORMED_RESIDUAL,
                "residual scores must be finite cosine values in [-1,1]",
            ));
        }
    }
    let bad_count = count_class(&request.residuals, WardResidualClass::KnownBad);
    if bad_count < request.min_anchor_count || request.residuals.len() < request.min_anchor_count {
        return Err(PolyError::diagnostics(
            ERR_WARD_CALIBRATION_INSUFFICIENT_ANCHORS,
            format!(
                "known-bad anchor count {} and total anchor count {} must both be at least {}",
                bad_count,
                request.residuals.len(),
                request.min_anchor_count
            ),
        ));
    }
    Ok(())
}

fn profile_template(request: &WardCalibrationRequest, guard_id: GuardId) -> GuardProfile {
    GuardProfile {
        guard_id,
        panel_version: request.panel_version,
        domain: request.domain.clone(),
        tau: BTreeMap::new(),
        required_slots: Vec::new(),
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn run_guard(profile: &GuardProfile, request: &WardCalibrationRequest) -> Result<GuardVerdict> {
    let produced = slot_vectors(request.slot, vec![1.0, 0.0]);
    let matched = slot_vectors(request.slot, cos_vector(request.candidate_score));
    guard(profile, &produced, &matched, true).map_err(map_ward_error)
}

fn slot_vectors(slot: SlotId, vector: Vec<f32>) -> ProducedSlots {
    BTreeMap::from([(slot, vector)])
}

fn cos_vector(cos: f32) -> Vec<f32> {
    vec![cos, (1.0 - cos * cos).max(0.0).sqrt()]
}

fn split_scores(residuals: &[WardCalibrationResidual]) -> (Vec<f32>, Vec<f32>) {
    let mut good = Vec::new();
    let mut bad = Vec::new();
    for residual in residuals {
        match residual.class {
            WardResidualClass::KnownGood => good.push(residual.score),
            WardResidualClass::KnownBad => bad.push(residual.score),
        }
    }
    (good, bad)
}

fn count_class(residuals: &[WardCalibrationResidual], class: WardResidualClass) -> usize {
    residuals.iter().filter(|row| row.class == class).count()
}

fn parse_guard_id(value: &str) -> Result<GuardId> {
    value.parse::<GuardId>().map_err(|err| {
        PolyError::diagnostics(
            ERR_WARD_CALIBRATION_INVALID_REQUEST,
            format!("guard_id must be a UUID: {err}"),
        )
    })
}

fn invalid_request<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_WARD_CALIBRATION_INVALID_REQUEST,
        message.into(),
    ))
}

fn map_ward_error(err: WardError) -> PolyError {
    PolyError::diagnostics(
        ERR_WARD_CALIBRATION_INVALID_REQUEST,
        format!("{}: {}", err.code(), err),
    )
}

fn summarize_calibration(meta: &CalibrationMeta) -> WardCalibrationMetaSummary {
    WardCalibrationMetaSummary {
        corpus_hash: hex(&meta.corpus_hash),
        estimator: meta.estimator.clone(),
        far: meta.far,
        frr: meta.frr,
        confidence: meta.confidence,
        ts: meta.ts,
        per_slot_count: meta.per_slot.len(),
    }
}

fn residual_hash(residuals: &[WardCalibrationResidual]) -> String {
    let mut hasher = Sha256::new();
    for residual in residuals {
        hasher.update(residual.slot.get().to_be_bytes());
        hasher.update(match residual.class {
            WardResidualClass::KnownGood => [1],
            WardResidualClass::KnownBad => [2],
        });
        hasher.update(residual.score.to_le_bytes());
    }
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
