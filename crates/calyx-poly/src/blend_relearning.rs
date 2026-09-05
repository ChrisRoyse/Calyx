//! Blend-weight re-learning from rolling held-out Brier (issue #112).
//!
//! Existing forecast blending consumes per-component reliability weights. This module learns those
//! weights from held-out component forecasts and resolved outcomes, persists the learned weights,
//! and reads the report back as the source of truth. Reliability is Brier skill versus the 0.25
//! Bernoulli no-skill baseline, clamped to `[0, 1]`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::forecast::ComponentKind;
use crate::{PolyError, Result};

pub const BLEND_RELEARNING_SCHEMA_VERSION: &str = "poly.blend_relearning.v1";
pub const BLEND_RELEARNING_ARTIFACT_KIND: &str = "poly_blend_relearning_report";
pub const BLEND_RELEARNING_REPORT_FILE: &str = "blend_relearning_report.json";

pub const ERR_BLEND_RELEARNING_EMPTY: &str = "CALYX_POLY_BLEND_RELEARNING_EMPTY";
pub const ERR_BLEND_RELEARNING_INVALID: &str = "CALYX_POLY_BLEND_RELEARNING_INVALID";
pub const ERR_BLEND_RELEARNING_INSUFFICIENT: &str = "CALYX_POLY_BLEND_RELEARNING_INSUFFICIENT";
pub const ERR_BLEND_RELEARNING_NO_SKILL: &str = "CALYX_POLY_BLEND_RELEARNING_NO_SKILL";
pub const ERR_BLEND_RELEARNING_READBACK_MISMATCH: &str =
    "CALYX_POLY_BLEND_RELEARNING_READBACK_MISMATCH";

const NO_SKILL_BRIER: f64 = 0.25;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendWeightObservation {
    pub component: ComponentKind,
    pub p_yes: f64,
    pub outcome_yes: bool,
    pub observed_at_millis: u64,
}

pub struct BlendRelearningRequest<'a> {
    pub out_dir: &'a Path,
    pub domain: &'a str,
    pub horizon_bucket: &'a str,
    pub as_of_millis: u64,
    pub min_samples_per_component: usize,
    pub observations: Vec<BlendWeightObservation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendWeightRow {
    pub component: ComponentKind,
    pub n: usize,
    pub brier: f64,
    pub reliability_weight: f64,
    pub normalized_weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendRelearningReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub horizon_bucket: String,
    pub as_of_millis: u64,
    pub min_samples_per_component: usize,
    pub component_count: usize,
    pub observation_count: usize,
    pub no_skill_brier: f64,
    pub total_reliability_weight: f64,
    pub rows: Vec<BlendWeightRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendRelearningRun {
    pub report_path: PathBuf,
    pub report: BlendRelearningReport,
}

pub fn run_blend_relearning(request: &BlendRelearningRequest<'_>) -> Result<BlendRelearningRun> {
    let report = compute_blend_relearning_report(request)?;
    let report_path = write_json(request.out_dir, BLEND_RELEARNING_REPORT_FILE, &report)?;
    let readback = read_blend_relearning_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_BLEND_RELEARNING_READBACK_MISMATCH,
            format!(
                "blend relearning report changed during readback from {}",
                report_path.display()
            ),
        ));
    }
    Ok(BlendRelearningRun {
        report_path,
        report: readback,
    })
}

pub fn compute_blend_relearning_report(
    request: &BlendRelearningRequest<'_>,
) -> Result<BlendRelearningReport> {
    validate_request(request)?;
    let mut groups: BTreeMap<ComponentKind, Vec<&BlendWeightObservation>> = BTreeMap::new();
    for obs in &request.observations {
        groups.entry(obs.component).or_default().push(obs);
    }
    if groups.len() < 2 {
        return Err(PolyError::diagnostics(
            ERR_BLEND_RELEARNING_INSUFFICIENT,
            "blend relearning needs at least two component families",
        ));
    }

    let mut rows = Vec::new();
    for (component, observations) in groups {
        if observations.len() < request.min_samples_per_component {
            return Err(PolyError::diagnostics(
                ERR_BLEND_RELEARNING_INSUFFICIENT,
                format!(
                    "{} has {} samples, below min {}",
                    component.slug(),
                    observations.len(),
                    request.min_samples_per_component
                ),
            ));
        }
        let brier = report_float(mean_brier(&observations));
        let reliability_weight =
            report_float(((NO_SKILL_BRIER - brier) / NO_SKILL_BRIER).clamp(0.0, 1.0));
        rows.push(BlendWeightRow {
            component,
            n: observations.len(),
            brier,
            reliability_weight,
            normalized_weight: 0.0,
        });
    }
    rows.sort_by_key(|row| row.component.slug());
    let total = report_float(rows.iter().map(|row| row.reliability_weight).sum::<f64>());
    if total <= 0.0 {
        return Err(PolyError::diagnostics(
            ERR_BLEND_RELEARNING_NO_SKILL,
            "all held-out component Brier scores were at or worse than no-skill",
        ));
    }
    for row in &mut rows {
        row.normalized_weight = report_float(row.reliability_weight / total);
    }

    Ok(BlendRelearningReport {
        schema_version: BLEND_RELEARNING_SCHEMA_VERSION.to_string(),
        artifact_kind: BLEND_RELEARNING_ARTIFACT_KIND.to_string(),
        domain: request.domain.to_string(),
        horizon_bucket: request.horizon_bucket.to_string(),
        as_of_millis: request.as_of_millis,
        min_samples_per_component: request.min_samples_per_component,
        component_count: rows.len(),
        observation_count: request.observations.len(),
        no_skill_brier: NO_SKILL_BRIER,
        total_reliability_weight: total,
        rows,
    })
}

pub fn read_blend_relearning_report(path: &Path) -> Result<BlendRelearningReport> {
    read_json(path)
}

fn validate_request(request: &BlendRelearningRequest<'_>) -> Result<()> {
    if request.domain.trim().is_empty()
        || request.horizon_bucket.trim().is_empty()
        || request.min_samples_per_component == 0
    {
        return Err(PolyError::diagnostics(
            ERR_BLEND_RELEARNING_INVALID,
            "domain, horizon_bucket, and min_samples_per_component are required",
        ));
    }
    if request.observations.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_BLEND_RELEARNING_EMPTY,
            "blend relearning requires held-out component observations",
        ));
    }
    for obs in &request.observations {
        if !obs.p_yes.is_finite() || !(0.0..=1.0).contains(&obs.p_yes) {
            return Err(PolyError::diagnostics(
                ERR_BLEND_RELEARNING_INVALID,
                format!(
                    "{} probability must be finite in [0, 1]",
                    obs.component.slug()
                ),
            ));
        }
        if obs.observed_at_millis > request.as_of_millis {
            return Err(PolyError::diagnostics(
                ERR_BLEND_RELEARNING_INVALID,
                "held-out observation cannot be after as_of_millis",
            ));
        }
    }
    Ok(())
}

fn mean_brier(observations: &[&BlendWeightObservation]) -> f64 {
    observations
        .iter()
        .map(|obs| {
            let y = if obs.outcome_yes { 1.0 } else { 0.0 };
            (obs.p_yes - y).powi(2)
        })
        .sum::<f64>()
        / observations.len() as f64
}

fn report_float(value: f64) -> f64 {
    let rounded = (value * 1_000_000_000_000.0).round() / 1_000_000_000_000.0;
    if rounded == -0.0 { 0.0 } else { rounded }
}
