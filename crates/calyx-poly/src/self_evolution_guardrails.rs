//! Safe self-evolution guardrails (issue #114).
//!
//! A self-tune can be promoted only when measured recall, guard false-accept rate, and latency clear
//! explicit tripwires, and when rollback plus reproduction artifacts physically exist and are hashed
//! into the report. Rejected candidates are recorded, but `require_self_evolution_approved` fails
//! closed so callers cannot treat a regression as a promotion.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::{PolyError, Result};

pub const SELF_EVOLUTION_GUARDRAIL_SCHEMA_VERSION: &str = "poly.self_evolution_guardrail.v1";
pub const SELF_EVOLUTION_GUARDRAIL_ARTIFACT_KIND: &str = "poly_self_evolution_guardrail_report";
pub const SELF_EVOLUTION_GUARDRAIL_REPORT_FILE: &str = "self_evolution_guardrail_report.json";

pub const ERR_SELF_EVOLUTION_INVALID_REQUEST: &str = "CALYX_POLY_SELF_EVOLUTION_INVALID_REQUEST";
pub const ERR_SELF_EVOLUTION_MISSING_ROLLBACK: &str = "CALYX_POLY_SELF_EVOLUTION_MISSING_ROLLBACK";
pub const ERR_SELF_EVOLUTION_MISSING_REPRODUCTION: &str =
    "CALYX_POLY_SELF_EVOLUTION_MISSING_REPRODUCTION";
pub const ERR_SELF_EVOLUTION_TRIPWIRE: &str = "CALYX_POLY_SELF_EVOLUTION_TRIPWIRE";
pub const ERR_SELF_EVOLUTION_READBACK_MISMATCH: &str =
    "CALYX_POLY_SELF_EVOLUTION_READBACK_MISMATCH";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfEvolutionMetrics {
    pub kernel_recall_ratio: f64,
    pub guard_far_ratio: f64,
    pub p95_latency_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfEvolutionTripwires {
    pub min_kernel_recall_ratio: f64,
    pub max_recall_regression: f64,
    pub max_guard_far_ratio: f64,
    pub max_guard_far_increase: f64,
    pub max_p95_latency_ms: f64,
    pub max_latency_increase_ratio: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelfEvolutionStatus {
    Approved,
    Rejected,
}

pub struct SelfEvolutionGuardrailRequest<'a> {
    pub out_dir: &'a Path,
    pub change_id: &'a str,
    pub rationale: &'a str,
    pub baseline: SelfEvolutionMetrics,
    pub candidate: SelfEvolutionMetrics,
    pub tripwires: SelfEvolutionTripwires,
    pub rollback_artifact_path: &'a Path,
    pub reproduction_plan_path: &'a Path,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfEvolutionTripwireCheck {
    pub name: String,
    pub baseline: f64,
    pub candidate: f64,
    pub comparator: String,
    pub threshold: f64,
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfEvolutionGuardrailReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub change_id: String,
    pub rationale: String,
    pub baseline: SelfEvolutionMetrics,
    pub candidate: SelfEvolutionMetrics,
    pub tripwires: SelfEvolutionTripwires,
    pub rollback_artifact_path: String,
    pub rollback_artifact_blake3: String,
    pub reproduction_plan_path: String,
    pub reproduction_plan_blake3: String,
    pub checks: Vec<SelfEvolutionTripwireCheck>,
    pub failed_check_count: usize,
    pub reversible: bool,
    pub reproducible: bool,
    pub status: SelfEvolutionStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfEvolutionGuardrailRun {
    pub report_path: PathBuf,
    pub report: SelfEvolutionGuardrailReport,
}

pub fn run_self_evolution_guardrail(
    request: &SelfEvolutionGuardrailRequest<'_>,
) -> Result<SelfEvolutionGuardrailRun> {
    let report = compute_self_evolution_guardrail(request)?;
    let report_path = write_json(
        request.out_dir,
        SELF_EVOLUTION_GUARDRAIL_REPORT_FILE,
        &report,
    )?;
    let readback = read_self_evolution_guardrail_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_SELF_EVOLUTION_READBACK_MISMATCH,
            format!(
                "self-evolution guardrail report changed during readback from {}",
                report_path.display()
            ),
        ));
    }
    Ok(SelfEvolutionGuardrailRun {
        report_path,
        report: readback,
    })
}

