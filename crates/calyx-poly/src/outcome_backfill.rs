//! Outcome-backfill scheduler for Provisional -> Trusted diagnostics (issue #77).
//!
//! The scheduler is intentionally small: a caller supplies queued provisional diagnostics, their
//! proxy/resolved anchors, and the same historical panel measured against resolved outcome anchors.
//! The scheduler reads the provisional artifact from disk, remeasures the panel, persists the trust
//! transition and resolved diagnostic, then reads the artifacts back before reporting success.

use std::path::{Path, PathBuf};

use calyx_assay::TrustTag;
use calyx_core::{Anchor, Clock};
use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};
use crate::grounding::{TrustTransition, promote_on_resolution};
use crate::no_lookahead::{NoLookaheadReport, NoLookaheadTiming, compute_no_lookahead_report};
use crate::panel_diagnostics::{
    PanelDiagnosticsConfig, PanelMatrix, compute_panel_diagnostics, read_panel_diagnostics,
    write_panel_diagnostics,
};

/// Schema tag for outcome-backfill scheduler reports.
pub const OUTCOME_BACKFILL_SCHEMA_VERSION: &str = "poly.outcome_backfill.v1";
/// Stable report filename written by the scheduler.
pub const OUTCOME_BACKFILL_REPORT_FILE: &str = "outcome-backfill-report.json";
/// No jobs were supplied, so no source-of-truth transition can be proven.
pub const ERR_BACKFILL_EMPTY: &str = "CALYX_POLY_OUTCOME_BACKFILL_EMPTY";
/// A queued job had missing identifying metadata.
pub const ERR_BACKFILL_INVALID_JOB: &str = "CALYX_POLY_OUTCOME_BACKFILL_INVALID_JOB";
/// The on-disk starting diagnostic was not Provisional.
pub const ERR_BACKFILL_NOT_PROVISIONAL: &str = "CALYX_POLY_OUTCOME_BACKFILL_SOURCE_NOT_PROVISIONAL";
/// The resolved recompute did not match the original diagnostic corpus shape.
pub const ERR_BACKFILL_CORPUS_MISMATCH: &str = "CALYX_POLY_OUTCOME_BACKFILL_CORPUS_MISMATCH";
/// Remeasuring against resolved anchors still produced Provisional output.
pub const ERR_BACKFILL_NOT_TRUSTED: &str = "CALYX_POLY_OUTCOME_BACKFILL_NOT_TRUSTED";
/// A just-written artifact did not read back byte-for-byte as the scheduler's source of truth.
pub const ERR_BACKFILL_READBACK_MISMATCH: &str = "CALYX_POLY_OUTCOME_BACKFILL_READBACK_MISMATCH";

/// One queued backfill job. `resolved_matrix` must contain the same slot keys and sample count as
/// the provisional diagnostic at `provisional_record_path`, with anchors replaced by resolved UMA
/// outcome anchors.
pub struct OutcomeBackfillJob {
    pub job_id: String,
    pub domain: String,
    pub panel_version: u32,
    pub provisional_record_path: PathBuf,
    pub proxy_anchor: Anchor,
    pub resolved_anchor: Anchor,
    pub timing: NoLookaheadTiming,
    pub resolved_matrix: PanelMatrix,
}

/// Result returned by a scheduler run.
#[derive(Clone, Debug, PartialEq)]
pub struct OutcomeBackfillRun {
    pub report_path: PathBuf,
    pub report: OutcomeBackfillReport,
}

/// Persisted scheduler report read back during FSV.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutcomeBackfillReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub job_count: usize,
    pub completed_count: usize,
    pub jobs: Vec<OutcomeBackfillJobReport>,
}

/// Persisted per-job evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutcomeBackfillJobReport {
    pub job_id: String,
    pub domain: String,
    pub panel_version: u32,
    pub provisional_record_path: String,
    pub resolved_record_path: String,
    pub transition_path: String,
    pub before_trust: TrustTag,
    pub after_trust: TrustTag,
    pub n_samples: usize,
    pub slot_keys: Vec<String>,
    pub resolved_provenance_hash: String,
    pub no_lookahead: NoLookaheadReport,
    pub transition: TrustTransition,
}

/// Runs all queued outcome-backfill jobs and writes a readback-verified report under `output_root`.
pub fn run_outcome_backfill_schedule(
    output_root: &Path,
    jobs: &[OutcomeBackfillJob],
    clock: &dyn Clock,
    config: &PanelDiagnosticsConfig,
) -> Result<OutcomeBackfillRun> {
    if jobs.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_BACKFILL_EMPTY,
            "outcome backfill scheduler requires at least one queued job",
        ));
    }

    let mut reports = Vec::with_capacity(jobs.len());
    for job in jobs {
        reports.push(run_job(output_root, job, clock, config)?);
    }

    let report = OutcomeBackfillReport {
        schema_version: OUTCOME_BACKFILL_SCHEMA_VERSION.to_string(),
        source_of_truth:
            "physical outcome-backfill report, per-job resolved diagnostics, and trust transitions"
                .to_string(),
        job_count: jobs.len(),
        completed_count: reports.len(),
        jobs: reports,
    };
    let report_path = write_outcome_backfill_report(output_root, &report)?;
    let readback = read_outcome_backfill_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_BACKFILL_READBACK_MISMATCH,
            format!(
                "outcome backfill report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(OutcomeBackfillRun {
        report_path,
        report: readback,
    })
}

