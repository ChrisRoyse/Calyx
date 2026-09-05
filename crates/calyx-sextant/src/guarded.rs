//! Ward-backed guarded search filtering.

use std::collections::BTreeMap;

use calyx_core::{CalyxError, Constellation, CxId, Result, SlotId, SlotVector};
use calyx_ward::{
    GuardProfile, GuardVerdict, SlotVerdict, TypedGuardVerdict, TypedMatchedSlots,
    TypedProducedSlots, WardError, guard_typed, validate_non_inert_profile,
};
use serde::{Deserialize, Serialize};

use crate::hit::{DroppedGuardHit, Hit, HitGuardEvidence, HitGuardMode};
use crate::query::{Query, QueryGuard};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuardedSearchReport {
    pub hits: Vec<Hit>,
    pub dropped_guard_hits: Vec<DroppedGuardHit>,
}

pub fn apply_in_region_guard_to_hits(
    docs: &BTreeMap<CxId, Constellation>,
    profile: &GuardProfile,
    query_vectors: &[(SlotId, SlotVector)],
    hits: &mut Vec<Hit>,
    high_stakes: bool,
) -> Result<Vec<DroppedGuardHit>> {
    validate_trusted_guard_profile(profile)?;
    let produced = produced_guard_slots_from_vectors(query_vectors, profile)?;
    if high_stakes {
        guard_typed(profile, &produced, &produced, true).map_err(ward_error)?;
    }

    let mut kept = Vec::with_capacity(hits.len());
    let mut dropped = Vec::new();
    for mut hit in hits.drain(..) {
        match guard_candidate_with_stakes(docs, profile, &produced, &hit, high_stakes) {
            CandidateGuard::Pass(typed_verdict) => {
                hit.guard = Some(HitGuardEvidence {
                    mode: HitGuardMode::InRegionOnly,
                    verdict: dense_summary(&typed_verdict),
                    typed_verdict: Some(typed_verdict),
                });
                kept.push(hit);
            }
            CandidateGuard::Drop {
                cx_id,
                reason,
                typed_verdict,
            } => {
                dropped.push(DroppedGuardHit {
                    cx_id,
                    mode: HitGuardMode::InRegionOnly,
                    reason,
                    verdict: typed_verdict.as_ref().map(dense_summary),
                    typed_verdict,
                });
            }
        }
    }
    *hits = kept;
    Ok(dropped)
}

pub(crate) fn apply_query_guard(
    docs: &BTreeMap<CxId, Constellation>,
    query: &Query,
    hits: &mut Vec<Hit>,
) -> Result<Vec<DroppedGuardHit>> {
    let Some(QueryGuard::InRegionOnly(profile)) = &query.guard else {
        return Ok(Vec::new());
    };
    validate_trusted_guard_profile(profile)?;
    let produced = produced_guard_slots(query, profile)?;
    let mut kept = Vec::with_capacity(hits.len());
    let mut dropped = Vec::new();
    for mut hit in hits.drain(..) {
        match guard_candidate(docs, profile, &produced, &hit) {
            CandidateGuard::Pass(typed_verdict) => {
                hit.guard = Some(HitGuardEvidence {
                    mode: HitGuardMode::InRegionOnly,
                    verdict: dense_summary(&typed_verdict),
                    typed_verdict: Some(typed_verdict),
                });
                kept.push(hit);
            }
            CandidateGuard::Drop {
                cx_id,
                reason,
                typed_verdict,
            } => {
                dropped.push(DroppedGuardHit {
                    cx_id,
                    mode: HitGuardMode::InRegionOnly,
                    reason,
                    verdict: typed_verdict.as_ref().map(dense_summary),
                    typed_verdict,
                });
            }
        }
    }
    if query.explain {
        for hit in &mut kept {
            if let Some(explain) = &mut hit.explain {
                explain.guard_dropped = dropped.clone();
            }
        }
    }
    *hits = kept;
    Ok(dropped)
}

fn validate_trusted_guard_profile(profile: &GuardProfile) -> Result<()> {
    validate_non_inert_profile(profile).map_err(ward_error)
}

enum CandidateGuard {
    Pass(TypedGuardVerdict),
    Drop {
        cx_id: CxId,
        reason: String,
        typed_verdict: Option<TypedGuardVerdict>,
    },
}

fn guard_candidate(
    docs: &BTreeMap<CxId, Constellation>,
    profile: &GuardProfile,
    produced: &TypedProducedSlots,
    hit: &Hit,
) -> CandidateGuard {
    guard_candidate_with_stakes(docs, profile, produced, hit, false)
}

