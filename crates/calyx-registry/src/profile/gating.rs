use std::env;

use calyx_assay::contract::{MAX_PAIRWISE_CORR, MIN_SIGNAL_BITS};
use calyx_core::{CalyxError, Clock, LedgerRef, LensId, Panel, Result, SlotId, SlotState};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, LedgerCfStore, SubjectId};
use serde::{Deserialize, Serialize};

use super::reliability::signal_floor;
use super::{CapabilityCard, CapabilitySignalKind, ProfileProbe, dense_projection};
use crate::Registry;

pub const CAPABILITY_MIN_SIGNAL_BITS_ENV: &str = "CALYX_CAPABILITY_MIN_SIGNAL_BITS";
pub const CAPABILITY_MAX_PAIRWISE_CORR_ENV: &str = "CALYX_CAPABILITY_MAX_PAIRWISE_CORR";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityGateThresholds {
    pub min_signal_bits: f32,
    pub max_pairwise_corr: f32,
}

impl Default for CapabilityGateThresholds {
    fn default() -> Self {
        Self {
            min_signal_bits: MIN_SIGNAL_BITS,
            max_pairwise_corr: MAX_PAIRWISE_CORR,
        }
    }
}

impl CapabilityGateThresholds {
    pub fn from_env() -> Result<Self> {
        let thresholds = Self {
            min_signal_bits: env_f32(CAPABILITY_MIN_SIGNAL_BITS_ENV, MIN_SIGNAL_BITS)?,
            max_pairwise_corr: env_f32(CAPABILITY_MAX_PAIRWISE_CORR_ENV, MAX_PAIRWISE_CORR)?,
        };
        thresholds.validate()?;
        Ok(thresholds)
    }

