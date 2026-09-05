use std::time::Instant;

use calyx_core::{CalyxError, Input, LensId, Result, SlotVector};
use serde::{Deserialize, Serialize};

use crate::lens::Registry;
use crate::spec::LensHealth;

mod assay;
mod cost;
mod dense_card;
mod dense_matrix;
mod gating;
mod reliability;
mod signal_kind;
pub use assay::{apply_assay_metrics, profile_slot_with_assay};
pub use cost::CostMetrics;
pub(crate) use dense_card::Observation;
use dense_card::{DenseCapabilityRequest, dense_capability_card};
pub use dense_card::{DenseProfileRequest, profile_dense_vectors};
pub use dense_matrix::{
    CALYX_PROFILE_CUDA_MIN_ROWS_ENV, CALYX_PROFILE_REQUIRE_CUDA_ENV, DEFAULT_PROFILE_CUDA_MIN_ROWS,
};
pub use gating::{
    CAPABILITY_MAX_PAIRWISE_CORR_ENV, CAPABILITY_MIN_SIGNAL_BITS_ENV, CapabilityGateDecision,
    CapabilityGateEvaluation, CapabilityGateThresholds, append_capability_gate_ledger,
    capability_gate_json, evaluate_capability_gate, max_panel_pairwise_correlation,
};
use signal_kind::registry_signal_kind;
pub use signal_kind::{CapabilitySignalKind, signal_kind_from_spec};

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
    #[serde(default)]
    pub execution: ProfileExecutionStats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileMathBackend {
    #[default]
    CpuFullMatrix,
    CudaCublasTiledGram,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileExecutionStats {
    pub measurement_passes: usize,
    pub batch_measure_calls: usize,
    pub scalar_measure_calls: usize,
    pub resident_matrices: usize,
    pub pairwise_distance_matrices: usize,
    pub pairwise_distance_values: usize,
    pub pairwise_distance_backend: ProfileMathBackend,
    pub pairwise_tile_rows: usize,
    pub pairwise_tiles: usize,
    pub measured_rows: usize,
    pub vector_dim: usize,
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
        let modality = registry.lens_modality(lens_id)?;
        let mut inputs = Vec::new();
        let mut labels = Vec::new();
        let mut failed = 0_usize;
        for probe in probes {
            if probe.input.modality == modality {
                inputs.push(probe.input.clone());
                labels.push(probe.label.clone());
            } else {
                failed += 1;
            }
        }
        let mut execution = ProfileExecutionStats {
            scalar_measure_calls: 0,
            ..ProfileExecutionStats::default()
        };
        let mut observations = Vec::new();
        if !inputs.is_empty() {
            execution.measurement_passes = 1;
            execution.batch_measure_calls = 1;
            let measured = registry.measure_batch(lens_id, &inputs)?;
            for (idx, vector) in measured.into_iter().enumerate() {
                match dense_projection(&vector)? {
                    Some(data) => observations.push(Observation {
                        data,
                        label: labels[idx].clone(),
                    }),
                    None => failed += 1,
                }
            }
        }
        let total_ms = started.elapsed().as_secs_f64() as f32 * 1000.0;
        let vram_after = vram_bytes();
        if observations.is_empty() {
            return Err(CalyxError::assay_insufficient_samples(
                "profile produced no measurable vectors",
            ));
        }

        let cost =
            CostMetrics::from_profile(total_ms, probes, &observations, vram_before, vram_after);
        let mut card = dense_capability_card(DenseCapabilityRequest {
            lens_id,
            probe_count: probes.len(),
            observations,
            cost,
            signal: None,
            signal_kind: registry_signal_kind(registry, lens_id),
            health: registry.health(lens_id)?,
            options: self.options,
            execution,
        })?;
        card.coverage.failed = failed;
        card.coverage.rate = card.coverage.measured as f32 / probes.len() as f32;
        Ok(card)
    }
}

fn vram_bytes() -> Option<u64> {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .output();
    let Ok(output) = output else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u64>().ok())
            .map(|mib| mib * 1024 * 1024)
            .sum(),
    )
}

pub fn profile_lens(
    registry: &Registry,
    lens_id: LensId,
    probes: &[ProfileProbe],
) -> Result<CapabilityCard> {
    Profiler::default().profile_lens(registry, lens_id, probes)
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
        SlotVector::Multi { .. } | SlotVector::SegmentedMulti { .. } => {
            Err(CalyxError::lens_dim_mismatch(
                "dense capability profiling cannot pool a multi-vector slot; use native block profiling",
            ))
        }
        SlotVector::Absent { .. } => Ok(None),
    }
}

#[cfg(test)]
mod tests;
