//! Resolved-anchor floor tracker by domain and outcome axis (issue #78).
//!
//! Admission already refuses when it receives too few grounding anchors. This module owns the
//! source-of-truth count that feeds that refusal: local outcome-anchor row files are read back,
//! filtered to one `(domain, outcome_axis)`, deduplicated by anchor id, and counted only when the
//! anchor is a resolved UMA boolean outcome.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use calyx_core::{Anchor, AnchorValue};
use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};
use crate::grounding::{GroundingKind, grounding_kind_of};

/// Schema tag for anchor-floor reports.
pub const ANCHOR_FLOOR_SCHEMA_VERSION: &str = "poly.anchor_floor.v1";
/// Stable report filename.
pub const ANCHOR_FLOOR_REPORT_FILE: &str = "anchor-floor-report.json";
/// Default resolved-anchor floor for one domain/outcome axis.
pub const MIN_RESOLVED_ANCHOR_FLOOR: usize = 50;
/// Invalid tracker request.
pub const ERR_ANCHOR_FLOOR_INVALID_REQUEST: &str = "CALYX_POLY_ANCHOR_FLOOR_INVALID_REQUEST";
/// Invalid source row read from local storage.
pub const ERR_ANCHOR_FLOOR_INVALID_ROW: &str = "CALYX_POLY_ANCHOR_FLOOR_INVALID_ROW";
/// Report write/readback mismatch.
pub const ERR_ANCHOR_FLOOR_READBACK_MISMATCH: &str = "CALYX_POLY_ANCHOR_FLOOR_READBACK_MISMATCH";

/// One local source-of-truth row containing a resolved or candidate outcome anchor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnchorFloorRow {
    pub anchor_id: String,
    pub domain: String,
    pub outcome_axis: String,
    pub anchor: Anchor,
}

/// Request to count local anchor rows for one domain/outcome axis.
#[derive(Clone, Debug, PartialEq)]
pub struct AnchorFloorRequest {
    pub target_domain: String,
    pub target_outcome_axis: String,
    pub min_resolved_anchors: usize,
    pub row_paths: Vec<PathBuf>,
}

/// Result returned by a persisted tracker run.
#[derive(Clone, Debug, PartialEq)]
pub struct AnchorFloorRun {
    pub report_path: PathBuf,
    pub report: AnchorFloorReport,
}

/// Persisted anchor-floor count report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnchorFloorReport {
    pub schema_version: String,
    pub target_domain: String,
    pub target_outcome_axis: String,
    pub min_resolved_anchors: usize,
    pub source_row_count: usize,
    pub qualified_unique_count: usize,
    pub duplicate_excluded_count: usize,
    pub cross_domain_excluded_count: usize,
    pub cross_axis_excluded_count: usize,
    pub non_resolved_excluded_count: usize,
    pub non_boolean_excluded_count: usize,
    pub passed: bool,
    pub rows: Vec<AnchorFloorRowAudit>,
}

/// Per-row inclusion/exclusion evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnchorFloorRowAudit {
    pub row_path: String,
    pub anchor_id: String,
    pub domain: String,
    pub outcome_axis: String,
    pub decision: String,
    pub reason: String,
}

/// Counts local anchor row files and writes a readback-verified report.
pub fn run_anchor_floor_tracker(
    request: &AnchorFloorRequest,
    output_root: &Path,
) -> Result<AnchorFloorRun> {
    let report = evaluate_anchor_floor_from_files(request)?;
    let report_path = write_anchor_floor_report(output_root, &report)?;
    let readback = read_anchor_floor_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_ANCHOR_FLOOR_READBACK_MISMATCH,
            format!(
                "anchor-floor report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(AnchorFloorRun {
        report_path,
        report: readback,
    })
}