    pub fn validate(&self) -> Result<()> {
        if !self.min_signal_bits.is_finite() || self.min_signal_bits < 0.0 {
            return Err(CalyxError::assay_low_signal(
                "capability min_signal_bits must be finite and non-negative",
            ));
        }
        if !self.max_pairwise_corr.is_finite() || !(0.0..=1.0).contains(&self.max_pairwise_corr) {
            return Err(CalyxError::assay_redundant(
                "capability max_pairwise_corr must be finite in [0, 1]",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityGateDecision {
    Admit,
    Park,
    Retire,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityGateEvaluation {
    pub lens_id: LensId,
    pub decision: CapabilityGateDecision,
    pub signal_bits: f32,
    pub signal_grounded: bool,
    pub max_pairwise_corr: f32,
    pub thresholds: CapabilityGateThresholds,
    pub reason: String,
    pub card: CapabilityCard,
}

pub fn evaluate_capability_gate(
    card: CapabilityCard,
    max_pairwise_corr: f32,
    thresholds: CapabilityGateThresholds,
) -> Result<CapabilityGateEvaluation> {
    thresholds.validate()?;
    if !max_pairwise_corr.is_finite() || max_pairwise_corr < 0.0 {
        return Err(CalyxError::assay_redundant(
            "capability max_pairwise_corr must be finite and non-negative",
        ));
    }

    let signal_grounded = card.signal.is_some();
    let signal_bits = card.signal.unwrap_or(0.0);
    if !signal_bits.is_finite() || signal_bits < 0.0 {
        return Err(CalyxError::assay_low_signal(
            "capability signal_bits must be finite and non-negative",
        ));
    }
    let (signal_floor, reliability_reason) = signal_floor(&card)?;
    let corr = max_pairwise_corr.min(1.0);
    let (decision, reason) = if corr > thresholds.max_pairwise_corr {
        (
            CapabilityGateDecision::Retire,
            format!(
                "max_pairwise_corr {corr:.4} above {:.4}",
                thresholds.max_pairwise_corr
            ),
        )
    } else if !signal_grounded {
        (
            CapabilityGateDecision::Park,
            "missing grounded assay signal bits".to_string(),
        )
    } else if card.signal_kind != CapabilitySignalKind::LearnedEncoder {
        (
            CapabilityGateDecision::Park,
            format!(
                "signal kind {} cannot close a sufficiency deficit; need learned_encoder",
                card.signal_kind.as_str()
            ),
        )
    } else if card.low_spread {
        (
            CapabilityGateDecision::Park,
            "low spread/collapsed lens".to_string(),
        )
    } else if let Some(reason) = reliability_reason {
        (CapabilityGateDecision::Park, reason)
    } else if signal_floor < thresholds.min_signal_bits {
        (
            CapabilityGateDecision::Park,
            format!(
                "signal lower bound {signal_floor:.4} below {:.4}",
                thresholds.min_signal_bits
            ),
        )
    } else {
        (
            CapabilityGateDecision::Admit,
            "signal and correlation thresholds pass".to_string(),
        )
    };

    Ok(CapabilityGateEvaluation {
        lens_id: card.lens_id,
        decision,
        signal_bits,
        signal_grounded,
        max_pairwise_corr: corr,
        thresholds,
        reason,
        card,
    })
}

pub fn max_panel_pairwise_correlation(
    registry: &Registry,
    panel: &Panel,
    candidate_lens_id: LensId,
    exclude_slot_id: Option<SlotId>,
    probes: &[ProfileProbe],
) -> Result<f32> {
    let candidate = lens_signature(registry, candidate_lens_id, probes)?;
    let mut max_corr = 0.0_f32;
    for slot in panel
        .slots
        .iter()
        .filter(|slot| slot.state != SlotState::Retired)
    {
        if Some(slot.slot_id) == exclude_slot_id {
            continue;
        }
        let other = lens_signature(registry, slot.lens_id, probes)?;
        max_corr = max_corr.max(pearson_abs(&candidate, &other)?);
    }
    Ok(max_corr)
}

pub fn capability_gate_json(evaluation: &CapabilityGateEvaluation) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(evaluation)
        .map_err(|error| CalyxError::disk_pressure(format!("encode capability gate: {error}")))
}

pub fn append_capability_gate_ledger<S, C>(
    appender: &mut LedgerAppender<S, C>,
    evaluation: &CapabilityGateEvaluation,
    actor: ActorId,
) -> Result<LedgerRef>
where
    S: LedgerCfStore,
    C: Clock,
{
    appender.append(
        EntryKind::Assay,
        SubjectId::Lens(evaluation.lens_id),
        capability_gate_json(evaluation)?,
        actor,
    )
}

fn lens_signature(
    registry: &Registry,
    lens_id: LensId,
    probes: &[ProfileProbe],
) -> Result<Vec<f32>> {
    if probes.len() < 3 {
        return Err(CalyxError::assay_insufficient_samples(
            "capability correlation requires at least three probes",
        ));
    }
    let mut vectors = Vec::new();
    for probe in probes {
        let measured = registry.measure(lens_id, &probe.input)?;
        if let Some(vector) = dense_projection(&measured)? {
            vectors.push(vector);
        }
    }
    if vectors.len() < 3 {
        return Err(CalyxError::assay_insufficient_samples(
            "capability correlation produced fewer than three vectors",
        ));
    }
    ensure_same_dim(&vectors)?;
    let mut signature = Vec::new();
    for left in 0..vectors.len() {
        for right in (left + 1)..vectors.len() {
            signature.push(euclidean(&vectors[left], &vectors[right]));
        }
    }
    Ok(signature)
}

fn ensure_same_dim(vectors: &[Vec<f32>]) -> Result<()> {
    let dim = vectors[0].len();
    if dim == 0 || vectors.iter().any(|vector| vector.len() != dim) {
        return Err(CalyxError::lens_dim_mismatch(
            "capability correlation vectors have inconsistent dimensions",
        ));
    }
    Ok(())
}

fn pearson_abs(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() || left.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "capability correlation signatures are not comparable",
        ));
    }
    let left_mean = mean(left);
    let right_mean = mean(right);
    let mut covariance = 0.0_f32;
    let mut left_ss = 0.0_f32;
    let mut right_ss = 0.0_f32;
    for (left_value, right_value) in left.iter().zip(right) {
        if !left_value.is_finite() || !right_value.is_finite() {
            return Err(CalyxError::lens_numerical_invariant(
                "capability correlation signature contains non-finite value",
            ));
        }
        let left_delta = *left_value - left_mean;
        let right_delta = *right_value - right_mean;
        covariance += left_delta * right_delta;
        left_ss += left_delta * left_delta;
        right_ss += right_delta * right_delta;
    }
    let denom = (left_ss * right_ss).sqrt();
    if denom <= f32::EPSILON {
        return Ok(0.0);
    }
    Ok((covariance / denom).abs().clamp(0.0, 1.0))
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len() as f32
}

fn euclidean(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| {
            let delta = *left - *right;
            delta * delta
        })
        .sum::<f32>()
        .sqrt()
}

fn env_f32(name: &str, default: f32) -> Result<f32> {
    match env::var(name) {
        Ok(raw) => raw.parse().map_err(|error| {
            CalyxError::assay_insufficient_samples(format!("parse {name}={raw}: {error}"))
        }),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(CalyxError::assay_insufficient_samples(format!(
            "read {name}: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use calyx_core::{
        Asymmetry, Input, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape,
        SlotState,
    };

    use super::*;
    use crate::AlgorithmicLens;
    use crate::profile::{
        CostMetrics, CoverageMetrics, MetricSource, SeparationMetrics, SpreadMetrics,
    };

    #[test]
    fn gate_decisions_match_threshold_contract() {
        let thresholds = CapabilityGateThresholds::default();

        let admit = evaluate_capability_gate(card(Some(0.08), false), 0.10, thresholds).unwrap();
        let park = evaluate_capability_gate(card(Some(0.01), false), 0.10, thresholds).unwrap();
        let retire = evaluate_capability_gate(card(Some(0.20), false), 0.90, thresholds).unwrap();

        assert_eq!(admit.decision, CapabilityGateDecision::Admit);
        assert_eq!(park.decision, CapabilityGateDecision::Park);
        assert_eq!(retire.decision, CapabilityGateDecision::Retire);
    }

    #[test]
    fn missing_grounded_bits_and_collapsed_lens_park() {
        let thresholds = CapabilityGateThresholds::default();

        let missing = evaluate_capability_gate(card(None, false), 0.0, thresholds).unwrap();
        let collapsed = evaluate_capability_gate(card(Some(0.20), true), 0.0, thresholds).unwrap();

        assert_eq!(missing.decision, CapabilityGateDecision::Park);
        assert_eq!(missing.signal_bits, 0.0);
        assert!(!missing.signal_grounded);
        assert_eq!(collapsed.decision, CapabilityGateDecision::Park);
        assert!(collapsed.reason.contains("low spread"));
    }

    #[test]
    fn non_learned_grounded_signal_parks() {
        let thresholds = CapabilityGateThresholds::default();
        let mut candidate = card(Some(0.20), false);
        candidate.signal_kind = CapabilitySignalKind::Placeholder;

        let evaluation = evaluate_capability_gate(candidate, 0.0, thresholds).unwrap();

        assert_eq!(evaluation.decision, CapabilityGateDecision::Park);
        assert!(evaluation.reason.contains("placeholder"));
    }

    #[test]
    fn pearson_abs_matches_hand_computed_cases() {
        let duplicate = pearson_abs(&[1.0, 2.0, 1.0], &[1.0, 2.0, 1.0]).unwrap();
        let orthogonal = pearson_abs(&[-1.0, 0.0, 1.0], &[1.0, 0.0, 1.0]).unwrap();

        assert!((duplicate - 1.0).abs() <= 1e-6);
        assert!(orthogonal <= 1e-6);
    }

    #[test]
    fn panel_correlation_uses_real_probe_vectors() {
        let mut registry = Registry::new();
        let panel_lens = AlgorithmicLens::byte_features("gate-panel", Modality::Text);
        let candidate_lens = AlgorithmicLens::byte_features("gate-candidate", Modality::Text);
        let panel_id = registry
            .register_frozen(panel_lens.clone(), panel_lens.contract().clone())
            .unwrap();
        let candidate_id = registry
            .register_frozen(candidate_lens.clone(), candidate_lens.contract().clone())
            .unwrap();
        let panel = Panel {
            version: 1,
            slots: vec![slot(panel_id)],
            created_at: 1,
            kernel_ref: None,
            guard_ref: None,
        };
        let probes = vec![
            ProfileProbe::new(Input::new(Modality::Text, b"alpha".to_vec())),
            ProfileProbe::new(Input::new(Modality::Text, b"beta".to_vec())),
            ProfileProbe::new(Input::new(Modality::Text, b"gamma".to_vec())),
            ProfileProbe::new(Input::new(Modality::Text, b"delta".to_vec())),
        ];

        let corr =
            max_panel_pairwise_correlation(&registry, &panel, candidate_id, None, &probes).unwrap();

        assert!((corr - 1.0).abs() <= 1e-6);
    }

    #[test]
    fn empty_probe_correlation_fails_closed() {
        let registry = Registry::new();
        let panel = Panel {
            version: 1,
            slots: Vec::new(),
            created_at: 1,
            kernel_ref: None,
            guard_ref: None,
        };

        let error = max_panel_pairwise_correlation(
            &registry,
            &panel,
            LensId::from_bytes([9; 16]),
            None,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    }

    fn card(signal: Option<f32>, low_spread: bool) -> CapabilityCard {
        CapabilityCard {
            lens_id: LensId::from_bytes([3; 16]),
            probe_count: 4,
            signal,
            signal_source: if signal.is_some() {
                MetricSource::AssayStore
            } else {
                MetricSource::AssayPending
            },
            signal_kind: CapabilitySignalKind::LearnedEncoder,
            signal_reliability: None,
            proxy_signal: signal.unwrap_or(0.0),
            differentiation: Some(0.07),
            differentiation_source: MetricSource::AssayStore,
            proxy_differentiation: 0.5,
            spread: SpreadMetrics {
                participation_ratio: if low_spread { 0.0 } else { 2.0 },
                normalized_participation_ratio: if low_spread { 0.0 } else { 0.5 },
                stable_rank: if low_spread { 0.0 } else { 2.0 },
                total_variance: if low_spread { 0.0 } else { 1.0 },
                mean_pairwise_distance: if low_spread { 0.0 } else { 1.0 },
            },
            separation: SeparationMetrics {
                score: 0.5,
                silhouette: 0.5,
                mean_pairwise_distance: 1.0,
                labeled_groups: 2,
                used_labels: true,
            },
            cost: CostMetrics {
                total_ms: 1.0,
                ms_per_input: 1.0,
                vram_bytes: 0,
                ram_bytes: 0,
                batch_ceiling: 1_000,
            },
            coverage: CoverageMetrics {
                requested: 4,
                measured: 4,
                failed: 0,
                rate: 1.0,
            },
            health: crate::LensHealth::Loaded,
            low_spread,
        }
    }

    fn slot(lens_id: LensId) -> Slot {
        let slot_id = SlotId::new(0);
        Slot {
            slot_id,
            slot_key: SlotKey::new(slot_id, "gate-panel".to_string()),
            lens_id,
            shape: SlotShape::Dense(4),
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: None,
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: Default::default(),
            state: SlotState::Active,
            added_at_panel_version: 1,
        }
    }
}
