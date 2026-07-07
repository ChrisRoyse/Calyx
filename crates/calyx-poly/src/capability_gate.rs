//! Poly wiring for the Calyx Registry capability gate (#45).
//!
//! The gate itself lives in `calyx-registry`: it admits learned lenses only when grounded signal
//! clears the Assay bit floor and pairwise correlation stays below the redundancy ceiling. This
//! module gives Poly a persisted, read-backable report around that engine decision and applies the
//! resulting panel lifecycle state through the registry swap controller.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use calyx_assay::contract::{MAX_PAIRWISE_CORR, MIN_SIGNAL_BITS};
use calyx_core::{LensId, Panel, SlotId, SlotState};
use calyx_registry::swap::SwapController;
use calyx_registry::{
    CapabilityCard, CapabilityGateDecision, CapabilityGateEvaluation, CapabilityGateThresholds,
    apply_capability_gate,
};
use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

pub const POLY_CAPABILITY_GATE_SCHEMA_VERSION: &str = "poly.capability_gate.v1";
pub const POLY_CAPABILITY_GATE_ARTIFACT_KIND: &str = "poly_lens_capability_gate";
pub const POLY_CAPABILITY_GATE_REPORT_FILE: &str = "capability_gate_report.json";
pub const POLY_CAPABILITY_MIN_SIGNAL_BITS: f32 = MIN_SIGNAL_BITS;
pub const POLY_CAPABILITY_MAX_PAIRWISE_CORR: f32 = MAX_PAIRWISE_CORR;