fn guard_candidate_with_stakes(
    docs: &BTreeMap<CxId, Constellation>,
    profile: &GuardProfile,
    produced: &TypedProducedSlots,
    hit: &Hit,
    high_stakes: bool,
) -> CandidateGuard {
    let Some(cx) = docs.get(&hit.cx_id) else {
        return drop_without_verdict(hit.cx_id, "missing_constellation");
    };
    let matched = match matched_guard_slots(cx, profile) {
        Ok(matched) => matched,
        Err(reason) => return drop_without_verdict(hit.cx_id, reason),
    };
    let result = guard_typed(profile, produced, &matched, high_stakes);
    match result {
        Ok(verdict) if verdict.overall_pass => CandidateGuard::Pass(verdict),
        Ok(verdict) => CandidateGuard::Drop {
            cx_id: hit.cx_id,
            reason: "ood".to_string(),
            typed_verdict: Some(verdict),
        },
        Err(error) => drop_without_verdict(hit.cx_id, ward_reason(&error)),
    }
}

fn produced_guard_slots(query: &Query, profile: &GuardProfile) -> Result<TypedProducedSlots> {
    let slots = required_slots(profile);
    let mut produced = TypedProducedSlots::new();

    if !query.guard_vectors.is_empty() {
        for slot in slots {
            let vector = query.guard_vectors.get(&slot).ok_or_else(|| {
                crate::error::sextant_error(
                    crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                    format!("InRegionOnly guard missing slot-aware query vector:{slot}"),
                )
            })?;
            vector.validate_schema()?;
            produced.insert(slot, vector.clone());
        }
        return Ok(produced);
    }

    if slots.len() > 1 {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
            "InRegionOnly guard requires slot-aware query guard vectors for multi-slot profiles",
        ));
    }

    let vector = query.vector.as_ref().ok_or_else(|| {
        crate::error::sextant_error(
            crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
            "InRegionOnly guard requires a typed query vector",
        )
    })?;
    vector.validate_schema()?;
    for slot in slots {
        produced.insert(slot, vector.clone());
    }
    Ok(produced)
}

fn produced_guard_slots_from_vectors(
    query_vectors: &[(SlotId, SlotVector)],
    profile: &GuardProfile,
) -> Result<TypedProducedSlots> {
    let mut produced = TypedProducedSlots::new();
    for slot in required_slots(profile) {
        let vector = query_vectors
            .iter()
            .find(|(candidate, _)| *candidate == slot)
            .map(|(_, vector)| vector)
            .ok_or_else(|| {
                crate::error::sextant_error(
                    crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                    format!("InRegionOnly guard missing slot-aware query vector:{slot}"),
                )
            })?;
        vector.validate_schema()?;
        produced.insert(slot, vector.clone());
    }
    Ok(produced)
}

fn matched_guard_slots(
    cx: &Constellation,
    profile: &GuardProfile,
) -> std::result::Result<TypedMatchedSlots, String> {
    let mut matched = TypedMatchedSlots::new();
    for slot in required_slots(profile) {
        let vector = cx
            .slots
            .get(&slot)
            .ok_or_else(|| format!("missing_hit_slot:{slot}"))?;
        vector
            .validate_schema()
            .map_err(|error| format!("invalid_hit_slot:{slot}:{}", error.code))?;
        matched.insert(slot, vector.clone());
    }
    Ok(matched)
}

fn required_slots(profile: &GuardProfile) -> Vec<SlotId> {
    let mut slots = profile.required_slots.clone();
    slots.sort_unstable();
    slots.dedup();
    slots
}

fn drop_without_verdict(cx_id: CxId, reason: impl Into<String>) -> CandidateGuard {
    CandidateGuard::Drop {
        cx_id,
        reason: reason.into(),
        typed_verdict: None,
    }
}

fn dense_summary(typed: &TypedGuardVerdict) -> GuardVerdict {
    GuardVerdict {
        guard_id: typed.guard_id,
        overall_pass: typed.overall_pass,
        provisional: typed.provisional,
        per_slot: typed
            .per_slot
            .iter()
            .map(|slot| SlotVerdict {
                slot: slot.slot,
                cos: slot.cos,
                tau: slot.tau,
                pass: slot.pass,
            })
            .collect(),
        action: typed.action.clone(),
    }
}

fn ward_reason(error: &WardError) -> String {
    format!("ward_error:{}", error.code())
}

fn ward_error(error: WardError) -> CalyxError {
    CalyxError {
        code: error.code(),
        message: error.to_string(),
        remediation: "configure at least one required guard slot and a non-zero pass policy",
    }
}
