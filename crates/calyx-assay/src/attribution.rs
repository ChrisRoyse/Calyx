//! Per-sensor signal attribution and bits reports.

use calyx_core::SlotId;
use serde::{Deserialize, Serialize};

use crate::estimate::TrustTag;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotAttribution {
    pub slot: SlotId,
    pub marginal_bits: f32,
    pub sole_carrier: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BitsReport {
    pub slots: Vec<SlotAttribution>,
    pub total_bits: f32,
    pub trust: TrustTag,
}

pub fn per_sensor_attribution(
    slot_bits: &[(SlotId, f32)],
    sole_threshold_bits: f32,
) -> Vec<SlotAttribution> {
    let strong_slots = slot_bits
        .iter()
        .filter(|(_, bits)| *bits >= sole_threshold_bits)
        .count();
    slot_bits
        .iter()
        .map(|(slot, bits)| SlotAttribution {
            slot: *slot,
            marginal_bits: *bits,
            sole_carrier: *bits >= sole_threshold_bits && strong_slots == 1,
        })
        .collect()
}

pub fn bits_report(slots: Vec<SlotAttribution>, trust: TrustTag) -> BitsReport {
    BitsReport {
        total_bits: slots.iter().map(|slot| slot.marginal_bits).sum(),
        slots,
        trust,
    }
}
