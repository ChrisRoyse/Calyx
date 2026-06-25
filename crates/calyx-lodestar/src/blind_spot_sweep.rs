use calyx_core::{CxId, SlotId};
use calyx_loom::{BlindSpotAlert, Severity, detect_blind_spot};
use serde::{Deserialize, Serialize};

use crate::{LodestarError, Result};

pub const BLIND_SPOT_SWEEP_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindSpotSweepParams {
    pub min_severity: Severity,
    pub min_gate_confidence: f32,
    pub max_candidates: usize,
}

impl Default for BlindSpotSweepParams {
    fn default() -> Self {
        Self {
            min_severity: Severity::High,
            min_gate_confidence: 0.25,
            max_candidates: 128,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindSpotNeighbor {
    pub cx_id: CxId,
    pub text: String,
    pub similarity: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindSpotGateVerdict {
    pub passed: bool,
    pub confidence: f32,
    pub code: String,
    pub reason: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindSpotObservation {
    pub cx_id: CxId,
    pub text: String,
    pub lens_a: SlotId,
    pub lens_b: SlotId,
    pub lens_a_similarity: f32,
    pub lens_b_neighbor_mean: f32,
    pub lens_a_neighbors: Vec<BlindSpotNeighbor>,
    pub lens_b_neighbors: Vec<BlindSpotNeighbor>,
    pub gate: BlindSpotGateVerdict,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindSpotCandidate {
    pub alert: BlindSpotAlert,
    pub text: String,
    pub lens_a_similarity: f32,
    pub lens_b_neighbor_mean: f32,
    pub gate: BlindSpotGateVerdict,
    pub lens_a_neighbors: Vec<BlindSpotNeighbor>,
    pub lens_b_neighbors: Vec<BlindSpotNeighbor>,
    pub rank_score: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindSpotSweepLog {
    pub schema_version: u32,
    pub observation_count: usize,
    pub detected_alert_count: usize,
    pub gate_refused_count: usize,
    pub severity_filtered_count: usize,
    pub candidates: Vec<BlindSpotCandidate>,
}

pub fn sweep_blind_spots(
    observations: &[BlindSpotObservation],
    params: &BlindSpotSweepParams,
) -> Result<BlindSpotSweepLog> {
    validate_params(params)?;
    let mut detected_alert_count = 0_usize;
    let mut gate_refused_count = 0_usize;
    let mut severity_filtered_count = 0_usize;
    let mut candidates = Vec::new();
    for observation in observations {
        validate_observation(observation)?;
        let Some(alert) = detect_blind_spot(
            observation.cx_id,
            observation.lens_a,
            observation.lens_b,
            observation.lens_a_similarity,
            observation.lens_b_neighbor_mean,
        ) else {
            continue;
        };
        detected_alert_count += 1;
        if severity_rank(alert.severity) < severity_rank(params.min_severity) {
            severity_filtered_count += 1;
            continue;
        }
        if !observation.gate.passed || observation.gate.confidence < params.min_gate_confidence {
            gate_refused_count += 1;
            continue;
        }
        candidates.push(candidate_from_observation(observation, alert));
    }
    sort_candidates(&mut candidates);
    candidates.truncate(params.max_candidates);
    Ok(BlindSpotSweepLog {
        schema_version: BLIND_SPOT_SWEEP_SCHEMA_VERSION,
        observation_count: observations.len(),
        detected_alert_count,
        gate_refused_count,
        severity_filtered_count,
        candidates,
    })
}

fn candidate_from_observation(
    observation: &BlindSpotObservation,
    alert: BlindSpotAlert,
) -> BlindSpotCandidate {
    let rank_score = alert.delta * 0.70 + observation.gate.confidence * 0.30;
    BlindSpotCandidate {
        alert,
        text: observation.text.clone(),
        lens_a_similarity: observation.lens_a_similarity,
        lens_b_neighbor_mean: observation.lens_b_neighbor_mean,
        gate: observation.gate.clone(),
        lens_a_neighbors: observation.lens_a_neighbors.clone(),
        lens_b_neighbors: observation.lens_b_neighbors.clone(),
        rank_score,
    }
}

fn sort_candidates(candidates: &mut [BlindSpotCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .rank_score
            .total_cmp(&left.rank_score)
            .then_with(|| right.alert.delta.total_cmp(&left.alert.delta))
            .then_with(|| {
                left.alert
                    .cx_id
                    .as_bytes()
                    .cmp(right.alert.cx_id.as_bytes())
            })
            .then_with(|| left.alert.a.get().cmp(&right.alert.a.get()))
            .then_with(|| left.alert.b.get().cmp(&right.alert.b.get()))
    });
}

fn validate_params(params: &BlindSpotSweepParams) -> Result<()> {
    if !params.min_gate_confidence.is_finite() || !(0.0..=1.0).contains(&params.min_gate_confidence)
    {
        return invalid_params("min_gate_confidence must be finite and in [0,1]");
    }
    if params.max_candidates == 0 {
        return invalid_params("max_candidates must be greater than zero");
    }
    Ok(())
}

fn validate_observation(observation: &BlindSpotObservation) -> Result<()> {
    validate_metric("lens_a_similarity", observation.lens_a_similarity)?;
    validate_metric("lens_b_neighbor_mean", observation.lens_b_neighbor_mean)?;
    validate_metric("gate.confidence", observation.gate.confidence)?;
    if !(0.0..=1.0).contains(&observation.gate.confidence) {
        return invalid_params("gate confidence must be in [0,1]");
    }
    if observation.gate.code.trim().is_empty() {
        return invalid_params("gate code must not be empty");
    }
    for neighbor in observation
        .lens_a_neighbors
        .iter()
        .chain(&observation.lens_b_neighbors)
    {
        validate_metric("neighbor.similarity", neighbor.similarity)?;
    }
    Ok(())
}

fn validate_metric(field: &'static str, value: f32) -> Result<()> {
    if value.is_finite() {
        Ok(())
    } else {
        invalid_params(format!("{field} must be finite"))
    }
}

fn invalid_params<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::KernelInvalidParams {
        detail: detail.into(),
    })
}

fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Low => 1,
        Severity::Medium => 2,
        Severity::High => 3,
    }
}
