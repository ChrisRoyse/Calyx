//! Ward-backed guarded search filtering.

use std::collections::BTreeMap;

use calyx_core::{CalyxError, Constellation, CxId, Result, SlotId, SlotVector};
use calyx_ward::{
    GuardProfile, GuardVerdict, MatchedSlots, ProducedSlots, WardError, guard_non_high_stakes,
    validate_non_inert_profile,
};
use serde::{Deserialize, Serialize};

use crate::hit::{DroppedGuardHit, Hit, HitGuardEvidence, HitGuardMode};
use crate::query::{Query, QueryGuard};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuardedSearchReport {
    pub hits: Vec<Hit>,
    pub dropped_guard_hits: Vec<DroppedGuardHit>,
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
            CandidateGuard::Pass(verdict) => {
                hit.guard = Some(HitGuardEvidence {
                    mode: HitGuardMode::InRegionOnly,
                    verdict,
                });
                kept.push(hit);
            }
            CandidateGuard::Drop {
                cx_id,
                reason,
                verdict,
            } => {
                dropped.push(DroppedGuardHit {
                    cx_id,
                    mode: HitGuardMode::InRegionOnly,
                    reason,
                    verdict,
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
    Pass(GuardVerdict),
    Drop {
        cx_id: CxId,
        reason: String,
        verdict: Option<GuardVerdict>,
    },
}

fn guard_candidate(
    docs: &BTreeMap<CxId, Constellation>,
    profile: &GuardProfile,
    produced: &ProducedSlots,
    hit: &Hit,
) -> CandidateGuard {
    let Some(cx) = docs.get(&hit.cx_id) else {
        return drop_without_verdict(hit.cx_id, "missing_constellation");
    };
    let matched = match matched_guard_slots(cx, profile) {
        Ok(matched) => matched,
        Err(reason) => return drop_without_verdict(hit.cx_id, reason),
    };
    match guard_non_high_stakes(profile, produced, &matched) {
        Ok(verdict) if verdict.overall_pass => CandidateGuard::Pass(verdict),
        Ok(verdict) => CandidateGuard::Drop {
            cx_id: hit.cx_id,
            reason: "ood".to_string(),
            verdict: Some(verdict),
        },
        Err(error) => drop_without_verdict(hit.cx_id, ward_reason(&error)),
    }
}

fn produced_guard_slots(query: &Query, profile: &GuardProfile) -> Result<ProducedSlots> {
    let slots = required_slots(profile);
    let mut produced = ProducedSlots::new();

    if !query.guard_vectors.is_empty() {
        for slot in slots {
            let vector = query.guard_vectors.get(&slot).ok_or_else(|| {
                crate::error::sextant_error(
                    crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                    format!("InRegionOnly guard missing slot-aware query vector:{slot}"),
                )
            })?;
            let data = dense_data(Some(vector), "query").map_err(|reason| {
                crate::error::sextant_error(
                    crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                    format!(
                        "InRegionOnly guard requires dense query vector for slot {slot}: {reason}"
                    ),
                )
            })?;
            produced.insert(slot, data.to_vec());
        }
        return Ok(produced);
    }

    if slots.len() > 1 {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
            "InRegionOnly guard requires slot-aware query guard vectors for multi-slot profiles",
        ));
    }

    let data = dense_data(query.vector.as_ref(), "query").map_err(|reason| {
        crate::error::sextant_error(
            crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
            format!("InRegionOnly guard requires dense query vector: {reason}"),
        )
    })?;
    for slot in slots {
        produced.insert(slot, data.to_vec());
    }
    Ok(produced)
}

fn matched_guard_slots(
    cx: &Constellation,
    profile: &GuardProfile,
) -> std::result::Result<MatchedSlots, String> {
    let mut matched = MatchedSlots::new();
    for slot in required_slots(profile) {
        let vector = cx
            .slots
            .get(&slot)
            .ok_or_else(|| format!("missing_hit_slot:{slot}"))?;
        matched.insert(slot, dense_data(Some(vector), "hit")?.to_vec());
    }
    Ok(matched)
}

fn dense_data<'a>(
    vector: Option<&'a SlotVector>,
    owner: &str,
) -> std::result::Result<&'a [f32], String> {
    match vector.and_then(SlotVector::as_dense) {
        Some(data) => Ok(data),
        None => Err(format!("non_dense_{owner}_slot")),
    }
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
        verdict: None,
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
