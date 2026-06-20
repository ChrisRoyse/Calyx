//! Panel sufficiency and deficit routing.

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{Anchor, AnchorKind, CalyxError, Result, SlotId};
use serde::{Deserialize, Serialize};

use crate::attribution::SlotAttribution;
use crate::calibration::{PowerCalibration, PowerCalibrationStatus, underpowered};
use crate::estimate::{
    EstimateBound, MiEstimate, TrustTag, provisional_without_anchor, trust_for_anchor,
};

pub const CALYX_ASSAY_INVALID_SCOPE: &str = "CALYX_ASSAY_INVALID_SCOPE";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeficitSuggestedAction {
    AddOutcomeAnchor,
    ProposeLens,
    IncreaseSamples,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeficitRoutingContext {
    pub panel_id: String,
    pub anchor: AnchorKind,
    pub computed_at_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_scope: Option<ObservationScope>,
}

impl Default for DeficitRoutingContext {
    fn default() -> Self {
        Self {
            panel_id: "panel:unspecified".to_string(),
            anchor: AnchorKind::Reward,
            computed_at_seq: 0,
            observation_scope: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationScope {
    pub id: String,
    pub observed: usize,
    pub total: usize,
}

impl ObservationScope {
    pub fn new(id: impl Into<String>, observed: usize, total: usize) -> Result<ObservationScope> {
        let scope = Self {
            id: id.into(),
            observed,
            total,
        };
        scope.validate()?;
        Ok(scope)
    }

    pub fn coverage_rate(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.observed as f32 / self.total as f32
        }
    }

    fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(invalid_scope("observation scope id must not be empty"));
        }
        if self.observed > self.total {
            return Err(invalid_scope(format!(
                "scope {} observed {} rows but total is {}",
                self.id, self.observed, self.total
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SufficiencyDeficit {
    pub panel_id: String,
    pub anchor: AnchorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_scope: Option<ObservationScope>,
    pub slot: Option<SlotId>,
    pub per_slot_gaps: BTreeMap<SlotId, f32>,
    pub deficit_bits: f32,
    pub suggested_action: DeficitSuggestedAction,
    pub computed_at_seq: u64,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelSufficiency {
    pub panel_bits: f32,
    pub sufficiency_basis_bits: f32,
    pub anchor_entropy_bits: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_scope: Option<ObservationScope>,
    pub sufficient: bool,
    pub deficit_bits: f32,
    pub deficits: Vec<SufficiencyDeficit>,
    pub trust: TrustTag,
    pub estimate_bound: EstimateBound,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_calibration: Option<PowerCalibration>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SufficiencyScopeInput {
    pub scope: ObservationScope,
    pub panel_bits: f32,
    pub anchor_entropy_bits: f32,
    pub slots: Vec<SlotAttribution>,
    pub trust: TrustTag,
    pub context: DeficitRoutingContext,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScopedSufficiencyReport {
    pub scopes: Vec<PanelSufficiency>,
    pub best_scope: Option<ObservationScope>,
    pub sufficient_scopes: Vec<ObservationScope>,
}

pub trait SufficiencyDeficitSink {
    fn record_deficit(&mut self, deficit: SufficiencyDeficit);
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InMemoryDeficitSink {
    pub routed: Vec<SufficiencyDeficit>,
}

impl SufficiencyDeficitSink for InMemoryDeficitSink {
    fn record_deficit(&mut self, deficit: SufficiencyDeficit) {
        self.routed.push(deficit);
    }
}

impl PanelSufficiency {
    pub fn route_to<S: SufficiencyDeficitSink>(&self, sink: &mut S) {
        for deficit in &self.deficits {
            sink.record_deficit(deficit.clone());
        }
    }
}

pub fn panel_sufficiency(
    panel_bits: f32,
    anchor_entropy_bits: f32,
    slots: &[SlotAttribution],
    trust: TrustTag,
) -> PanelSufficiency {
    panel_sufficiency_with_trust(
        panel_bits,
        anchor_entropy_bits,
        slots,
        provisional_without_anchor(trust),
        DeficitRoutingContext::default(),
    )
}

pub fn panel_sufficiency_from_estimate(
    estimate: &MiEstimate,
    anchor_entropy_bits: f32,
    slots: &[SlotAttribution],
    trust: TrustTag,
) -> Result<PanelSufficiency> {
    let calibration = passing_calibration(estimate)?;
    Ok(panel_sufficiency_with_trust_and_basis(
        SufficiencyBasis {
            panel_bits: estimate.bits,
            sufficiency_basis_bits: estimate.ci_low,
            estimate_bound: estimate.bound,
            power_calibration: Some(calibration),
        },
        anchor_entropy_bits,
        slots,
        provisional_without_anchor(trust),
        DeficitRoutingContext::default(),
    ))
}

pub fn panel_sufficiency_with_anchor(
    panel_bits: f32,
    anchor_entropy_bits: f32,
    slots: &[SlotAttribution],
    anchor: &Anchor,
) -> PanelSufficiency {
    panel_sufficiency_with_trust(
        panel_bits,
        anchor_entropy_bits,
        slots,
        trust_for_anchor(Some(anchor)),
        DeficitRoutingContext::default(),
    )
}

pub fn panel_sufficiency_with_context(
    panel_bits: f32,
    anchor_entropy_bits: f32,
    slots: &[SlotAttribution],
    trust: TrustTag,
    context: DeficitRoutingContext,
) -> PanelSufficiency {
    panel_sufficiency_with_trust(
        panel_bits,
        anchor_entropy_bits,
        slots,
        provisional_without_anchor(trust),
        context,
    )
}

pub fn panel_sufficiency_with_anchor_and_context(
    panel_bits: f32,
    anchor_entropy_bits: f32,
    slots: &[SlotAttribution],
    anchor: &Anchor,
    context: DeficitRoutingContext,
) -> PanelSufficiency {
    panel_sufficiency_with_trust(
        panel_bits,
        anchor_entropy_bits,
        slots,
        trust_for_anchor(Some(anchor)),
        context,
    )
}

pub fn panel_sufficiency_by_scope(
    inputs: Vec<SufficiencyScopeInput>,
) -> Result<ScopedSufficiencyReport> {
    if inputs.is_empty() {
        return Err(invalid_scope(
            "sufficiency scope report requires at least one scope",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut scopes = Vec::with_capacity(inputs.len());
    for input in inputs {
        input.scope.validate()?;
        if !seen.insert(input.scope.id.clone()) {
            return Err(invalid_scope(format!(
                "duplicate observation scope {}",
                input.scope.id
            )));
        }
        let mut context = input.context;
        context.observation_scope = Some(input.scope);
        scopes.push(panel_sufficiency_with_trust(
            input.panel_bits,
            input.anchor_entropy_bits,
            &input.slots,
            provisional_without_anchor(input.trust),
            context,
        ));
    }
    let best_scope = scopes
        .iter()
        .min_by(|left, right| left.deficit_bits.total_cmp(&right.deficit_bits))
        .and_then(|scope| scope.observation_scope.clone());
    let sufficient_scopes = scopes
        .iter()
        .filter(|scope| scope.sufficient)
        .filter_map(|scope| scope.observation_scope.clone())
        .collect();
    Ok(ScopedSufficiencyReport {
        scopes,
        best_scope,
        sufficient_scopes,
    })
}

fn panel_sufficiency_with_trust(
    panel_bits: f32,
    anchor_entropy_bits: f32,
    slots: &[SlotAttribution],
    trust: TrustTag,
    context: DeficitRoutingContext,
) -> PanelSufficiency {
    panel_sufficiency_with_trust_and_basis(
        SufficiencyBasis::diagnostic(panel_bits),
        anchor_entropy_bits,
        slots,
        trust,
        context,
    )
}

struct SufficiencyBasis {
    panel_bits: f32,
    sufficiency_basis_bits: f32,
    estimate_bound: EstimateBound,
    power_calibration: Option<PowerCalibration>,
}

impl SufficiencyBasis {
    fn diagnostic(panel_bits: f32) -> Self {
        Self {
            panel_bits,
            sufficiency_basis_bits: panel_bits,
            estimate_bound: EstimateBound::LowerBound,
            power_calibration: None,
        }
    }
}

fn panel_sufficiency_with_trust_and_basis(
    basis: SufficiencyBasis,
    anchor_entropy_bits: f32,
    slots: &[SlotAttribution],
    trust: TrustTag,
    context: DeficitRoutingContext,
) -> PanelSufficiency {
    let deficit_bits = (anchor_entropy_bits - basis.sufficiency_basis_bits).max(0.0);
    let sufficient = basis.sufficiency_basis_bits >= anchor_entropy_bits;
    let deficits = if sufficient {
        Vec::new()
    } else {
        localized_deficits(deficit_bits, slots, &context)
    };
    PanelSufficiency {
        panel_bits: basis.panel_bits,
        sufficiency_basis_bits: basis.sufficiency_basis_bits,
        anchor_entropy_bits,
        observation_scope: context.observation_scope.clone(),
        sufficient,
        deficit_bits,
        deficits,
        trust,
        estimate_bound: basis.estimate_bound,
        power_calibration: basis.power_calibration,
    }
}

fn passing_calibration(estimate: &MiEstimate) -> Result<PowerCalibration> {
    let calibration = estimate.power_calibration.clone().ok_or_else(|| {
        underpowered("panel sufficiency requires a passing planted-signal power calibration")
    })?;
    if calibration.status != PowerCalibrationStatus::Passed {
        return Err(underpowered(format!(
            "panel sufficiency estimator calibration status is {:?}",
            calibration.status
        )));
    }
    calibration.require_passed()?;
    Ok(calibration)
}

pub fn entropy_bits<T>(labels: &[T]) -> f32
where
    T: Ord + Copy,
{
    let mut counts = BTreeMap::<T, usize>::new();
    for label in labels {
        *counts.entry(*label).or_default() += 1;
    }
    let n = labels.len().max(1) as f32;
    counts
        .values()
        .map(|count| {
            let p = *count as f32 / n;
            -p * p.log2()
        })
        .sum()
}

fn localized_deficits(
    deficit_bits: f32,
    slots: &[SlotAttribution],
    context: &DeficitRoutingContext,
) -> Vec<SufficiencyDeficit> {
    if slots.is_empty() {
        return vec![SufficiencyDeficit {
            panel_id: context.panel_id.clone(),
            anchor: context.anchor.clone(),
            observation_scope: context.observation_scope.clone(),
            slot: None,
            per_slot_gaps: BTreeMap::new(),
            deficit_bits,
            suggested_action: DeficitSuggestedAction::AddOutcomeAnchor,
            computed_at_seq: context.computed_at_seq,
            reason: "panel below anchor entropy".to_string(),
        }];
    }
    let per_slot_gaps = per_slot_gap_map(deficit_bits, slots);
    let total_missing_weight: f32 = slots
        .iter()
        .map(|slot| 1.0 / (slot.marginal_bits + 0.01))
        .sum();
    slots
        .iter()
        .map(|slot| {
            let weight = 1.0 / (slot.marginal_bits + 0.01);
            SufficiencyDeficit {
                panel_id: context.panel_id.clone(),
                anchor: context.anchor.clone(),
                observation_scope: context.observation_scope.clone(),
                slot: Some(slot.slot),
                per_slot_gaps: per_slot_gaps.clone(),
                deficit_bits: deficit_bits * weight / total_missing_weight,
                suggested_action: DeficitSuggestedAction::ProposeLens,
                computed_at_seq: context.computed_at_seq,
                reason: "slot marginal bits below sufficiency need".to_string(),
            }
        })
        .collect()
}

fn per_slot_gap_map(deficit_bits: f32, slots: &[SlotAttribution]) -> BTreeMap<SlotId, f32> {
    let total_missing_weight: f32 = slots
        .iter()
        .map(|slot| 1.0 / (slot.marginal_bits + 0.01))
        .sum();
    slots
        .iter()
        .map(|slot| {
            let weight = 1.0 / (slot.marginal_bits + 0.01);
            (slot.slot, deficit_bits * weight / total_missing_weight)
        })
        .collect()
}

fn invalid_scope(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_INVALID_SCOPE,
        message: message.into(),
        remediation: "provide unique observation scopes with observed <= total",
    }
}
