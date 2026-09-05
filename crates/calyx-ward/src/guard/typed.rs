use std::collections::BTreeMap;

use calyx_core::{SlotId, SlotSimilarityEvidence, SlotVector, slot_similarity};
use serde::{Deserialize, Serialize};

use super::{
    DEFAULT_TAU, required_slots, validate_high_stakes_profile, validate_non_inert_required,
};
use crate::error::WardError;
use crate::profile::{GuardId, GuardPolicy, GuardProfile, NoveltyAction};

pub type TypedProducedSlots = BTreeMap<SlotId, SlotVector>;
pub type TypedMatchedSlots = BTreeMap<SlotId, SlotVector>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypedSlotVerdict {
    pub slot: SlotId,
    pub cos: f32,
    pub tau: f32,
    pub pass: bool,
    pub evidence: SlotSimilarityEvidence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypedGuardVerdict {
    pub guard_id: GuardId,
    pub overall_pass: bool,
    #[serde(default)]
    pub provisional: bool,
    pub per_slot: Vec<TypedSlotVerdict>,
    pub action: Option<NoveltyAction>,
}

impl TypedGuardVerdict {
    pub fn failing_slots(&self) -> Vec<&TypedSlotVerdict> {
        self.per_slot.iter().filter(|slot| !slot.pass).collect()
    }
}

/// Gate every required slot through its original typed blocks.
pub fn guard_typed(
    profile: &GuardProfile,
    produced: &TypedProducedSlots,
    matched: &TypedMatchedSlots,
    high_stakes: bool,
) -> Result<TypedGuardVerdict, WardError> {
    let required = required_slots(profile);
    validate_non_inert_required(profile, &required)?;
    if high_stakes {
        validate_high_stakes_profile(profile, &required)?;
    }

    let mut per_slot = Vec::with_capacity(required.len());
    for slot in required {
        let produced_vector = produced.get(&slot).ok_or(WardError::MissingSlot { slot })?;
        let matched_vector = matched.get(&slot).ok_or(WardError::MissingSlot { slot })?;
        let evidence = slot_similarity(produced_vector, matched_vector).map_err(|error| {
            WardError::TypedSlotInvalid {
                slot,
                cause_code: error.code,
                message: error.message,
                remediation: error.remediation,
            }
        })?;
        let tau = profile.tau_for(&slot).unwrap_or(DEFAULT_TAU);
        per_slot.push(TypedSlotVerdict {
            slot,
            cos: evidence.score,
            tau,
            pass: evidence.score >= tau,
            evidence,
        });
    }

    let pass_count = per_slot.iter().filter(|slot| slot.pass).count();
    let overall_pass = match profile.policy {
        GuardPolicy::AllRequired => pass_count == per_slot.len(),
        GuardPolicy::KofN { k } => pass_count >= k,
    };
    let action = (!overall_pass).then(|| profile.novelty_action.clone());
    Ok(TypedGuardVerdict {
        guard_id: profile.guard_id,
        overall_pass,
        provisional: !profile.is_calibrated(),
        per_slot,
        action,
    })
}

pub fn guard_result_typed(
    profile: &GuardProfile,
    produced: &TypedProducedSlots,
    matched: &TypedMatchedSlots,
    high_stakes: bool,
) -> Result<TypedGuardVerdict, WardError> {
    let verdict = guard_typed(profile, produced, matched, high_stakes)?;
    if verdict.overall_pass {
        Ok(verdict)
    } else {
        Err(WardError::TypedOod {
            guard_id: verdict.guard_id,
            failing: verdict
                .per_slot
                .iter()
                .filter(|slot| !slot.pass)
                .cloned()
                .collect(),
        })
    }
}
