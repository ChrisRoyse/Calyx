use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use calyx_assay::{EstimateBound, TrustTag};

use super::types::{
    BuiltLensSpec, LensAutobuildReport, LensAutobuildRequest, LensAutobuildStatus,
    LensCandidateMeasurement, LensCandidateRejection, LensDeficit,
};
use super::{
    ERR_LENS_AUTOBUILD_INVALID_REQUEST, ERR_LENS_AUTOBUILD_NO_ADMISSIBLE,
    ERR_LENS_AUTOBUILD_NO_CANDIDATES, ERR_LENS_AUTOBUILD_NO_DEFICIT,
    ERR_LENS_AUTOBUILD_READBACK_MISMATCH, LENS_AUTOBUILD_ARTIFACT_KIND, LENS_AUTOBUILD_REPORT_FILE,
    LENS_AUTOBUILD_SCHEMA_VERSION,
};
use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct LensAutobuildRun {
    pub report_path: PathBuf,
    pub report: LensAutobuildReport,
}

pub fn run_lens_autobuild_report(
    request: &LensAutobuildRequest,
    output_root: &Path,
) -> Result<LensAutobuildRun> {
    let report = compute_lens_autobuild_report(request)?;
    let report_path = write_lens_autobuild_report(output_root, &report)?;
    let readback = read_lens_autobuild_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_LENS_AUTOBUILD_READBACK_MISMATCH,
            format!(
                "lens auto-build report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(LensAutobuildRun {
        report_path,
        report: readback,
    })
}

pub fn compute_lens_autobuild_report(
    request: &LensAutobuildRequest,
) -> Result<LensAutobuildReport> {
    validate_request(request)?;
    let primary_deficit = primary_deficit(request)?;
    let existing = existing_key_set(&request.existing_lens_keys);
    let mut admitted = Vec::new();
    let mut rejected = Vec::new();

    for candidate in &request.candidates {
        match reject_candidate(candidate, request.min_gain_bits, &existing) {
            Some(rejection) => rejected.push(rejection),
            None => admitted.push(build_lens_spec(candidate, primary_deficit)),
        }
    }

    let status = if admitted.is_empty() {
        LensAutobuildStatus::Rejected
    } else {
        LensAutobuildStatus::Admitted
    };
    let decision_hash = decision_hash(request, primary_deficit, &admitted, &rejected);

    Ok(LensAutobuildReport {
        schema_version: LENS_AUTOBUILD_SCHEMA_VERSION.to_string(),
        artifact_kind: LENS_AUTOBUILD_ARTIFACT_KIND.to_string(),
        domain: request.domain.clone(),
        panel_id: request.panel_id.clone(),
        panel_version: request.panel_version,
        min_gain_bits: request.min_gain_bits,
        existing_lens_count: request.existing_lens_keys.len(),
        deficit_count: request.deficits.len(),
        candidate_count: request.candidates.len(),
        admitted_count: admitted.len(),
        rejected_count: rejected.len(),
        status,
        primary_deficit: primary_deficit.clone(),
        admitted,
        rejected,
        decision_hash,
    })
}

pub fn require_lens_autobuild_admitted(report: &LensAutobuildReport) -> Result<()> {
    if report.status != LensAutobuildStatus::Admitted || report.admitted.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_LENS_AUTOBUILD_NO_ADMISSIBLE,
            "lens auto-build produced no admitted append_lens_spec artifact",
        ));
    }
    Ok(())
}

pub fn write_lens_autobuild_report(dir: &Path, report: &LensAutobuildReport) -> Result<PathBuf> {
    write_json(dir, LENS_AUTOBUILD_REPORT_FILE, report)
}

pub fn read_lens_autobuild_report(path: &Path) -> Result<LensAutobuildReport> {
    read_json(path)
}

fn validate_request(request: &LensAutobuildRequest) -> Result<()> {
    validate_label("domain", &request.domain)?;
    validate_label("panel_id", &request.panel_id)?;
    if request.panel_version == 0 {
        return invalid("panel_version must be positive");
    }
    if !request.min_gain_bits.is_finite() || request.min_gain_bits <= 0.0 {
        return invalid("min_gain_bits must be finite and positive");
    }
    if request.deficits.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_LENS_AUTOBUILD_NO_DEFICIT,
            "lens auto-build requires at least one propose_lens deficit",
        ));
    }
    if request.candidates.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_LENS_AUTOBUILD_NO_CANDIDATES,
            "lens auto-build requires at least one measured candidate lens",
        ));
    }
    for key in &request.existing_lens_keys {
        validate_existing_key("existing_lens_key", key)?;
    }
    for deficit in &request.deficits {
        validate_deficit(request, deficit)?;
    }
    for candidate in &request.candidates {
        validate_candidate(candidate)?;
    }
    Ok(())
}

