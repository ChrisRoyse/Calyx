//! Identity-locked Ward profile wrappers.

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{AnchorKind, SlotId};
use serde::{Deserialize, Serialize};

use crate::error::WardError;
use crate::guard::MatchedSlots;
use crate::profile::GuardProfile;

/// One identity-bearing slot guarded during speaker/style generation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IdentitySlotConfig {
    pub slot_id: SlotId,
    pub anchor_kind: AnchorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tau_override: Option<f32>,
}

impl IdentitySlotConfig {
    /// Returns true for PH39 identity anchor kinds.
    pub fn is_identity_anchor(&self) -> bool {
        matches!(
            self.anchor_kind,
            AnchorKind::SpeakerMatch | AnchorKind::StyleHold
        )
    }
}

/// Calibrated GuardProfile plus cached matched vectors for identity slots.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IdentityProfile {
    pub guard_profile: GuardProfile,
    pub identity_slots: Vec<IdentitySlotConfig>,
    pub matched_slot_cache: MatchedSlots,
}

impl<'de> Deserialize<'de> for IdentityProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawIdentityProfile {
            guard_profile: GuardProfile,
            identity_slots: Vec<IdentitySlotConfig>,
            matched_slot_cache: MatchedSlots,
        }

        let raw = RawIdentityProfile::deserialize(deserializer)?;
        Self::new(
            raw.guard_profile,
            raw.identity_slots,
            raw.matched_slot_cache,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl IdentityProfile {
    pub fn new(
        mut guard_profile: GuardProfile,
        identity_slots: Vec<IdentitySlotConfig>,
        matched_vecs: BTreeMap<SlotId, Vec<f32>>,
    ) -> Result<Self, WardError> {
        let required: BTreeSet<_> = guard_profile.required_slots.iter().copied().collect();
        let mut seen = BTreeSet::new();
        let mut matched_slot_cache = BTreeMap::new();

        for config in &identity_slots {
            if !required.contains(&config.slot_id) {
                return Err(WardError::IdentitySlotNotRequired {
                    slot: config.slot_id,
                });
            }
            if !seen.insert(config.slot_id) {
                return Err(WardError::InvalidRequiredSlotDerivation {
                    reason: "duplicate identity slot",
                });
            }
            if !config.is_identity_anchor() {
                return Err(WardError::InvalidRequiredSlotDerivation {
                    reason: "identity slot anchor kind must be SpeakerMatch or StyleHold",
                });
            }
            if let Some(tau) = config.tau_override {
                validate_identity_tau(tau)?;
                guard_profile.tau.insert(config.slot_id, tau);
            } else if let Some(tau) = guard_profile.tau_for(&config.slot_id) {
                validate_identity_tau(tau)?;
            } else {
                return Err(WardError::InvalidCalibrationInput {
                    reason: "identity slot tau must be present in guard profile",
                });
            }
            let matched = matched_vecs
                .get(&config.slot_id)
                .ok_or(WardError::MissingSlot {
                    slot: config.slot_id,
                })?;
            matched_slot_cache.insert(config.slot_id, normalize_matched(matched)?);
        }
        for slot in &required {
            if !seen.contains(slot) {
                return Err(WardError::InvalidRequiredSlotDerivation {
                    reason: "guard profile required slots must match identity slots",
                });
            }
        }

        Ok(Self {
            guard_profile,
            identity_slots,
            matched_slot_cache,
        })
    }

    pub fn is_calibrated(&self) -> bool {
        self.guard_profile.is_calibrated()
    }

    pub fn required_identity_slots(&self) -> Vec<SlotId> {
        self.identity_slots
            .iter()
            .map(|config| config.slot_id)
            .collect()
    }
}

fn validate_identity_tau(tau: f32) -> Result<(), WardError> {
    if tau.is_finite() && (0.0..=1.0).contains(&tau) {
        Ok(())
    } else {
        Err(WardError::InvalidCalibrationInput {
            reason: "identity tau must be finite and within [0, 1]",
        })
    }
}

fn normalize_matched(values: &[f32]) -> Result<Vec<f32>, WardError> {
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return Err(WardError::InvalidCalibrationInput {
            reason: "identity matched vector must be finite and non-empty",
        });
    }
    let norm_sq: f32 = values.iter().map(|value| value * value).sum();
    if !norm_sq.is_finite() || norm_sq <= f32::EPSILON {
        return Err(WardError::InvalidCalibrationInput {
            reason: "identity matched vector must have non-zero norm",
        });
    }
    let norm = norm_sq.sqrt();
    Ok(values.iter().map(|value| value / norm).collect())
}