/// Counts local anchor row files without writing a report.
pub fn evaluate_anchor_floor_from_files(request: &AnchorFloorRequest) -> Result<AnchorFloorReport> {
    validate_request(request)?;

    let mut seen = BTreeSet::new();
    let mut rows = Vec::with_capacity(request.row_paths.len());
    let mut qualified_unique_count = 0usize;
    let mut duplicate_excluded_count = 0usize;
    let mut cross_domain_excluded_count = 0usize;
    let mut cross_axis_excluded_count = 0usize;
    let mut non_resolved_excluded_count = 0usize;
    let mut non_boolean_excluded_count = 0usize;

    for path in &request.row_paths {
        let row: AnchorFloorRow = read_json(path)?;
        validate_row(path, &row)?;
        let row_path = path.display().to_string();

        if row.domain != request.target_domain {
            cross_domain_excluded_count += 1;
            rows.push(audit(row_path, &row, "excluded", "cross_domain"));
            continue;
        }
        if row.outcome_axis != request.target_outcome_axis {
            cross_axis_excluded_count += 1;
            rows.push(audit(row_path, &row, "excluded", "cross_axis"));
            continue;
        }
        if grounding_kind_of(&row.anchor)? != GroundingKind::ResolvedUma {
            non_resolved_excluded_count += 1;
            rows.push(audit(row_path, &row, "excluded", "not_resolved_uma"));
            continue;
        }
        if !matches!(row.anchor.value, AnchorValue::Bool(_)) {
            non_boolean_excluded_count += 1;
            rows.push(audit(row_path, &row, "excluded", "not_boolean_outcome"));
            continue;
        }
        if !seen.insert(row.anchor_id.clone()) {
            duplicate_excluded_count += 1;
            rows.push(audit(row_path, &row, "excluded", "duplicate_anchor_id"));
            continue;
        }

        qualified_unique_count += 1;
        rows.push(audit(
            row_path,
            &row,
            "included",
            "unique_resolved_target_axis",
        ));
    }

    Ok(AnchorFloorReport {
        schema_version: ANCHOR_FLOOR_SCHEMA_VERSION.to_string(),
        target_domain: request.target_domain.clone(),
        target_outcome_axis: request.target_outcome_axis.clone(),
        min_resolved_anchors: request.min_resolved_anchors,
        source_row_count: request.row_paths.len(),
        qualified_unique_count,
        duplicate_excluded_count,
        cross_domain_excluded_count,
        cross_axis_excluded_count,
        non_resolved_excluded_count,
        non_boolean_excluded_count,
        passed: qualified_unique_count >= request.min_resolved_anchors,
        rows,
    })
}

/// Writes an anchor-floor report.
pub fn write_anchor_floor_report(dir: &Path, report: &AnchorFloorReport) -> Result<PathBuf> {
    write_json(dir, ANCHOR_FLOOR_REPORT_FILE, report)
}

/// Reads an anchor-floor report.
pub fn read_anchor_floor_report(path: &Path) -> Result<AnchorFloorReport> {
    read_json(path)
}

fn validate_request(request: &AnchorFloorRequest) -> Result<()> {
    if request.target_domain.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_ANCHOR_FLOOR_INVALID_REQUEST,
            "anchor-floor target_domain must not be empty",
        ));
    }
    if request.target_outcome_axis.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_ANCHOR_FLOOR_INVALID_REQUEST,
            "anchor-floor target_outcome_axis must not be empty",
        ));
    }
    if request.min_resolved_anchors == 0 {
        return Err(PolyError::diagnostics(
            ERR_ANCHOR_FLOOR_INVALID_REQUEST,
            "anchor-floor min_resolved_anchors must be greater than zero",
        ));
    }
    Ok(())
}

fn validate_row(path: &Path, row: &AnchorFloorRow) -> Result<()> {
    if row.anchor_id.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_ANCHOR_FLOOR_INVALID_ROW,
            format!("anchor-floor row {} has an empty anchor_id", path.display()),
        ));
    }
    if row.domain.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_ANCHOR_FLOOR_INVALID_ROW,
            format!("anchor-floor row {} has an empty domain", path.display()),
        ));
    }
    if row.outcome_axis.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_ANCHOR_FLOOR_INVALID_ROW,
            format!(
                "anchor-floor row {} has an empty outcome_axis",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn audit(
    row_path: String,
    row: &AnchorFloorRow,
    decision: &str,
    reason: &str,
) -> AnchorFloorRowAudit {
    AnchorFloorRowAudit {
        row_path,
        anchor_id: row.anchor_id.clone(),
        domain: row.domain.clone(),
        outcome_axis: row.outcome_axis.clone(),
        decision: decision.to_string(),
        reason: reason.to_string(),
    }
}
