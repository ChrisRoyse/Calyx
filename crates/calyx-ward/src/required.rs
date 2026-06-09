//! Assay-bit derived required-slot selection for Ward profiles.

use std::collections::BTreeMap;

use calyx_core::{AnchorKind, Panel, SlotId, SlotState};
use serde::{Deserialize, Serialize};

use crate::error::WardError;
use crate::guard::DEFAULT_TAU;
use crate::profile::GuardProfile;

pub const LOAD_BEARING_MIN_BITS: f32 = 0.05;

/// Explicit configuration for deriving a guard profile's required slots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequiredSlotDerivation {
    pub anchor: AnchorKind,
    pub min_bits: f32,
    pub cold_start_tau: f32,
    pub manual_required_slots: Option<Vec<SlotId>>,
}

impl RequiredSlotDerivation {
    /// Uses Assay `Slot.bits_about[anchor]` with the paper's 0.05-bit threshold.
    pub fn assay_bits(anchor: AnchorKind) -> Self {
        Self {
            anchor,
            min_bits: LOAD_BEARING_MIN_BITS,
            cold_start_tau: DEFAULT_TAU,
            manual_required_slots: None,
        }
    }

    /// Replaces derived slots with an explicit operator-supplied slot set.
    pub fn manual(anchor: AnchorKind, slots: Vec<SlotId>) -> Self {
        Self {
            anchor,
            min_bits: LOAD_BEARING_MIN_BITS,
            cold_start_tau: DEFAULT_TAU,
            manual_required_slots: Some(slots),
        }
    }
}

/// Audit record showing why a slot became load-bearing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequiredSlotEvidence {
    pub slot: SlotId,
    pub bits: f32,
}

/// Reads Assay bits from a panel and returns active load-bearing slots.
pub fn derive_required_slots(
    panel: &Panel,
    config: &RequiredSlotDerivation,
) -> Result<Vec<RequiredSlotEvidence>, WardError> {
    validate_config(config)?;
    let mut slots = BTreeMap::<SlotId, f32>::new();
    for slot in &panel.slots {
        if slot.state != SlotState::Active {
            continue;
        }
        let Some(signal) = slot.bits_about.get(&config.anchor) else {
            continue;
        };
        if !signal.bits.is_finite() {
            return Err(WardError::InvalidRequiredSlotDerivation {
                reason: "slot bits_about must be finite",
            });
        }
        if signal.bits >= config.min_bits {
            slots
                .entry(slot.slot_id)
                .and_modify(|bits| *bits = bits.max(signal.bits))
                .or_insert(signal.bits);
        }
    }
    Ok(slots
        .into_iter()
        .map(|(slot, bits)| RequiredSlotEvidence { slot, bits })
        .collect())
}

/// Applies Assay-derived or explicit required slots to a profile template.
///
/// Existing calibrated tau entries are preserved. Slots without tau receive the
/// configured cold-start tau so every required slot has an explicit threshold.
pub fn derive_required_profile(
    mut profile: GuardProfile,
    panel: &Panel,
    config: &RequiredSlotDerivation,
) -> Result<GuardProfile, WardError> {
    validate_config(config)?;
    let required_slots = match &config.manual_required_slots {
        Some(slots) => normalize_slots(slots.clone()),
        None => {
            let evidence = derive_required_slots(panel, config)?;
            if evidence.is_empty() {
                return Err(WardError::InvalidRequiredSlotDerivation {
                    reason: "no load-bearing slots for anchor",
                });
            }
            evidence.into_iter().map(|entry| entry.slot).collect()
        }
    };

    for slot in &required_slots {
        profile.tau.entry(*slot).or_insert(config.cold_start_tau);
    }
    profile.required_slots = required_slots;
    profile.panel_version = u64::from(panel.version);
    Ok(profile)
}

fn validate_config(config: &RequiredSlotDerivation) -> Result<(), WardError> {
    if !config.min_bits.is_finite() || config.min_bits < 0.0 {
        return Err(WardError::InvalidRequiredSlotDerivation {
            reason: "min_bits must be finite and non-negative",
        });
    }
    if !config.cold_start_tau.is_finite() || !(0.0..=1.0).contains(&config.cold_start_tau) {
        return Err(WardError::InvalidRequiredSlotDerivation {
            reason: "cold_start_tau must be finite and in [0,1]",
        });
    }
    Ok(())
}

fn normalize_slots(mut slots: Vec<SlotId>) -> Vec<SlotId> {
    slots.sort_unstable();
    slots.dedup();
    slots
}
