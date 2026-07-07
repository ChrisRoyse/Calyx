//! Sparse Gaussian partial-correlation network (#69).
//!
//! This builds the partial-correlation-network side of graphical association:
//! each candidate undirected edge is tested after conditioning on every other
//! supplied signal. It is not graphical LASSO regularisation.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::partial_correlation::{PartialReport, partial_correlation_controlling};

pub const DEFAULT_PARTIAL_NETWORK_ALPHA: f32 = 0.05;
pub const DEFAULT_PARTIAL_NETWORK_MIN_ABS_R: f32 = 0.10;

#[derive(Clone, Copy)]
pub struct PartialNetworkSeries<'a> {
    pub name: &'a str,
    pub values: &'a [f32],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartialNetworkEdge {
    pub left: String,
    pub right: String,
    pub partial_r: f32,
    pub zero_order_r: f32,
    pub p_value: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    pub n_controls: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartialNetworkPrunedEdge {
    pub left: String,
    pub right: String,
    pub partial_r: f32,
    pub zero_order_r: f32,
    pub p_value: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    pub n_controls: usize,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartialNetworkReport {
    pub estimator: String,
    pub alpha: f32,
    pub min_abs_partial_r: f32,
    pub n_samples: usize,
    pub variables: Vec<String>,
    pub retained_edges: Vec<PartialNetworkEdge>,
    pub pruned_edges: Vec<PartialNetworkPrunedEdge>,
}

pub fn partial_correlation_network(
    series: &[PartialNetworkSeries<'_>],
    alpha: f32,
    min_abs_partial_r: f32,
) -> Result<PartialNetworkReport> {
    validate_partial_network_inputs(series, alpha, min_abs_partial_r)?;
    let mut retained_edges = Vec::new();
    let mut pruned_edges = Vec::new();

    for i in 0..series.len() {
        for j in (i + 1)..series.len() {
            let controls: Vec<&[f32]> = series
                .iter()
                .enumerate()
                .filter_map(|(idx, item)| (idx != i && idx != j).then_some(item.values))
                .collect();
            let partial =
                partial_correlation_controlling(series[i].values, series[j].values, &controls)?;
            let significant = partial.p_value < alpha;
            let clears_floor = partial.partial_r.abs() >= min_abs_partial_r;
            if significant && clears_floor {
                retained_edges.push(edge_record(series, i, j, partial));
            } else {
                pruned_edges.push(pruned_record(
                    series,
                    i,
                    j,
                    partial,
                    significant,
                    clears_floor,
                ));
            }
        }
    }

    Ok(PartialNetworkReport {
        estimator: "gaussian_partial_correlation_network".to_string(),
        alpha,
        min_abs_partial_r,
        n_samples: series[0].values.len(),
        variables: series.iter().map(|item| item.name.to_string()).collect(),
        retained_edges,
        pruned_edges,
    })
}

fn validate_partial_network_inputs(
    series: &[PartialNetworkSeries<'_>],
    alpha: f32,
    min_abs_partial_r: f32,
) -> Result<()> {
    if series.len() < 3 {
        return Err(CalyxError::assay_insufficient_samples(
            "partial-correlation network requires at least three variables",
        ));
    }
    if !(alpha > 0.0 && alpha < 1.0) {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "partial-correlation network alpha must be in (0,1); got {alpha}"
        )));
    }
    if !min_abs_partial_r.is_finite() || !(0.0..=1.0).contains(&min_abs_partial_r) {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "partial-correlation network min_abs_partial_r must be finite in [0,1]; got {min_abs_partial_r}"
        )));
    }
    let n = series[0].values.len();
    let min_samples = series.len() + 1;
    if n < min_samples {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "partial-correlation network over {} variables requires at least {min_samples} samples; got {n}",
            series.len()
        )));
    }
    let mut names = std::collections::BTreeSet::new();
    for item in series {
        if item.name.trim().is_empty() || !names.insert(item.name) {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "partial-correlation network variable names must be non-empty and unique; bad name {:?}",
                item.name
            )));
        }
        if item.values.len() != n {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "partial-correlation network requires equal sample lengths; {} has {}, expected {n}",
                item.name,
                item.values.len()
            )));
        }
        for (idx, value) in item.values.iter().enumerate() {
            if !value.is_finite() {
                return Err(CalyxError::assay_insufficient_samples(format!(
                    "partial-correlation network {}[{idx}] is not finite ({value})",
                    item.name
                )));
            }
        }
    }
    Ok(())
}

fn edge_record(
    series: &[PartialNetworkSeries<'_>],
    i: usize,
    j: usize,
    partial: PartialReport,
) -> PartialNetworkEdge {
    PartialNetworkEdge {
        left: series[i].name.to_string(),
        right: series[j].name.to_string(),
        partial_r: partial.partial_r,
        zero_order_r: partial.zero_order_r,
        p_value: partial.p_value,
        ci_low: partial.ci_low,
        ci_high: partial.ci_high,
        n_controls: partial.n_controls,
    }
}

fn pruned_record(
    series: &[PartialNetworkSeries<'_>],
    i: usize,
    j: usize,
    partial: PartialReport,
    significant: bool,
    clears_floor: bool,
) -> PartialNetworkPrunedEdge {
    PartialNetworkPrunedEdge {
        left: series[i].name.to_string(),
        right: series[j].name.to_string(),
        partial_r: partial.partial_r,
        zero_order_r: partial.zero_order_r,
        p_value: partial.p_value,
        ci_low: partial.ci_low,
        ci_high: partial.ci_high,
        n_controls: partial.n_controls,
        reason: prune_reason(significant, clears_floor).to_string(),
    }
}

fn prune_reason(significant: bool, clears_floor: bool) -> &'static str {
    match (significant, clears_floor) {
        (false, false) => "not_significant_and_below_effect_floor",
        (false, true) => "not_significant",
        (true, false) => "below_effect_floor",
        (true, true) => "retained",
    }
}
