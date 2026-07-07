//! Gaussian PC-stable causal skeleton discovery (#68).
//!
//! This is the linear/Gaussian skeleton phase: no orientation, no nonlinear CI.
//! Depth updates are stable: all removals discovered at a conditioning depth are
//! applied only after every currently-adjacent pair has been tested at that depth.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::conditional_mi::{
    ConditionalIndependence, ConditionalMiReport,
    conditional_mutual_information_gaussian_with_alpha,
};
use crate::partial_correlation::pearson;

pub const DEFAULT_PC_ALPHA: f32 = 0.05;

#[derive(Clone, Copy)]
pub struct PcSeries<'a> {
    pub name: &'a str,
    pub values: &'a [f32],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PcEdge {
    pub left: String,
    pub right: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PcRemovedEdge {
    pub left: String,
    pub right: String,
    pub conditioning_set: Vec<String>,
    pub statistic_bits: f32,
    pub p_value: f32,
    pub depth: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PcStableReport {
    pub estimator: String,
    pub alpha: f32,
    pub max_conditioning: usize,
    pub n_samples: usize,
    pub variables: Vec<String>,
    pub retained_edges: Vec<PcEdge>,
    pub removed_edges: Vec<PcRemovedEdge>,
}

pub fn pc_stable_gaussian(
    series: &[PcSeries<'_>],
    alpha: f32,
    max_conditioning: usize,
) -> Result<PcStableReport> {
    validate_pc_inputs(series, alpha, max_conditioning)?;
    let n_vars = series.len();
    let mut adjacent = vec![vec![true; n_vars]; n_vars];
    for (idx, row) in adjacent.iter_mut().enumerate() {
        row[idx] = false;
    }

    let mut removed_edges = Vec::new();
    for depth in 0..=max_conditioning {
        let snapshot = adjacent.clone();
        let mut removals = Vec::new();
        for i in 0..n_vars {
            for j in (i + 1)..n_vars {
                if !snapshot[i][j] {
                    continue;
                }
                let candidates = neighbor_indices(&snapshot, i, j);
                if candidates.len() < depth {
                    continue;
                }
                for conditioning in combinations(&candidates, depth) {
                    let test = gaussian_ci_test(series, i, j, &conditioning, alpha)?;
                    if test.independent {
                        removals.push((i, j, test));
                        break;
                    }
                }
            }
        }
        for (i, j, test) in removals {
            if adjacent[i][j] {
                adjacent[i][j] = false;
                adjacent[j][i] = false;
                removed_edges.push(PcRemovedEdge {
                    left: series[i].name.to_string(),
                    right: series[j].name.to_string(),
                    conditioning_set: test
                        .conditioning
                        .iter()
                        .map(|&idx| series[idx].name.to_string())
                        .collect(),
                    statistic_bits: test.statistic_bits,
                    p_value: test.p_value,
                    depth,
                });
            }
        }
    }

    let mut retained_edges = Vec::new();
    for i in 0..n_vars {
        for j in (i + 1)..n_vars {
            if adjacent[i][j] {
                retained_edges.push(PcEdge {
                    left: series[i].name.to_string(),
                    right: series[j].name.to_string(),
                });
            }
        }
    }

    Ok(PcStableReport {
        estimator: "gaussian_pc_stable_skeleton".to_string(),
        alpha,
        max_conditioning,
        n_samples: series[0].values.len(),
        variables: series.iter().map(|s| s.name.to_string()).collect(),
        retained_edges,
        removed_edges,
    })
}

struct CiDecision {
    independent: bool,
    conditioning: Vec<usize>,
    statistic_bits: f32,
    p_value: f32,
}

fn gaussian_ci_test(
    series: &[PcSeries<'_>],
    left: usize,
    right: usize,
    conditioning: &[usize],
    alpha: f32,
) -> Result<CiDecision> {
    if conditioning.is_empty() {
        let report = pearson(series[left].values, series[right].values)?;
        let r = report.r as f64;
        let unexplained = 1.0 - r * r;
        if unexplained <= f64::EPSILON {
            return Ok(CiDecision {
                independent: false,
                conditioning: Vec::new(),
                statistic_bits: f32::INFINITY,
                p_value: 0.0,
            });
        }
        let bits = (-0.5 * unexplained.ln() / std::f64::consts::LN_2) as f32;
        return Ok(CiDecision {
            independent: report.p_value >= alpha,
            conditioning: Vec::new(),
            statistic_bits: bits,
            p_value: report.p_value,
        });
    }
    let controls: Vec<&[f32]> = conditioning.iter().map(|&idx| series[idx].values).collect();
    let report: ConditionalMiReport = conditional_mutual_information_gaussian_with_alpha(
        series[left].values,
        series[right].values,
        &controls,
        alpha,
    )?;
    Ok(CiDecision {
        independent: report.decision == ConditionalIndependence::Independent,
        conditioning: conditioning.to_vec(),
        statistic_bits: report.cmi_bits,
        p_value: report.p_value,
    })
}

fn validate_pc_inputs(series: &[PcSeries<'_>], alpha: f32, max_conditioning: usize) -> Result<()> {
    if series.len() < 2 {
        return Err(CalyxError::assay_insufficient_samples(
            "PC-stable requires at least two variables",
        ));
    }
    if !(alpha > 0.0 && alpha < 1.0) {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "PC-stable alpha must be in (0,1); got {alpha}"
        )));
    }
    if max_conditioning > series.len().saturating_sub(2) {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "PC-stable max_conditioning {max_conditioning} exceeds variables-2 for {} variables",
            series.len()
        )));
    }
    let n = series[0].values.len();
    if n < 4 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "PC-stable requires at least 4 samples; got {n}"
        )));
    }
    let mut names = std::collections::BTreeSet::new();
    for item in series {
        if item.name.trim().is_empty() || !names.insert(item.name) {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "PC-stable variable names must be non-empty and unique; bad name {:?}",
                item.name
            )));
        }
        if item.values.len() != n {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "PC-stable requires equal sample lengths; {} has {}, expected {n}",
                item.name,
                item.values.len()
            )));
        }
    }
    Ok(())
}

fn neighbor_indices(snapshot: &[Vec<bool>], node: usize, exclude: usize) -> Vec<usize> {
    snapshot[node]
        .iter()
        .enumerate()
        .filter_map(|(idx, &is_adjacent)| (idx != exclude && is_adjacent).then_some(idx))
        .collect()
}

fn combinations(values: &[usize], k: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut current = Vec::with_capacity(k);
    combinations_inner(values, k, 0, &mut current, &mut out);
    out
}

fn combinations_inner(
    values: &[usize],
    k: usize,
    start: usize,
    current: &mut Vec<usize>,
    out: &mut Vec<Vec<usize>>,
) {
    if current.len() == k {
        out.push(current.clone());
        return;
    }
    let needed = k - current.len();
    for index in start..=values.len() - needed {
        current.push(values[index]);
        combinations_inner(values, k, index + 1, current, out);
        current.pop();
    }
}