pub fn compute_self_evolution_guardrail(
    request: &SelfEvolutionGuardrailRequest<'_>,
) -> Result<SelfEvolutionGuardrailReport> {
    validate_request(request)?;
    let rollback_bytes = std::fs::read(request.rollback_artifact_path).map_err(|err| {
        PolyError::diagnostics(
            ERR_SELF_EVOLUTION_MISSING_ROLLBACK,
            format!(
                "read rollback artifact {}: {err}",
                request.rollback_artifact_path.display()
            ),
        )
    })?;
    let reproduction_bytes = std::fs::read(request.reproduction_plan_path).map_err(|err| {
        PolyError::diagnostics(
            ERR_SELF_EVOLUTION_MISSING_REPRODUCTION,
            format!(
                "read reproduction plan {}: {err}",
                request.reproduction_plan_path.display()
            ),
        )
    })?;
    let checks = tripwire_checks(request);
    let failed_check_count = checks.iter().filter(|check| !check.passed).count();
    Ok(SelfEvolutionGuardrailReport {
        schema_version: SELF_EVOLUTION_GUARDRAIL_SCHEMA_VERSION.to_string(),
        artifact_kind: SELF_EVOLUTION_GUARDRAIL_ARTIFACT_KIND.to_string(),
        change_id: request.change_id.to_string(),
        rationale: request.rationale.to_string(),
        baseline: report_metrics(request.baseline),
        candidate: report_metrics(request.candidate),
        tripwires: report_tripwires(request.tripwires),
        rollback_artifact_path: request.rollback_artifact_path.display().to_string(),
        rollback_artifact_blake3: blake3::hash(&rollback_bytes).to_hex().to_string(),
        reproduction_plan_path: request.reproduction_plan_path.display().to_string(),
        reproduction_plan_blake3: blake3::hash(&reproduction_bytes).to_hex().to_string(),
        checks,
        failed_check_count,
        reversible: !rollback_bytes.is_empty(),
        reproducible: !reproduction_bytes.is_empty(),
        status: if failed_check_count == 0 {
            SelfEvolutionStatus::Approved
        } else {
            SelfEvolutionStatus::Rejected
        },
    })
}

pub fn require_self_evolution_approved(report: &SelfEvolutionGuardrailReport) -> Result<()> {
    if report.status == SelfEvolutionStatus::Approved && report.reversible && report.reproducible {
        return Ok(());
    }
    let failed = report
        .checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(PolyError::diagnostics(
        ERR_SELF_EVOLUTION_TRIPWIRE,
        format!(
            "self-evolution candidate {} rejected; failed checks: {failed}",
            report.change_id
        ),
    ))
}

pub fn read_self_evolution_guardrail_report(path: &Path) -> Result<SelfEvolutionGuardrailReport> {
    read_json(path)
}

fn validate_request(request: &SelfEvolutionGuardrailRequest<'_>) -> Result<()> {
    if request.change_id.trim().is_empty() || request.rationale.trim().is_empty() {
        return invalid("change_id and rationale are required");
    }
    validate_metrics("baseline", request.baseline)?;
    validate_metrics("candidate", request.candidate)?;
    validate_tripwires(request.tripwires)
}

fn validate_metrics(label: &str, metrics: SelfEvolutionMetrics) -> Result<()> {
    if finite_ratio(metrics.kernel_recall_ratio)
        && finite_ratio(metrics.guard_far_ratio)
        && metrics.p95_latency_ms.is_finite()
        && metrics.p95_latency_ms > 0.0
    {
        Ok(())
    } else {
        invalid(format!(
            "{label} metrics must be finite and in policy range"
        ))
    }
}

fn validate_tripwires(tripwires: SelfEvolutionTripwires) -> Result<()> {
    let valid = finite_ratio(tripwires.min_kernel_recall_ratio)
        && finite_ratio(tripwires.max_recall_regression)
        && finite_ratio(tripwires.max_guard_far_ratio)
        && finite_ratio(tripwires.max_guard_far_increase)
        && tripwires.max_p95_latency_ms.is_finite()
        && tripwires.max_p95_latency_ms > 0.0
        && tripwires.max_latency_increase_ratio.is_finite()
        && tripwires.max_latency_increase_ratio >= 1.0;
    if valid {
        Ok(())
    } else {
        invalid("tripwires must be finite and in policy range")
    }
}

