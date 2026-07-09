use std::collections::BTreeMap;

use calyx_core::{CalyxError, LensId, Result};

use crate::spec::LensHealth;

use super::{
    CapabilityCard, CapabilitySignalKind, CostMetrics, CoverageMetrics, MetricSource,
    ProfileOptions, SeparationMetrics, SpreadMetrics,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Observation {
    pub(crate) data: Vec<f32>,
    pub(crate) label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DenseProfileRequest<'a> {
    pub lens_id: LensId,
    pub probe_count: usize,
    pub vectors: &'a [Vec<f32>],
    pub labels: &'a [Option<String>],
    pub cost: CostMetrics,
    pub signal: Option<f32>,
    pub signal_kind: CapabilitySignalKind,
    pub health: LensHealth,
}

pub(super) struct DenseCapabilityRequest {
    pub(super) lens_id: LensId,
    pub(super) probe_count: usize,
    pub(super) observations: Vec<Observation>,
    pub(super) cost: CostMetrics,
    pub(super) signal: Option<f32>,
    pub(super) signal_kind: CapabilitySignalKind,
    pub(super) health: LensHealth,
    pub(super) options: ProfileOptions,
}

pub fn profile_dense_vectors(request: DenseProfileRequest<'_>) -> Result<CapabilityCard> {
    let observations = request
        .vectors
        .iter()
        .enumerate()
        .map(|(idx, data)| Observation {
            data: data.clone(),
            label: request.labels.get(idx).cloned().unwrap_or(None),
        })
        .collect();
    dense_capability_card(DenseCapabilityRequest {
        lens_id: request.lens_id,
        probe_count: request.probe_count,
        observations,
        cost: request.cost,
        signal: request.signal,
        signal_kind: request.signal_kind,
        health: request.health,
        options: ProfileOptions::default(),
    })
}

pub(super) fn dense_capability_card(request: DenseCapabilityRequest) -> Result<CapabilityCard> {
    let DenseCapabilityRequest {
        lens_id,
        probe_count,
        observations,
        cost,
        signal,
        signal_kind,
        health,
        options,
    } = request;
    if probe_count == 0 || observations.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "profile requires at least one measurable vector",
        ));
    }
    ensure_same_dim(&observations)?;
    let spread = spread_metrics(&observations);
    let separation = separation_metrics(&observations);
    let measured = observations.len();
    let coverage = CoverageMetrics {
        requested: probe_count,
        measured,
        failed: probe_count.saturating_sub(measured),
        rate: measured as f32 / probe_count as f32,
    };
    let signal = signal.map(|bits| if bits.is_finite() { bits.max(0.0) } else { 0.0 });
    let signal_source = if signal.is_some() {
        MetricSource::AssayStore
    } else {
        MetricSource::AssayPending
    };
    let proxy_differentiation = separation.score;
    let proxy_signal = clamp01(
        coverage.rate
            * spread.normalized_participation_ratio
            * proxy_differentiation.clamp(0.0, 1.0),
    );
    Ok(CapabilityCard {
        lens_id,
        probe_count,
        signal,
        signal_source,
        signal_kind,
        signal_reliability: None,
        proxy_signal,
        differentiation: signal,
        differentiation_source: signal_source,
        proxy_differentiation,
        spread,
        separation,
        cost,
        coverage,
        health,
        low_spread: spread.normalized_participation_ratio < options.low_spread_threshold
            || spread.mean_pairwise_distance < options.low_distance_threshold,
    })
}

fn ensure_same_dim(observations: &[Observation]) -> Result<()> {
    let dim = observations[0].data.len();
    if dim == 0 {
        return Err(CalyxError::lens_dim_mismatch(
            "profile vectors must have non-zero dimension",
        ));
    }
    if observations.iter().all(|obs| obs.data.len() == dim) {
        return Ok(());
    }
    Err(CalyxError::lens_dim_mismatch(
        "profile vectors have inconsistent dimensions",
    ))
}