fn validate_deficit(request: &LensAutobuildRequest, deficit: &LensDeficit) -> Result<()> {
    if deficit.domain != request.domain
        || deficit.panel_id != request.panel_id
        || deficit.panel_version != request.panel_version
    {
        return invalid("deficit domain, panel_id, and panel_version must match the request");
    }
    validate_label("deficit.source_artifact", &deficit.source_artifact)?;
    validate_label("deficit.proposal_action", &deficit.proposal_action)?;
    if !deficit.deficit_bits.is_finite() || deficit.deficit_bits <= 0.0 {
        return invalid("deficit_bits must be finite and positive");
    }
    if deficit.weakest_slots.is_empty() {
        return invalid("deficit weakest_slots must not be empty");
    }
    if deficit.proposal_action != "propose_lens" {
        return Err(PolyError::diagnostics(
            ERR_LENS_AUTOBUILD_NO_DEFICIT,
            format!(
                "deficit action {} is not propose_lens",
                deficit.proposal_action
            ),
        ));
    }
    Ok(())
}

fn validate_candidate(candidate: &LensCandidateMeasurement) -> Result<()> {
    validate_key("candidate.lens_key", &candidate.lens_key)?;
    validate_label("candidate.encoder_kind", &candidate.encoder_kind)?;
    validate_label("candidate.evidence_artifact", &candidate.evidence_artifact)?;
    validate_label("candidate.requested_action", &candidate.requested_action)?;
    if candidate.source_fields.is_empty() {
        return invalid("candidate source_fields must not be empty");
    }
    for field in &candidate.source_fields {
        validate_source_field(field)?;
    }
    if !candidate.measured_gain_bits.is_finite()
        || !candidate.ci_low_bits.is_finite()
        || !candidate.ci_high_bits.is_finite()
    {
        return invalid("candidate gain and CI values must be finite");
    }
    if candidate.ci_low_bits > candidate.ci_high_bits {
        return invalid("candidate ci_low_bits must be <= ci_high_bits");
    }
    if candidate.n_samples == 0 {
        return invalid("candidate n_samples must be positive");
    }
    Ok(())
}

fn primary_deficit(request: &LensAutobuildRequest) -> Result<&LensDeficit> {
    request
        .deficits
        .iter()
        .filter(|deficit| deficit.proposal_action == "propose_lens")
        .max_by(|left, right| left.deficit_bits.total_cmp(&right.deficit_bits))
        .ok_or_else(|| {
            PolyError::diagnostics(
                ERR_LENS_AUTOBUILD_NO_DEFICIT,
                "no propose_lens deficit was present",
            )
        })
}

fn reject_candidate(
    candidate: &LensCandidateMeasurement,
    min_gain_bits: f32,
    existing: &BTreeSet<String>,
) -> Option<LensCandidateRejection> {
    if existing.contains(&candidate.lens_key) {
        return Some(reject(candidate, "duplicate_existing_lens"));
    }
    if candidate.requested_action != "append_lens_spec"
        || forbidden_action(&candidate.requested_action)
    {
        return Some(reject(candidate, "forbidden_or_unsupported_action"));
    }
    if candidate.trust != TrustTag::Trusted {
        return Some(reject(candidate, "provisional_evidence"));
    }
    if candidate.estimate_bound != Some(EstimateBound::LowerBound) {
        return Some(reject(candidate, "uncalibrated_estimate_bound"));
    }
    if candidate.measured_gain_bits < min_gain_bits {
        return Some(reject(candidate, "below_gain_floor"));
    }
    if candidate.ci_low_bits <= 0.0 {
        return Some(reject(candidate, "gain_ci_crosses_zero"));
    }
    None
}

fn reject(candidate: &LensCandidateMeasurement, code: &str) -> LensCandidateRejection {
    LensCandidateRejection {
        lens_key: candidate.lens_key.clone(),
        code: code.to_string(),
        reason: rejection_reason(code).to_string(),
        measured_gain_bits: candidate.measured_gain_bits,
        trust: candidate.trust,
        estimate_bound: candidate.estimate_bound,
        requested_action: candidate.requested_action.clone(),
    }
}

fn rejection_reason(code: &str) -> &'static str {
    match code {
        "duplicate_existing_lens" => "candidate lens_key already exists in the panel",
        "forbidden_or_unsupported_action" => "candidate action must be local append_lens_spec only",
        "provisional_evidence" => "candidate evidence is provisional and cannot be admitted",
        "uncalibrated_estimate_bound" => "candidate evidence is not a calibrated lower bound",
        "below_gain_floor" => "candidate measured gain is below the lens-level bit floor",
        "gain_ci_crosses_zero" => "candidate gain CI does not stay above zero",
        _ => "candidate failed lens auto-build admission",
    }
}