pub const ERR_CAPABILITY_GATE_INVALID_REQUEST: &str = "CALYX_POLY_CAPABILITY_GATE_INVALID_REQUEST";
pub const ERR_CAPABILITY_GATE_READBACK_MISMATCH: &str =
    "CALYX_POLY_CAPABILITY_GATE_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolyCapabilityGateMeasurement {
    pub slot_id: SlotId,
    pub card: CapabilityCard,
    pub max_pairwise_corr: f32,
    pub evidence_artifact: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolyCapabilityGateRequest {
    pub domain: String,
    pub panel_id: String,
    pub panel: Panel,
    pub thresholds: CapabilityGateThresholds,
    pub measured: Vec<PolyCapabilityGateMeasurement>,
    pub now: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolyCapabilityGateDecisionRow {
    pub slot_id: SlotId,
    pub lens_id: LensId,
    pub before_state: SlotState,
    pub decision: CapabilityGateDecision,
    pub after_state: SlotState,
    pub panel_version: u32,
    pub signal_bits: f32,
    pub signal_grounded: bool,
    pub max_pairwise_corr: f32,
    pub reason: String,
    pub evidence_artifact: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolyCapabilityGateReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub panel_id: String,
    pub thresholds: CapabilityGateThresholds,
    pub input_panel_version: u32,
    pub output_panel_version: u32,
    pub evaluated_count: usize,
    pub admitted_count: usize,
    pub parked_count: usize,
    pub retired_count: usize,
    pub before_panel: Panel,
    pub after_panel: Panel,
    pub evaluations: Vec<CapabilityGateEvaluation>,
    pub decisions: Vec<PolyCapabilityGateDecisionRow>,
    pub decision_hash: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PolyCapabilityGateRun {
    pub report_path: PathBuf,
    pub report: PolyCapabilityGateReport,
}

pub fn run_poly_capability_gate_report(
    request: &PolyCapabilityGateRequest,
    output_root: &Path,
) -> Result<PolyCapabilityGateRun> {
    let report = compute_poly_capability_gate_report(request)?;
    let report_path = write_poly_capability_gate_report(output_root, &report)?;
    let readback = read_poly_capability_gate_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_CAPABILITY_GATE_READBACK_MISMATCH,
            format!(
                "capability gate report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(PolyCapabilityGateRun {
        report_path,
        report: readback,
    })
}

pub fn compute_poly_capability_gate_report(
    request: &PolyCapabilityGateRequest,
) -> Result<PolyCapabilityGateReport> {
    validate_request(request)?;
    let before_panel = request.panel.clone();
    let mut controller = SwapController::new(request.panel.clone());
    let mut evaluations = Vec::with_capacity(request.measured.len());
    let mut decisions = Vec::with_capacity(request.measured.len());

    for measurement in &request.measured {
        let before_state = slot_state(controller.panel(), measurement.slot_id)?;
        let evaluation = calyx_registry::evaluate_capability_gate(
            measurement.card.clone(),
            measurement.max_pairwise_corr,
            request.thresholds,
        )?;
        let outcome = apply_capability_gate(
            &mut controller,
            measurement.slot_id,
            &evaluation,
            request.now,
        )?;
        decisions.push(decision_row(
            measurement,
            &evaluation,
            before_state,
            &outcome,
        ));
        evaluations.push(evaluation);
    }

    let after_panel = controller.panel().clone();
    let admitted_count = decisions
        .iter()
        .filter(|row| row.decision == CapabilityGateDecision::Admit)
        .count();
    let parked_count = decisions
        .iter()
        .filter(|row| row.decision == CapabilityGateDecision::Park)
        .count();
    let retired_count = decisions
        .iter()
        .filter(|row| row.decision == CapabilityGateDecision::Retire)
        .count();
    let decision_hash = decision_hash(request, &decisions, after_panel.version);

    Ok(PolyCapabilityGateReport {
        schema_version: POLY_CAPABILITY_GATE_SCHEMA_VERSION.to_string(),
        artifact_kind: POLY_CAPABILITY_GATE_ARTIFACT_KIND.to_string(),
        domain: request.domain.clone(),
        panel_id: request.panel_id.clone(),
        thresholds: request.thresholds,
        input_panel_version: before_panel.version,
        output_panel_version: after_panel.version,
        evaluated_count: decisions.len(),
        admitted_count,
        parked_count,
        retired_count,
        before_panel,
        after_panel,
        evaluations,
        decisions,
        decision_hash,
    })
}

pub fn write_poly_capability_gate_report(
    dir: &Path,
    report: &PolyCapabilityGateReport,
) -> Result<PathBuf> {
    write_json(dir, POLY_CAPABILITY_GATE_REPORT_FILE, report)
}

pub fn read_poly_capability_gate_report(path: &Path) -> Result<PolyCapabilityGateReport> {
    read_json(path)
}

fn validate_request(request: &PolyCapabilityGateRequest) -> Result<()> {
    validate_label("domain", &request.domain)?;
    validate_label("panel_id", &request.panel_id)?;
    request.thresholds.validate()?;
    if request.panel.version == 0 {
        return invalid("panel version must be positive");
    }
    if request.panel.slots.is_empty() {
        return invalid("capability gate requires at least one panel slot");
    }
    if request.measured.is_empty() {
        return invalid("capability gate requires at least one measured lens card");
    }
    let mut seen = BTreeSet::new();
    for measurement in &request.measured {
        validate_label(
            "measurement.evidence_artifact",
            &measurement.evidence_artifact,
        )?;
        if !seen.insert(measurement.slot_id.get()) {
            return invalid(format!(
                "duplicate measurement for slot {}",
                measurement.slot_id
            ));
        }
        let slot = request
            .panel
            .slots
            .iter()
            .find(|slot| slot.slot_id == measurement.slot_id)
            .ok_or_else(|| {
                PolyError::diagnostics(
                    ERR_CAPABILITY_GATE_INVALID_REQUEST,
                    format!(
                        "slot {} was measured but is not in the panel",
                        measurement.slot_id
                    ),
                )
            })?;
        if slot.lens_id != measurement.card.lens_id {
            return invalid(format!(
                "measurement lens {} does not match slot {} lens {}",
                measurement.card.lens_id, measurement.slot_id, slot.lens_id
            ));
        }
        if !measurement.max_pairwise_corr.is_finite() || measurement.max_pairwise_corr < 0.0 {
            return invalid("measurement max_pairwise_corr must be finite and non-negative");
        }
    }
    Ok(())
}

fn slot_state(panel: &Panel, slot_id: SlotId) -> Result<SlotState> {
    panel
        .slots
        .iter()
        .find(|slot| slot.slot_id == slot_id)
        .map(|slot| slot.state)
        .ok_or_else(|| {
            PolyError::diagnostics(
                ERR_CAPABILITY_GATE_INVALID_REQUEST,
                format!("slot {slot_id} was not found in the panel"),
            )
        })
}

fn decision_row(
    measurement: &PolyCapabilityGateMeasurement,
    evaluation: &CapabilityGateEvaluation,
    before_state: SlotState,
    outcome: &calyx_registry::PanelCapabilityGateOutcome,
) -> PolyCapabilityGateDecisionRow {
    PolyCapabilityGateDecisionRow {
        slot_id: measurement.slot_id,
        lens_id: outcome.lens_id,
        before_state,
        decision: outcome.decision,
        after_state: outcome.state,
        panel_version: outcome.panel_version,
        signal_bits: evaluation.signal_bits,
        signal_grounded: evaluation.signal_grounded,
        max_pairwise_corr: evaluation.max_pairwise_corr,
        reason: outcome.reason.clone(),
        evidence_artifact: measurement.evidence_artifact.clone(),
    }
}

fn decision_hash(
    request: &PolyCapabilityGateRequest,
    decisions: &[PolyCapabilityGateDecisionRow],
    output_panel_version: u32,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(request.domain.as_bytes());
    hasher.update(request.panel_id.as_bytes());
    hasher.update(&request.panel.version.to_le_bytes());
    hasher.update(&output_panel_version.to_le_bytes());
    hasher.update(&request.thresholds.min_signal_bits.to_le_bytes());
    hasher.update(&request.thresholds.max_pairwise_corr.to_le_bytes());
    for row in decisions {
        hasher.update(&row.slot_id.get().to_le_bytes());
        hasher.update(row.lens_id.to_string().as_bytes());
        hasher.update(format!("{:?}", row.before_state).as_bytes());
        hasher.update(format!("{:?}", row.decision).as_bytes());
        hasher.update(format!("{:?}", row.after_state).as_bytes());
        hasher.update(&row.signal_bits.to_le_bytes());
        hasher.update(&row.max_pairwise_corr.to_le_bytes());
        hasher.update(row.reason.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn validate_label(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return invalid(format!("{field} must not be empty"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_CAPABILITY_GATE_INVALID_REQUEST,
        message.into(),
    ))
}
