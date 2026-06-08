//! Panel sufficiency and deficit routing.

use std::collections::BTreeMap;

use calyx_core::SlotId;
use serde::{Deserialize, Serialize};

use crate::attribution::SlotAttribution;
use crate::estimate::TrustTag;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SufficiencyDeficit {
    pub slot: Option<SlotId>,
    pub deficit_bits: f32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelSufficiency {
    pub panel_bits: f32,
    pub anchor_entropy_bits: f32,
    pub sufficient: bool,
    pub deficit_bits: f32,
    pub deficits: Vec<SufficiencyDeficit>,
    pub trust: TrustTag,
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
    let deficit_bits = (anchor_entropy_bits - panel_bits).max(0.0);
    let sufficient = deficit_bits <= 1.0e-6;
    let deficits = if sufficient {
        Vec::new()
    } else {
        localized_deficits(deficit_bits, slots)
    };
    PanelSufficiency {
        panel_bits,
        anchor_entropy_bits,
        sufficient,
        deficit_bits,
        deficits,
        trust,
    }
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

fn localized_deficits(deficit_bits: f32, slots: &[SlotAttribution]) -> Vec<SufficiencyDeficit> {
    if slots.is_empty() {
        return vec![SufficiencyDeficit {
            slot: None,
            deficit_bits,
            reason: "panel below anchor entropy".to_string(),
        }];
    }
    let total_missing_weight: f32 = slots
        .iter()
        .map(|slot| 1.0 / (slot.marginal_bits + 0.01))
        .sum();
    slots
        .iter()
        .map(|slot| {
            let weight = 1.0 / (slot.marginal_bits + 0.01);
            SufficiencyDeficit {
                slot: Some(slot.slot),
                deficit_bits: deficit_bits * weight / total_missing_weight,
                reason: "slot marginal bits below sufficiency need".to_string(),
            }
        })
        .collect()
}
