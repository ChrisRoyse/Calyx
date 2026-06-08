//! Cross-term materialization policy.

use calyx_core::{Result, SlotId};
use serde::{Deserialize, Serialize};

use crate::cross_term::CrossTermKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializationAction {
    EagerStore,
    LazyCache,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterializationEntry {
    pub a: SlotId,
    pub b: SlotId,
    pub kind: CrossTermKind,
    pub action: MaterializationAction,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MaterializationPlan {
    pub entries: Vec<MaterializationEntry>,
}

impl MaterializationPlan {
    pub fn materialized_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.action == MaterializationAction::EagerStore)
            .count()
    }
}

pub trait PairGainGate {
    fn pair_gain_bits(&self, a: SlotId, b: SlotId) -> f32;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StaticPairGainGate {
    pub gain_bits: f32,
}

impl PairGainGate for StaticPairGainGate {
    fn pair_gain_bits(&self, _a: SlotId, _b: SlotId) -> f32 {
        self.gain_bits
    }
}

pub fn plan_cross_terms(slots: &[SlotId], gate: &dyn PairGainGate) -> MaterializationPlan {
    plan_cross_terms_checked(slots, |a, b| Ok(gate.pair_gain_bits(a, b)))
        .expect("infallible PairGainGate adapter")
}

pub fn plan_cross_terms_checked<F>(
    slots: &[SlotId],
    mut pair_gain: F,
) -> Result<MaterializationPlan>
where
    F: FnMut(SlotId, SlotId) -> Result<f32>,
{
    let mut entries = Vec::new();
    for i in 0..slots.len() {
        for j in i + 1..slots.len() {
            let a = slots[i];
            let b = slots[j];
            let gain_bits = pair_gain(a, b)?;
            entries.push(MaterializationEntry {
                a,
                b,
                kind: CrossTermKind::Agreement,
                action: MaterializationAction::EagerStore,
            });
            entries.push(MaterializationEntry {
                a,
                b,
                kind: CrossTermKind::Delta,
                action: MaterializationAction::LazyCache,
            });
            entries.push(MaterializationEntry {
                a,
                b,
                kind: CrossTermKind::Interaction,
                action: if gain_bits >= 0.05 {
                    MaterializationAction::EagerStore
                } else {
                    MaterializationAction::LazyCache
                },
            });
            entries.push(MaterializationEntry {
                a,
                b,
                kind: CrossTermKind::Concat,
                action: MaterializationAction::LazyCache,
            });
        }
    }
    Ok(MaterializationPlan { entries })
}
