//! Incoming-query Ward guard over trusted regions.

use calyx_core::CxId;
use serde::{Deserialize, Serialize};

use crate::error::WardError;
use crate::guard::{
    MatchedSlots, ProducedSlots, guard_non_high_stakes, validate_non_inert_profile,
};
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

/// Source of the trusted region selected by a kernel-first guard query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionSource {
    KernelNear,
    Peripheral,
}

/// Query verdict that records whether the selected match came from the kernel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum KernelFirstQueryVerdict {
    Pass {
        nearest_cx: CxId,
        match_source: RegionSource,
        gap: f32,
        per_slot: Vec<SlotVerdict>,
    },
    Ood {
        nearest_cx: Option<CxId>,
        match_source: Option<RegionSource>,
        gap: Option<f32>,
        per_slot: Vec<SlotVerdict>,
        action: NoveltyAction,
    },
}

impl KernelFirstQueryVerdict {
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
    match guard_query_kernel_first(profile, query_slots, &[], trusted_regions)? {
        KernelFirstQueryVerdict::Pass {
            nearest_cx,
            gap,
            per_slot,
            ..
        } => Ok(QueryVerdict::Pass {
            nearest_cx,
            gap,
            per_slot,
        }),
        KernelFirstQueryVerdict::Ood {
            nearest_cx,
            gap,
            per_slot,
            action,
            ..
        } => Ok(QueryVerdict::Ood {
            nearest_cx,
            gap,
            per_slot,
            action,
        }),
    }
}

/// Gates incoming queries against kernel-near regions before peripheral regions.
pub fn guard_query_kernel_first(
    profile: &GuardProfile,
    query_slots: &ProducedSlots,
    kernel_regions: &[TrustedRegion],
    peripheral_regions: &[TrustedRegion],
) -> Result<KernelFirstQueryVerdict, WardError> {
    validate_non_inert_profile(profile)?;
    let kernel = evaluate_regions(
        profile,
        query_slots,
        kernel_regions,
        RegionSource::KernelNear,
    )?;
    if let Some(candidate) = kernel.best_pass {
        return Ok(pass_verdict(candidate));
    }

    let peripheral = evaluate_regions(
        profile,
        query_slots,
        peripheral_regions,
        RegionSource::Peripheral,
    )?;
    if let Some(candidate) = peripheral.best_pass {
        return Ok(pass_verdict(candidate));
    }

    if let Some(candidate) = best_candidate(kernel.best_ood, peripheral.best_ood) {
        Ok(KernelFirstQueryVerdict::Ood {
            nearest_cx: Some(candidate.cx_id),
            match_source: Some(candidate.source),
            gap: Some((-candidate.margin).max(0.0)),
            per_slot: candidate.per_slot,
            action: profile.novelty_action.clone(),
        })
    } else {
        Ok(KernelFirstQueryVerdict::Ood {
            nearest_cx: None,
            match_source: None,
            gap: None,
            per_slot: Vec::new(),
            action: profile.novelty_action.clone(),
        })
    }
}

fn evaluate_regions(
    profile: &GuardProfile,
    query_slots: &ProducedSlots,
    trusted_regions: &[TrustedRegion],
    source: RegionSource,
) -> Result<Candidates, WardError> {
    let mut best_pass = None;
    let mut best_ood = None;

    for region in trusted_regions {
        let verdict = guard_non_high_stakes(profile, query_slots, &region.slots)?;
        let margin = nearest_margin(&verdict.per_slot);
        let candidate = Candidate {
            cx_id: region.cx_id,
            source,
            margin,
            per_slot: verdict.per_slot,
        };
        if verdict.overall_pass {
            keep_best(&mut best_pass, candidate);
        } else {
            keep_best(&mut best_ood, candidate);
        }
    }

    Ok(Candidates {
        best_pass,
        best_ood,
    })
}

fn pass_verdict(candidate: Candidate) -> KernelFirstQueryVerdict {
    KernelFirstQueryVerdict::Pass {
        nearest_cx: candidate.cx_id,
        match_source: candidate.source,
        gap: 0.0,
        per_slot: candidate.per_slot,
    }
}

#[derive(Clone, Debug)]
struct Candidate {
    cx_id: CxId,
    source: RegionSource,
    margin: f32,
    per_slot: Vec<SlotVerdict>,
}

#[derive(Clone, Debug, Default)]
struct Candidates {
    best_pass: Option<Candidate>,
    best_ood: Option<Candidate>,
}

fn keep_best(best: &mut Option<Candidate>, candidate: Candidate) {
    if best
        .as_ref()
        .is_none_or(|existing| candidate.margin > existing.margin)
    {
        *best = Some(candidate);
    }
}

fn best_candidate(left: Option<Candidate>, right: Option<Candidate>) -> Option<Candidate> {
    match (left, right) {
        (Some(left), Some(right)) => {
            if left.margin >= right.margin {
                Some(left)
            } else {
                Some(right)
            }
        }
        (Some(candidate), None) | (None, Some(candidate)) => Some(candidate),
        (None, None) => None,
    }
}

fn nearest_margin(per_slot: &[SlotVerdict]) -> f32 {
    per_slot
        .iter()
        .map(|slot| slot.cos - slot.tau)
        .reduce(f32::min)
        .unwrap_or(0.0)
}