fn tripwire_checks(request: &SelfEvolutionGuardrailRequest<'_>) -> Vec<SelfEvolutionTripwireCheck> {
    let recall_regression =
        request.baseline.kernel_recall_ratio - request.candidate.kernel_recall_ratio;
    let guard_increase = request.candidate.guard_far_ratio - request.baseline.guard_far_ratio;
    let latency_ratio = request.candidate.p95_latency_ms / request.baseline.p95_latency_ms;
    vec![
        check(
            "kernel_recall_floor",
            request.baseline.kernel_recall_ratio,
            request.candidate.kernel_recall_ratio,
            ">=",
            request.tripwires.min_kernel_recall_ratio,
            request.candidate.kernel_recall_ratio >= request.tripwires.min_kernel_recall_ratio,
        ),
        check(
            "kernel_recall_regression",
            request.baseline.kernel_recall_ratio,
            recall_regression,
            "<=",
            request.tripwires.max_recall_regression,
            recall_regression <= request.tripwires.max_recall_regression,
        ),
        check(
            "guard_far_floor",
            request.baseline.guard_far_ratio,
            request.candidate.guard_far_ratio,
            "<=",
            request.tripwires.max_guard_far_ratio,
            request.candidate.guard_far_ratio <= request.tripwires.max_guard_far_ratio,
        ),
        check(
            "guard_far_increase",
            request.baseline.guard_far_ratio,
            guard_increase,
            "<=",
            request.tripwires.max_guard_far_increase,
            guard_increase <= request.tripwires.max_guard_far_increase,
        ),
        check(
            "p95_latency_floor",
            request.baseline.p95_latency_ms,
            request.candidate.p95_latency_ms,
            "<=",
            request.tripwires.max_p95_latency_ms,
            request.candidate.p95_latency_ms <= request.tripwires.max_p95_latency_ms,
        ),
        check(
            "p95_latency_increase_ratio",
            request.baseline.p95_latency_ms,
            latency_ratio,
            "<=",
            request.tripwires.max_latency_increase_ratio,
            latency_ratio <= request.tripwires.max_latency_increase_ratio,
        ),
    ]
}

fn check(
    name: &str,
    baseline: f64,
    candidate: f64,
    comparator: &str,
    threshold: f64,
    passed: bool,
) -> SelfEvolutionTripwireCheck {
    SelfEvolutionTripwireCheck {
        name: name.to_string(),
        baseline: report_float(baseline),
        candidate: report_float(candidate),
        comparator: comparator.to_string(),
        threshold: report_float(threshold),
        passed,
    }
}

fn report_metrics(metrics: SelfEvolutionMetrics) -> SelfEvolutionMetrics {
    SelfEvolutionMetrics {
        kernel_recall_ratio: report_float(metrics.kernel_recall_ratio),
        guard_far_ratio: report_float(metrics.guard_far_ratio),
        p95_latency_ms: report_float(metrics.p95_latency_ms),
    }
}

fn report_tripwires(tripwires: SelfEvolutionTripwires) -> SelfEvolutionTripwires {
    SelfEvolutionTripwires {
        min_kernel_recall_ratio: report_float(tripwires.min_kernel_recall_ratio),
        max_recall_regression: report_float(tripwires.max_recall_regression),
        max_guard_far_ratio: report_float(tripwires.max_guard_far_ratio),
        max_guard_far_increase: report_float(tripwires.max_guard_far_increase),
        max_p95_latency_ms: report_float(tripwires.max_p95_latency_ms),
        max_latency_increase_ratio: report_float(tripwires.max_latency_increase_ratio),
    }
}

fn report_float(value: f64) -> f64 {
    let rounded = (value * 1_000_000_000_000.0).round() / 1_000_000_000_000.0;
    if rounded == -0.0 { 0.0 } else { rounded }
}

fn finite_ratio(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_SELF_EVOLUTION_INVALID_REQUEST,
        message.into(),
    ))
}
