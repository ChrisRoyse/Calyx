//! No-look-ahead timing guards for as-of features and resolved outcome anchors (issue #80).
//!
//! All timestamps in a [`NoLookaheadTiming`] must be in the same unit. When validating Calyx
//! anchors, use Unix milliseconds (`calyx_core::Ts`), because anchors store `observed_at` in that
//! unit. Market snapshots/resolutions can use their native Unix seconds through
//! [`validate_snapshot_before_resolution`], which compares only those two source timestamps.

use std::path::{Path, PathBuf};

use calyx_core::Anchor;
use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

/// Schema tag for persisted no-look-ahead reports.
pub const NO_LOOKAHEAD_SCHEMA_VERSION: &str = "poly.no_lookahead.v1";
/// Stable report filename written by the no-look-ahead verifier.
pub const NO_LOOKAHEAD_REPORT_FILE: &str = "no-lookahead-report.json";
/// A feature observation is newer than the snapshot/forecast it is meant to support.
pub const ERR_LOOKAHEAD_FEATURE_AFTER_SNAPSHOT: &str =
    "CALYX_POLY_NO_LOOKAHEAD_FEATURE_AFTER_SNAPSHOT";
/// A resolved label is not strictly after the snapshot it would ground.
pub const ERR_LOOKAHEAD_RESOLUTION_NOT_AFTER_SNAPSHOT: &str =
    "CALYX_POLY_NO_LOOKAHEAD_RESOLUTION_NOT_AFTER_SNAPSHOT";
/// A backfill is timestamped before the resolution label it would use.
pub const ERR_LOOKAHEAD_BACKFILL_BEFORE_RESOLUTION: &str =
    "CALYX_POLY_NO_LOOKAHEAD_BACKFILL_BEFORE_RESOLUTION";
/// A no-look-ahead anchor audit had no anchors to prove.
pub const ERR_LOOKAHEAD_EMPTY_ANCHORS: &str = "CALYX_POLY_NO_LOOKAHEAD_EMPTY_ANCHORS";
/// A resolved anchor is not strictly after the snapshot it would ground.
pub const ERR_LOOKAHEAD_ANCHOR_NOT_AFTER_SNAPSHOT: &str =
    "CALYX_POLY_NO_LOOKAHEAD_ANCHOR_NOT_AFTER_SNAPSHOT";
/// A resolved anchor was observed before the resolution timestamp.
pub const ERR_LOOKAHEAD_ANCHOR_BEFORE_RESOLUTION: &str =
    "CALYX_POLY_NO_LOOKAHEAD_ANCHOR_BEFORE_RESOLUTION";
/// A resolved anchor was observed after the backfill/report timestamp.
pub const ERR_LOOKAHEAD_ANCHOR_AFTER_BACKFILL: &str =
    "CALYX_POLY_NO_LOOKAHEAD_ANCHOR_AFTER_BACKFILL";
/// A just-written no-look-ahead report did not read back as written.
pub const ERR_LOOKAHEAD_READBACK_MISMATCH: &str = "CALYX_POLY_NO_LOOKAHEAD_READBACK_MISMATCH";

/// The minimum timing proof that labels cannot leak into as-of features.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoLookaheadTiming {
    /// Newest feature input used by the pre-resolution snapshot/forecast.
    pub feature_max_observed_at: u64,
    /// The snapshot/forecast as-of timestamp.
    pub snapshot_observed_at: u64,
    /// The trusted outcome resolution timestamp.
    pub resolution_observed_at: u64,
    /// The timestamp of the backfill/report using the resolved label.
    pub backfill_observed_at: u64,
}

/// One resolved anchor timing audit row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoLookaheadAnchorAudit {
    pub source: String,
    pub observed_at: u64,
    pub snapshot_observed_at: u64,
    pub resolution_observed_at: u64,
    pub backfill_observed_at: u64,
    pub passed: bool,
}

/// Result returned by a persisted no-look-ahead run.
#[derive(Clone, Debug, PartialEq)]
pub struct NoLookaheadRun {
    pub report_path: PathBuf,
    pub report: NoLookaheadReport,
}

/// Persisted no-look-ahead proof.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoLookaheadReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub timing: NoLookaheadTiming,
    pub anchor_count: usize,
    pub anchor_audits: Vec<NoLookaheadAnchorAudit>,
    pub passed: bool,
}

/// Validates scalar as-of timing. Features must be no newer than the snapshot, resolution must be
/// strictly after the snapshot, and backfill/reporting must not predate the resolution.
pub fn validate_no_lookahead_timing(timing: &NoLookaheadTiming) -> Result<()> {
    if timing.feature_max_observed_at > timing.snapshot_observed_at {
        return Err(PolyError::diagnostics(
            ERR_LOOKAHEAD_FEATURE_AFTER_SNAPSHOT,
            format!(
                "feature_max_observed_at {} is after snapshot_observed_at {}; refusing \
                 look-ahead input",
                timing.feature_max_observed_at, timing.snapshot_observed_at
            ),
        ));
    }
    if timing.resolution_observed_at <= timing.snapshot_observed_at {
        return Err(PolyError::diagnostics(
            ERR_LOOKAHEAD_RESOLUTION_NOT_AFTER_SNAPSHOT,
            format!(
                "resolution_observed_at {} must be strictly after snapshot_observed_at {}",
                timing.resolution_observed_at, timing.snapshot_observed_at
            ),
        ));
    }
    if timing.backfill_observed_at < timing.resolution_observed_at {
        return Err(PolyError::diagnostics(
            ERR_LOOKAHEAD_BACKFILL_BEFORE_RESOLUTION,
            format!(
                "backfill_observed_at {} is before resolution_observed_at {}",
                timing.backfill_observed_at, timing.resolution_observed_at
            ),
        ));
    }
    Ok(())
}

