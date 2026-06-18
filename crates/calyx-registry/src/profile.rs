use std::collections::BTreeMap;
use std::time::Instant;

use calyx_core::{CalyxError, Input, LensId, Result, SlotVector};
use serde::{Deserialize, Serialize};

use crate::lens::Registry;
use crate::spec::LensHealth;

mod assay;
mod cost;
mod gating;
mod reliability;
mod signal_kind;
pub use assay::{apply_assay_metrics, profile_slot_with_assay};
pub use cost::CostMetrics;
pub use gating::{
    CAPABILITY_MAX_PAIRWISE_CORR_ENV, CAPABILITY_MIN_SIGNAL_BITS_ENV, CapabilityGateDecision,
    CapabilityGateEvaluation, CapabilityGateThresholds, append_capability_gate_ledger,
    capability_gate_json, evaluate_capability_gate, max_panel_pairwise_correlation,
};
use signal_kind::registry_signal_kind;
pub use signal_kind::{CapabilitySignalKind, signal_kind_from_runtime};

/// One profiling probe, optionally labeled for silhouette separation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileProbe {
    pub input: Input,
    pub label: Option<String>,
}

impl ProfileProbe {
    pub fn new(input: Input) -> Self {
        Self { input, label: None }
    }

    pub fn labeled(input: Input, label: impl Into<String>) -> Self {
        Self {
            input,
            label: Some(label.into()),
        }
    }
}

/// Lens capability summary produced from a fast probe set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityCard {
    pub lens_id: LensId,
    pub probe_count: usize,
    pub signal: Option<f32>,
    pub signal_source: MetricSource,
    #[serde(default)]
    pub signal_kind: CapabilitySignalKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_reliability: Option<CapabilitySignalReliability>,
    pub proxy_signal: f32,
    pub differentiation: Option<f32>,
    pub differentiation_source: MetricSource,
    pub proxy_differentiation: f32,
    pub spread: SpreadMetrics,
    pub separation: SeparationMetrics,
    pub cost: CostMetrics,
    pub coverage: CoverageMetrics,
    pub health: LensHealth,
    pub low_spread: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilitySignalReliability {
    pub ci_low: f32,
    pub ci_high: f32,
    pub seed_sigma: f32,
    pub seed_count: usize,
    pub unresolved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricSource {
    ProfileProxy,
    AssayPending,
    AssayStore,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpreadMetrics {
    pub participation_ratio: f32,
    pub normalized_participation_ratio: f32,
    pub stable_rank: f32,
    pub total_variance: f32,
    pub mean_pairwise_distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeparationMetrics {
    pub score: f32,
    pub silhouette: f32,
    pub mean_pairwise_distance: f32,
    pub labeled_groups: usize,
    pub used_labels: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoverageMetrics {
    pub requested: usize,
    pub measured: usize,
    pub failed: usize,
    pub rate: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileOptions {
    pub low_spread_threshold: f32,
    pub low_distance_threshold: f32,
}

impl Default for ProfileOptions {
    fn default() -> Self {
        Self {
            low_spread_threshold: 0.02,
            low_distance_threshold: 0.001,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Profiler {
    options: ProfileOptions,
}

impl Profiler {
    pub fn new(options: ProfileOptions) -> Self {
        Self { options }
    }

    pub fn profile_lens(
        &self,
        registry: &Registry,
        lens_id: LensId,
        probes: &[ProfileProbe],
    ) -> Result<CapabilityCard> {
        if probes.is_empty() {
            return Err(CalyxError::assay_insufficient_samples(
                "profile requires at least one probe",
            ));
        }

        let vram_before = vram_bytes();
        let started = Instant::now();
        let mut observations = Vec::new();
        let mut failed = 0_usize;
        for probe in probes {
            match registry.measure(lens_id, &probe.input) {
                Ok(vector) => match dense_projection(&vector)? {
                    Some(data) => observations.push(Observation {
                        data,
                        label: probe.label.clone(),
                    }),
                    None => failed += 1,
                },
                Err(_) => failed += 1,
            }
        }
        let total_ms = started.elapsed().as_secs_f64() as f32 * 1000.0;
        let vram_after = vram_bytes();
        if observations.is_empty() {
            return Err(CalyxError::assay_insufficient_samples(
                "profile produced no measurable vectors",
            ));
        }

        ensure_same_dim(&observations)?;
        let spread = spread_metrics(&observations);
        let separation = separation_metrics(&observations);
        let coverage = CoverageMetrics {
            requested: probes.len(),
            measured: observations.len(),
            failed,
            rate: observations.len() as f32 / probes.len() as f32,
        };
        let cost =
            CostMetrics::from_profile(total_ms, probes, &observations, vram_before, vram_after);
        let proxy_differentiation = separation.score;
        let proxy_signal = clamp01(
            coverage.rate
                * spread.normalized_participation_ratio
                * proxy_differentiation.clamp(0.0, 1.0),
        );
        let low_spread = spread.normalized_participation_ratio < self.options.low_spread_threshold
            || spread.mean_pairwise_distance < self.options.low_distance_threshold;

        Ok(CapabilityCard {
            lens_id,
            probe_count: probes.len(),
            signal: None,
            signal_source: MetricSource::AssayPending,
            signal_kind: registry_signal_kind(registry, lens_id),
            signal_reliability: None,
            proxy_signal,
            differentiation: None,
            differentiation_source: MetricSource::AssayPending,
            proxy_differentiation,
            spread,
            separation,
            cost,
            coverage,
            health: registry.health(lens_id)?,
            low_spread,
        })
    }
}

fn vram_bytes() -> u64 {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .output();
    let Ok(output) = output else {
        return 0;
    };
    if !output.status.success() {
        return 0;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .map(|mib| mib * 1024 * 1024)
        .sum()
}

pub fn profile_lens(
    registry: &Registry,
    lens_id: LensId,
    probes: &[ProfileProbe],
) -> Result<CapabilityCard> {
    Profiler::default().profile_lens(registry, lens_id, probes)
}

#[derive(Clone, Debug, PartialEq)]
struct Observation {
    data: Vec<f32>,
    label: Option<String>,
}

fn dense_projection(vector: &SlotVector) -> Result<Option<Vec<f32>>> {
    match vector {
        SlotVector::Dense { data, .. } => Ok(Some(data.clone())),
        SlotVector::Sparse { dim, entries } => {
            let mut data = vec![0.0; *dim as usize];
            for entry in entries {
                let Some(value) = data.get_mut(entry.idx as usize) else {
                    return Err(CalyxError::lens_dim_mismatch(format!(
                        "sparse entry {} outside dim {dim}",
                        entry.idx
                    )));
                };
                *value = entry.val;
            }
            Ok(Some(data))
        }
        SlotVector::Multi { token_dim, tokens } => {
            if tokens.is_empty() {
                return Ok(None);
            }
            let mut data = vec![0.0; *token_dim as usize];
            for token in tokens {
                if token.len() != *token_dim as usize {
                    return Err(CalyxError::lens_dim_mismatch(format!(
                        "multi token length {} != token_dim {token_dim}",
                        token.len()
                    )));
                }
                for (dst, src) in data.iter_mut().zip(token) {
                    *dst += *src;
                }
            }
            let scale = 1.0 / tokens.len() as f32;
            for value in &mut data {
                *value *= scale;
            }
            Ok(Some(data))
        }
        SlotVector::Absent { .. } => Ok(None),
    }
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

#[cfg(test)]
mod tests;