/// Writes an outcome-backfill report.
pub fn write_outcome_backfill_report(
    dir: &Path,
    report: &OutcomeBackfillReport,
) -> Result<PathBuf> {
    write_json(dir, OUTCOME_BACKFILL_REPORT_FILE, report)
}

/// Reads an outcome-backfill report.
pub fn read_outcome_backfill_report(path: &Path) -> Result<OutcomeBackfillReport> {
    read_json(path)
}

fn run_job(
    output_root: &Path,
    job: &OutcomeBackfillJob,
    clock: &dyn Clock,
    config: &PanelDiagnosticsConfig,
) -> Result<OutcomeBackfillJobReport> {
    validate_job(job)?;
    let no_lookahead = compute_no_lookahead_report(
        format!("outcome-backfill job '{}'", job.job_id),
        job.timing.clone(),
        std::slice::from_ref(&job.resolved_anchor),
    )?;
    let before = read_panel_diagnostics(&job.provisional_record_path)?;
    if before.trust != TrustTag::Provisional {
        return Err(PolyError::diagnostics(
            ERR_BACKFILL_NOT_PROVISIONAL,
            format!(
                "backfill job '{}' expected a Provisional source diagnostic, got {:?}",
                job.job_id, before.trust
            ),
        ));
    }
    if before.n_samples != job.resolved_matrix.n_samples()
        || before.slot_keys != job.resolved_matrix.slot_keys()
    {
        return Err(PolyError::diagnostics(
            ERR_BACKFILL_CORPUS_MISMATCH,
            format!(
                "backfill job '{}' must remeasure the same corpus; before n={} slots={:?}, \
                 resolved n={} slots={:?}",
                job.job_id,
                before.n_samples,
                before.slot_keys,
                job.resolved_matrix.n_samples(),
                job.resolved_matrix.slot_keys()
            ),
        ));
    }

    let transition = promote_on_resolution(&job.proxy_anchor, &job.resolved_anchor)?;
    let resolved = compute_panel_diagnostics(
        &job.domain,
        job.panel_version,
        &job.resolved_matrix,
        clock,
        config,
    )?;
    if resolved.trust != TrustTag::Trusted {
        return Err(PolyError::diagnostics(
            ERR_BACKFILL_NOT_TRUSTED,
            format!(
                "backfill job '{}' remeasured against resolved anchors but produced {:?}; \
                 resolved output cannot be used as Trusted evidence",
                job.job_id, resolved.trust
            ),
        ));
    }

    let job_dir = output_root.join(sanitize(&job.job_id));
    let resolved_path = write_panel_diagnostics(&job_dir, &resolved)?;
    let resolved_readback = read_panel_diagnostics(&resolved_path)?;
    if resolved_readback != resolved {
        return Err(PolyError::diagnostics(
            ERR_BACKFILL_READBACK_MISMATCH,
            format!(
                "resolved diagnostic {} for job '{}' did not read back as written",
                resolved_path.display(),
                job.job_id
            ),
        ));
    }

    let transition_path = write_json(&job_dir, "trust-transition.json", &transition)?;
    let transition_readback: TrustTransition = read_json(&transition_path)?;
    if transition_readback != transition {
        return Err(PolyError::diagnostics(
            ERR_BACKFILL_READBACK_MISMATCH,
            format!(
                "trust transition {} for job '{}' did not read back as written",
                transition_path.display(),
                job.job_id
            ),
        ));
    }

    Ok(OutcomeBackfillJobReport {
        job_id: job.job_id.clone(),
        domain: job.domain.clone(),
        panel_version: job.panel_version,
        provisional_record_path: job.provisional_record_path.display().to_string(),
        resolved_record_path: resolved_path.display().to_string(),
        transition_path: transition_path.display().to_string(),
        before_trust: before.trust,
        after_trust: resolved_readback.trust,
        n_samples: resolved_readback.n_samples,
        slot_keys: resolved_readback.slot_keys,
        resolved_provenance_hash: resolved_readback.provenance_hash,
        no_lookahead,
        transition: transition_readback,
    })
}

fn validate_job(job: &OutcomeBackfillJob) -> Result<()> {
    if job.job_id.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_BACKFILL_INVALID_JOB,
            "outcome backfill job_id must not be empty",
        ));
    }
    if job.domain.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_BACKFILL_INVALID_JOB,
            format!("outcome backfill job '{}' has an empty domain", job.job_id),
        ));
    }
    Ok(())
}

fn sanitize(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if sanitized.is_empty() {
        "job".to_string()
    } else {
        sanitized
    }
}