/// Validates a market snapshot/resolution pair using their native Unix-second timestamps.
pub fn validate_snapshot_before_resolution(
    snapshot_ts: u64,
    resolved_ts: u64,
    context: &str,
) -> Result<()> {
    validate_no_lookahead_timing(&NoLookaheadTiming {
        feature_max_observed_at: snapshot_ts,
        snapshot_observed_at: snapshot_ts,
        resolution_observed_at: resolved_ts,
        backfill_observed_at: resolved_ts,
    })
    .map_err(|err| PolyError::diagnostics(err.code(), format!("{context}: {}", err.message())))
}

/// Validates resolved anchors against a timing proof and returns readbackable audit rows.
pub fn validate_resolution_anchor_timing(
    timing: &NoLookaheadTiming,
    resolution_anchors: &[Anchor],
) -> Result<Vec<NoLookaheadAnchorAudit>> {
    validate_no_lookahead_timing(timing)?;
    if resolution_anchors.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_LOOKAHEAD_EMPTY_ANCHORS,
            "no-look-ahead anchor audit requires at least one resolved anchor",
        ));
    }

    let mut audits = Vec::with_capacity(resolution_anchors.len());
    for anchor in resolution_anchors {
        if anchor.observed_at <= timing.snapshot_observed_at {
            return Err(PolyError::diagnostics(
                ERR_LOOKAHEAD_ANCHOR_NOT_AFTER_SNAPSHOT,
                format!(
                    "anchor '{}' observed_at {} must be after snapshot_observed_at {}",
                    anchor.source, anchor.observed_at, timing.snapshot_observed_at
                ),
            ));
        }
        if anchor.observed_at < timing.resolution_observed_at {
            return Err(PolyError::diagnostics(
                ERR_LOOKAHEAD_ANCHOR_BEFORE_RESOLUTION,
                format!(
                    "anchor '{}' observed_at {} is before resolution_observed_at {}",
                    anchor.source, anchor.observed_at, timing.resolution_observed_at
                ),
            ));
        }
        if anchor.observed_at > timing.backfill_observed_at {
            return Err(PolyError::diagnostics(
                ERR_LOOKAHEAD_ANCHOR_AFTER_BACKFILL,
                format!(
                    "anchor '{}' observed_at {} is after backfill_observed_at {}",
                    anchor.source, anchor.observed_at, timing.backfill_observed_at
                ),
            ));
        }
        audits.push(NoLookaheadAnchorAudit {
            source: anchor.source.clone(),
            observed_at: anchor.observed_at,
            snapshot_observed_at: timing.snapshot_observed_at,
            resolution_observed_at: timing.resolution_observed_at,
            backfill_observed_at: timing.backfill_observed_at,
            passed: true,
        });
    }
    Ok(audits)
}

/// Computes a readbackable no-look-ahead report from source timing and resolved anchors.
pub fn compute_no_lookahead_report(
    source_of_truth: impl Into<String>,
    timing: NoLookaheadTiming,
    resolution_anchors: &[Anchor],
) -> Result<NoLookaheadReport> {
    let anchor_audits = validate_resolution_anchor_timing(&timing, resolution_anchors)?;
    Ok(NoLookaheadReport {
        schema_version: NO_LOOKAHEAD_SCHEMA_VERSION.to_string(),
        source_of_truth: source_of_truth.into(),
        timing,
        anchor_count: anchor_audits.len(),
        anchor_audits,
        passed: true,
    })
}

/// Writes a no-look-ahead report.
pub fn write_no_lookahead_report(dir: &Path, report: &NoLookaheadReport) -> Result<PathBuf> {
    write_json(dir, NO_LOOKAHEAD_REPORT_FILE, report)
}

/// Reads a no-look-ahead report.
pub fn read_no_lookahead_report(path: &Path) -> Result<NoLookaheadReport> {
    read_json(path)
}

/// Writes and reads back a no-look-ahead report, returning the physical source-of-truth path.
pub fn run_no_lookahead_report(
    output_root: &Path,
    source_of_truth: impl Into<String>,
    timing: NoLookaheadTiming,
    resolution_anchors: &[Anchor],
) -> Result<NoLookaheadRun> {
    let report = compute_no_lookahead_report(source_of_truth, timing, resolution_anchors)?;
    let report_path = write_no_lookahead_report(output_root, &report)?;
    let readback = read_no_lookahead_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_LOOKAHEAD_READBACK_MISMATCH,
            format!(
                "no-look-ahead report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(NoLookaheadRun {
        report_path,
        report: readback,
    })
}
