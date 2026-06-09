//! Incoming-query Ward guard over trusted regions.

use calyx_core::CxId;
use serde::{Deserialize, Serialize};

use crate::error::WardError;
use crate::guard::{MatchedSlots, ProducedSlots, guard};
use crate::profile::{GuardProfile, NoveltyAction};
use crate::verdict::SlotVerdict;

/// Trusted constellation region used to test whether a query is in-distribution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrustedRegion {
    pub cx_id: CxId,
    pub slots: MatchedSlots,
}

/// Query verdict with nearest-region evidence for both pass and OOD outcomes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum QueryVerdict {
    Pass {
        nearest_cx: CxId,
        gap: f32,
        per_slot: Vec<SlotVerdict>,
    },
    Ood {
        nearest_cx: Option<CxId>,
        gap: Option<f32>,
        per_slot: Vec<SlotVerdict>,
        action: NoveltyAction,
    },
}

impl QueryVerdict {
    /// Returns true only when the query is inside a trusted region.
    pub const fn is_pass(&self) -> bool {
        matches!(self, Self::Pass { .. })
    }
}

/// Gates incoming query slots against trusted regions slot by slot.
pub fn guard_query(
    profile: &GuardProfile,
    query_slots: &ProducedSlots,
    trusted_regions: &[TrustedRegion],
) -> Result<QueryVerdict, WardError> {
    let mut best_pass = None;
    let mut best_ood = None;

    for region in trusted_regions {
        let verdict = guard(profile, query_slots, &region.slots)?;
        let margin = nearest_margin(&verdict.per_slot);
        let candidate = Candidate {
            cx_id: region.cx_id,
            margin,
            per_slot: verdict.per_slot,
        };
        if verdict.overall_pass {
            keep_best(&mut best_pass, candidate);
        } else {
            keep_best(&mut best_ood, candidate);
        }
    }

    if let Some(candidate) = best_pass {
        Ok(QueryVerdict::Pass {
            nearest_cx: candidate.cx_id,
            gap: 0.0,
            per_slot: candidate.per_slot,
        })
    } else if let Some(candidate) = best_ood {
        Ok(QueryVerdict::Ood {
            nearest_cx: Some(candidate.cx_id),
            gap: Some((-candidate.margin).max(0.0)),
            per_slot: candidate.per_slot,
            action: profile.novelty_action.clone(),
        })
    } else {
        Ok(QueryVerdict::Ood {
            nearest_cx: None,
            gap: None,
            per_slot: Vec::new(),
            action: profile.novelty_action.clone(),
        })
    }
}

#[derive(Clone, Debug)]
struct Candidate {
    cx_id: CxId,
    margin: f32,
    per_slot: Vec<SlotVerdict>,
}

fn keep_best(best: &mut Option<Candidate>, candidate: Candidate) {
    if best
        .as_ref()
        .is_none_or(|existing| candidate.margin > existing.margin)
    {
        *best = Some(candidate);
    }
}

fn nearest_margin(per_slot: &[SlotVerdict]) -> f32 {
    per_slot
        .iter()
        .map(|slot| slot.cos - slot.tau)
        .reduce(f32::min)
        .unwrap_or(0.0)
}
