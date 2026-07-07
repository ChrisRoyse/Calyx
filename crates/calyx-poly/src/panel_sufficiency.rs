//! Persisted panel sufficiency measurement for Poly forecast admission (issue #79).
//!
//! The computation delegates to the real `calyx-assay` ensemble card, which estimates panel MI,
//! outcome entropy, per-lens marginal values, sufficiency, deficits, and the deficit proposal. Poly
//! owns the local artifact wrapper and readback gate.

use std::path::{Path, PathBuf};

use calyx_assay::{EnsembleCard, EnsembleConfig, EnsembleLensInput, ensemble_card};
use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

/// Schema tag for persisted Poly panel-sufficiency artifacts.
pub const POLY_PANEL_SUFFICIENCY_SCHEMA_VERSION: &str = "poly.panel_sufficiency.v1";
/// Artifact kind tag.
pub const POLY_PANEL_SUFFICIENCY_ARTIFACT_KIND: &str = "poly_panel_sufficiency";
/// Invalid request before Assay is invoked.
pub const ERR_PANEL_SUFFICIENCY_INVALID_REQUEST: &str =
    "CALYX_POLY_PANEL_SUFFICIENCY_INVALID_REQUEST";
/// Report write/readback mismatch.
pub const ERR_PANEL_SUFFICIENCY_READBACK_MISMATCH: &str =
    "CALYX_POLY_PANEL_SUFFICIENCY_READBACK_MISMATCH";

/// Request for one persisted panel-sufficiency measurement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolyPanelSufficiencyRequest {
    pub domain: String,
    pub panel_id: String,
    pub panel_version: u32,
    pub lenses: Vec<EnsembleLensInput>,
    pub labels: Vec<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    pub config: EnsembleConfig,
}

/// Result returned by a persisted sufficiency run.
#[derive(Clone, Debug, PartialEq)]
pub struct PolyPanelSufficiencyRun {
    pub report_path: PathBuf,
    pub report: PolyPanelSufficiencyReport,
}

/// Persisted sufficiency report. `assay_card` is included so FSV can read back the exact Assay
/// source values, not just a lossy Poly summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolyPanelSufficiencyReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub panel_id: String,
    pub panel_version: u32,
    pub n_samples: usize,
    pub lens_count: usize,
    pub anchor_entropy_bits: f32,
    pub panel_bits: f32,
    pub panel_ci: [f32; 2],
    pub sufficient: bool,
    pub deficit_bits: f32,
    pub deficit_count: usize,
    pub has_deficit_proposal: bool,
    pub assay_card: EnsembleCard,
}

/// Computes, writes, and readback-verifies one panel-sufficiency report.
pub fn run_panel_sufficiency_report(
    request: &PolyPanelSufficiencyRequest,
    output_root: &Path,
) -> Result<PolyPanelSufficiencyRun> {
    let report = compute_panel_sufficiency_report(request)?;
    let report_path = write_panel_sufficiency_report(output_root, &report)?;
    let readback = read_panel_sufficiency_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_PANEL_SUFFICIENCY_READBACK_MISMATCH,
            format!(
                "panel-sufficiency report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(PolyPanelSufficiencyRun {
        report_path,
        report: readback,
    })
}

/// Computes one panel-sufficiency report without writing it.
pub fn compute_panel_sufficiency_report(
    request: &PolyPanelSufficiencyRequest,
) -> Result<PolyPanelSufficiencyReport> {
    validate_request(request)?;
    let groups = request.groups.as_deref();
    let card = ensemble_card(&request.lenses, &request.labels, groups, &request.config)?;
    Ok(PolyPanelSufficiencyReport {
        schema_version: POLY_PANEL_SUFFICIENCY_SCHEMA_VERSION.to_string(),
        artifact_kind: POLY_PANEL_SUFFICIENCY_ARTIFACT_KIND.to_string(),
        domain: request.domain.clone(),
        panel_id: request.panel_id.clone(),
        panel_version: request.panel_version,
        n_samples: card.n_samples,
        lens_count: card.panel_lens_count,
        anchor_entropy_bits: card.anchor_entropy_bits,
        panel_bits: card.panel_bits,
        panel_ci: card.panel_ci,
        sufficient: card.sufficient,
        deficit_bits: card.deficit_bits,
        deficit_count: card.sufficiency.deficits.len(),
        has_deficit_proposal: card.deficit_proposal.is_some(),
        assay_card: card,
    })
}

/// Writes a panel-sufficiency report.
pub fn write_panel_sufficiency_report(
    dir: &Path,
    report: &PolyPanelSufficiencyReport,
) -> Result<PathBuf> {
    let name = format!(
        "panel_sufficiency_{}_{}_v{}.json",
        sanitize(&report.domain),
        sanitize(&report.panel_id),
        report.panel_version
    );
    write_json(dir, &name, report)
}

/// Reads a panel-sufficiency report.
pub fn read_panel_sufficiency_report(path: &Path) -> Result<PolyPanelSufficiencyReport> {
    read_json(path)
}

fn validate_request(request: &PolyPanelSufficiencyRequest) -> Result<()> {
    if request.domain.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_PANEL_SUFFICIENCY_INVALID_REQUEST,
            "panel sufficiency domain must not be empty",
        ));
    }
    if request.panel_id.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_PANEL_SUFFICIENCY_INVALID_REQUEST,
            "panel sufficiency panel_id must not be empty",
        ));
    }
    if let Some(groups) = &request.groups
        && groups.len() != request.labels.len()
    {
        return Err(PolyError::diagnostics(
            ERR_PANEL_SUFFICIENCY_INVALID_REQUEST,
            format!(
                "panel sufficiency groups length {} != labels length {}",
                groups.len(),
                request.labels.len()
            ),
        ));
    }
    Ok(())
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