fn spread_metrics(observations: &[Observation]) -> SpreadMetrics {
    let dim = observations[0].data.len();
    let mean = mean_vector(observations, dim);
    let mut variances = vec![0.0_f32; dim];
    for obs in observations {
        for (idx, value) in obs.data.iter().enumerate() {
            let delta = *value - mean[idx];
            variances[idx] += delta * delta;
        }
    }
    let inv_n = 1.0 / observations.len() as f32;
    for value in &mut variances {
        *value *= inv_n;
    }

    let total_variance: f32 = variances.iter().sum();
    let variance_square_sum: f32 = variances.iter().map(|value| value * value).sum();
    let max_variance = variances.iter().copied().fold(0.0_f32, f32::max);
    let participation_ratio = if variance_square_sum <= f32::EPSILON {
        0.0
    } else {
        (total_variance * total_variance) / variance_square_sum
    };
    let stable_rank = if max_variance <= f32::EPSILON {
        0.0
    } else {
        total_variance / max_variance
    };
    let mean_pairwise_distance = mean_pairwise_distance(observations);

    SpreadMetrics {
        participation_ratio,
        normalized_participation_ratio: participation_ratio / dim as f32,
        stable_rank,
        total_variance,
        mean_pairwise_distance,
    }
}

fn separation_metrics(observations: &[Observation]) -> SeparationMetrics {
    let mean_pairwise_distance = mean_pairwise_distance(observations);
    let groups = label_groups(observations);
    let used_labels = groups.len() >= 2;
    let silhouette = if used_labels {
        silhouette_score(observations, &groups)
    } else {
        0.0
    };
    let score = if used_labels {
        silhouette
    } else {
        mean_pairwise_distance
    };

    SeparationMetrics {
        score,
        silhouette,
        mean_pairwise_distance,
        labeled_groups: groups.len(),
        used_labels,
    }
}

fn mean_vector(observations: &[Observation], dim: usize) -> Vec<f32> {
    let mut mean = vec![0.0_f32; dim];
    for obs in observations {
        for (dst, src) in mean.iter_mut().zip(&obs.data) {
            *dst += *src;
        }
    }
    let inv_n = 1.0 / observations.len() as f32;
    for value in &mut mean {
        *value *= inv_n;
    }
    mean
}

fn mean_pairwise_distance(observations: &[Observation]) -> f32 {
    if observations.len() < 2 {
        return 0.0;
    }
    let mut sum = 0.0_f32;
    let mut count = 0_usize;
    for left in 0..observations.len() {
        for right in (left + 1)..observations.len() {
            sum += euclidean(&observations[left].data, &observations[right].data);
            count += 1;
        }
    }
    sum / count as f32
}

fn label_groups(observations: &[Observation]) -> BTreeMap<String, Vec<usize>> {
    let mut groups = BTreeMap::new();
    for (idx, obs) in observations.iter().enumerate() {
        if let Some(label) = &obs.label {
            groups
                .entry(label.clone())
                .or_insert_with(Vec::new)
                .push(idx);
        }
    }
    groups
}

fn silhouette_score(observations: &[Observation], groups: &BTreeMap<String, Vec<usize>>) -> f32 {
    let mut sum = 0.0_f32;
    let mut count = 0_usize;
    for (idx, obs) in observations.iter().enumerate() {
        let Some(label) = &obs.label else {
            continue;
        };
        let Some(same) = groups.get(label) else {
            continue;
        };
        let a = mean_distance_to_group(idx, observations, same, true);
        let mut b = f32::INFINITY;
        for (other_label, group) in groups {
            if other_label == label {
                continue;
            }
            b = b.min(mean_distance_to_group(idx, observations, group, false));
        }
        let denom = a.max(b);
        let score = if denom <= f32::EPSILON {
            0.0
        } else {
            (b - a) / denom
        };
        sum += score;
        count += 1;
    }
    if count == 0 { 0.0 } else { sum / count as f32 }
}

fn mean_distance_to_group(
    idx: usize,
    observations: &[Observation],
    group: &[usize],
    skip_self: bool,
) -> f32 {
    let mut sum = 0.0_f32;
    let mut count = 0_usize;
    for &other in group {
        if skip_self && other == idx {
            continue;
        }
        sum += euclidean(&observations[idx].data, &observations[other].data);
        count += 1;
    }
    if count == 0 { 0.0 } else { sum / count as f32 }
}

fn euclidean(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(a, b)| {
            let delta = *a - *b;
            delta * delta
        })
        .sum::<f32>()
        .sqrt()
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}