fn build_lens_spec(candidate: &LensCandidateMeasurement, deficit: &LensDeficit) -> BuiltLensSpec {
    BuiltLensSpec {
        lens_id: lens_id(candidate, deficit),
        lens_key: candidate.lens_key.clone(),
        encoder_kind: candidate.encoder_kind.clone(),
        source_fields: candidate.source_fields.clone(),
        registry_patch_kind: "append_lens_spec".to_string(),
        target_slots: deficit.weakest_slots.clone(),
        expected_gain_bits: candidate.measured_gain_bits,
        ci_low_bits: candidate.ci_low_bits,
        n_samples: candidate.n_samples,
        trust: candidate.trust,
        estimate_bound: Some(EstimateBound::LowerBound),
        deficit_source_artifact: deficit.source_artifact.clone(),
        evidence_artifact: candidate.evidence_artifact.clone(),
    }
}

fn existing_key_set(existing: &[String]) -> BTreeSet<String> {
    existing.iter().map(|key| key.trim().to_string()).collect()
}

fn lens_id(candidate: &LensCandidateMeasurement, deficit: &LensDeficit) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(deficit.domain.as_bytes());
    hasher.update(deficit.panel_id.as_bytes());
    hasher.update(&deficit.panel_version.to_le_bytes());
    hasher.update(candidate.lens_key.as_bytes());
    hasher.update(candidate.encoder_kind.as_bytes());
    for field in &candidate.source_fields {
        hasher.update(field.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
}

fn decision_hash(
    request: &LensAutobuildRequest,
    deficit: &LensDeficit,
    admitted: &[BuiltLensSpec],
    rejected: &[LensCandidateRejection],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(LENS_AUTOBUILD_SCHEMA_VERSION.as_bytes());
    hasher.update(&[0]);
    hasher.update(request.domain.as_bytes());
    hasher.update(request.panel_id.as_bytes());
    hasher.update(&request.panel_version.to_le_bytes());
    hasher.update(&request.min_gain_bits.to_le_bytes());
    hasher.update(&deficit.deficit_bits.to_le_bytes());
    for slot in &deficit.weakest_slots {
        hasher.update(&slot.to_le_bytes());
    }
    for spec in admitted {
        hasher.update(spec.lens_id.as_bytes());
        hasher.update(spec.lens_key.as_bytes());
        hasher.update(&spec.expected_gain_bits.to_le_bytes());
        hasher.update(&spec.ci_low_bits.to_le_bytes());
        hasher.update(&(spec.n_samples as u64).to_le_bytes());
        hasher.update(&[estimate_bound_tag(spec.estimate_bound)]);
    }
    for rejection in rejected {
        hasher.update(rejection.lens_key.as_bytes());
        hasher.update(rejection.code.as_bytes());
        hasher.update(&rejection.measured_gain_bits.to_le_bytes());
        hasher.update(&[estimate_bound_tag(rejection.estimate_bound)]);
        hasher.update(format!("{:?}", rejection.trust).as_bytes());
        hasher.update(rejection.requested_action.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn estimate_bound_tag(bound: Option<EstimateBound>) -> u8 {
    match bound {
        None => 0,
        Some(EstimateBound::LowerBound) => 1,
        Some(EstimateBound::Point) => 2,
        Some(EstimateBound::UpperBound) => 3,
    }
}

fn forbidden_action(action: &str) -> bool {
    let normalized = action.to_ascii_lowercase();
    ["trade", "order", "sign", "execute", "wallet", "private_key"]
        .iter()
        .any(|term| normalized.contains(term))
}

fn validate_label(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return invalid(format!("{field} must not be empty"));
    }
    Ok(())
}

fn validate_key(field: &str, value: &str) -> Result<()> {
    validate_label(field, value)?;
    let valid = value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_');
    if !valid {
        return invalid(format!("{field} must be lowercase ascii/digit/underscore"));
    }
    Ok(())
}

fn validate_existing_key(field: &str, value: &str) -> Result<()> {
    validate_label(field, value)?;
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Ok(());
    }
    invalid(format!("{field} must be ascii alnum/underscore"))
}

fn validate_source_field(value: &str) -> Result<()> {
    validate_label("candidate.source_field", value)?;
    let valid = value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '.');
    if !valid {
        return invalid("candidate source fields must be lowercase field paths");
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_LENS_AUTOBUILD_INVALID_REQUEST,
        message.into(),
    ))
}
