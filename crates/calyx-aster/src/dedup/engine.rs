//! Dedup decision engine for PH41 T02.

use crate::cf::{ColumnFamily, base_key};
use crate::dedup::{
    AnchorConflictResult, CALYX_DEDUP_ANCHOR_CONFLICT, CALYX_DEDUP_DPI_EXCEEDED,
    CALYX_DEDUP_INVALID_TAU, CALYX_DEDUP_MISSING_GUARD_PROFILE,
    CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION, CALYX_DEDUP_SLOT_NOT_IN_TAU, ConflictReason,
    ContestedWith, DedupPolicy, TauStrategy, TctCosineConfig, check_anchor_conflict,
    contested_with_key, dedup_error, encode_contested_with,
};
use crate::vault::AsterVault;
use calyx_core::{
    Clock, Constellation, CxId, GuardTauProfile, Result, SlotId, VaultStore, dense_cosine,
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_DEDUP_DPI_CANDIDATE_LIMIT: usize = 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DedupDecision {
    NoMatch,
    Match {
        existing: CxId,
        per_slot_cos: Vec<(SlotId, f32)>,
    },
    AnchorConflict {
        existing: CxId,
    },
}

pub fn resolve_tau(
    slot_id: SlotId,
    config: &TctCosineConfig,
    guard_profile: Option<&dyn GuardTauProfile>,
) -> Result<f32> {
    let tau = match &config.tau {
        TauStrategy::PerSlot(entries) => entries
            .iter()
            .find_map(|(slot, tau)| (*slot == slot_id).then_some(*tau))
            .ok_or_else(|| {
                dedup_error(
                    CALYX_DEDUP_SLOT_NOT_IN_TAU,
                    format!("required slot {slot_id} is missing a tau threshold"),
                )
            }),
        TauStrategy::Calibrated => guard_profile
            .and_then(|profile| profile.tau_for(&slot_id))
            .ok_or_else(|| {
                dedup_error(
                    CALYX_DEDUP_MISSING_GUARD_PROFILE,
                    format!("guard profile has no tau for required slot {slot_id}"),
                )
            }),
    }?;
    validate_resolved_tau(slot_id, tau)
}

pub fn cosine_passes_all_required(
    new_cx: &Constellation,
    existing_cx: &Constellation,
    config: &TctCosineConfig,
    guard_profile: Option<&dyn GuardTauProfile>,
) -> Result<Option<Vec<(SlotId, f32)>>> {
    config.validate_static()?;
    let mut per_slot = Vec::with_capacity(config.required_slots.len());
    for slot in &config.required_slots {
        let new_dense = required_dense(new_cx, *slot)?;
        let existing_dense = required_dense(existing_cx, *slot)?;
        let tau = resolve_tau(*slot, config, guard_profile)?;
        let cosine = dense_cosine(new_dense, existing_dense).ok_or_else(|| {
            dedup_error(
                CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION,
                format!("required slot {slot} has an invalid dense vector"),
            )
        })?;
        if cosine < tau {
            return Ok(None);
        }
        per_slot.push((*slot, cosine));
    }
    Ok(Some(per_slot))
}

pub fn check_dedup<C>(
    new_cx: &Constellation,
    vault: &AsterVault<C>,
    policy: &DedupPolicy,
    guard_profile: Option<&dyn GuardTauProfile>,
) -> Result<DedupDecision>
where
    C: Clock,
{
    check_dedup_inner(
        new_cx,
        vault,
        policy,
        guard_profile,
        DEFAULT_DEDUP_DPI_CANDIDATE_LIMIT,
        true,
    )
}

pub fn check_dedup_with_limit<C>(
    new_cx: &Constellation,
    vault: &AsterVault<C>,
    policy: &DedupPolicy,
    guard_profile: Option<&dyn GuardTauProfile>,
    candidate_limit: usize,
) -> Result<DedupDecision>
where
    C: Clock,
{
    check_dedup_inner(new_cx, vault, policy, guard_profile, candidate_limit, true)
}

pub(crate) fn check_dedup_without_conflict_write<C>(
    new_cx: &Constellation,
    vault: &AsterVault<C>,
    policy: &DedupPolicy,
    guard_profile: Option<&dyn GuardTauProfile>,
) -> Result<DedupDecision>
where
    C: Clock,
{
    check_dedup_inner(
        new_cx,
        vault,
        policy,
        guard_profile,
        DEFAULT_DEDUP_DPI_CANDIDATE_LIMIT,
        false,
    )
}

fn check_dedup_inner<C>(
    new_cx: &Constellation,
    vault: &AsterVault<C>,
    policy: &DedupPolicy,
    guard_profile: Option<&dyn GuardTauProfile>,
    candidate_limit: usize,
    write_conflict_rows: bool,
) -> Result<DedupDecision>
where
    C: Clock,
{
    match policy {
        DedupPolicy::Off => Ok(DedupDecision::NoMatch),
        DedupPolicy::Exact => exact_match(new_cx, vault),
        DedupPolicy::TctCosine(config) => {
            config.validate_static()?;
            let exact = exact_match(new_cx, vault)?;
            if matches!(exact, DedupDecision::Match { .. }) {
                return Ok(exact);
            }
            let snapshot = vault.snapshot();
            let candidates = vault.scan_cf_at(snapshot, ColumnFamily::Base)?;
            if candidates.len() > candidate_limit {
                let exact = exact_match(new_cx, vault)?;
                if matches!(exact, DedupDecision::Match { .. }) {
                    return Ok(exact);
                }
                return Err(dedup_error(
                    CALYX_DEDUP_DPI_EXCEEDED,
                    format!(
                        "dedup candidate set {} exceeds DPI limit {candidate_limit}",
                        candidates.len()
                    ),
                ));
            }
            for (key, _) in candidates {
                let existing_id = cx_id_from_base_key(&key)?;
                if existing_id == new_cx.cx_id {
                    continue;
                }
                let existing = vault.get(existing_id, snapshot)?;
                if let AnchorConflictResult::Conflicting {
                    anchor_type,
                    reason,
                } = check_anchor_conflict(new_cx, &existing)
                {
                    if write_conflict_rows {
                        write_anchor_conflict(
                            vault,
                            new_cx.cx_id,
                            existing_id,
                            anchor_type,
                            reason,
                        )?;
                    }
                    return Ok(DedupDecision::AnchorConflict {
                        existing: existing_id,
                    });
                }
                if let Some(per_slot_cos) =
                    cosine_passes_all_required(new_cx, &existing, config, guard_profile)?
                {
                    return Ok(DedupDecision::Match {
                        existing: existing_id,
                        per_slot_cos,
                    });
                }
            }
            Ok(DedupDecision::NoMatch)
        }
    }
}

fn exact_match<C>(new_cx: &Constellation, vault: &AsterVault<C>) -> Result<DedupDecision>
where
    C: Clock,
{
    let snapshot = vault.snapshot();
    if vault
        .read_cf_at(snapshot, ColumnFamily::Base, &base_key(new_cx.cx_id))?
        .is_some()
    {
        let existing = vault.get(new_cx.cx_id, snapshot)?;
        reject_exact_anchor_conflict(new_cx, &existing)?;
        Ok(DedupDecision::Match {
            existing: new_cx.cx_id,
            per_slot_cos: Vec::new(),
        })
    } else {
        Ok(DedupDecision::NoMatch)
    }
}

fn reject_exact_anchor_conflict(new_cx: &Constellation, existing: &Constellation) -> Result<()> {
    if let AnchorConflictResult::Conflicting {
        anchor_type,
        reason,
    } = check_anchor_conflict(new_cx, existing)
    {
        return Err(dedup_error(
            CALYX_DEDUP_ANCHOR_CONFLICT,
            format!(
                "exact duplicate {} has conflicting {anchor_type:?} anchor: {reason:?}",
                new_cx.cx_id
            ),
        ));
    }
    Ok(())
}

fn required_dense(cx: &Constellation, slot: SlotId) -> Result<&[f32]> {
    cx.slots
        .get(&slot)
        .and_then(|vector| vector.as_dense())
        .ok_or_else(|| {
            dedup_error(
                CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION,
                format!(
                    "constellation {} is missing dense required slot {slot}",
                    cx.cx_id
                ),
            )
        })
}

fn validate_resolved_tau(slot_id: SlotId, tau: f32) -> Result<f32> {
    if tau.is_finite() && (-1.0..=1.0).contains(&tau) {
        Ok(tau)
    } else {
        Err(dedup_error(
            CALYX_DEDUP_INVALID_TAU,
            format!("tau for slot {slot_id} must be finite and in -1.0..=1.0"),
        ))
    }
}

fn cx_id_from_base_key(key: &[u8]) -> Result<CxId> {
    let bytes: [u8; 16] = key.try_into().map_err(|_| {
        calyx_core::CalyxError::aster_corrupt_shard("base CF key is not a 16-byte CxId")
    })?;
    Ok(CxId::from_bytes(bytes))
}

fn write_anchor_conflict<C>(
    vault: &AsterVault<C>,
    new_id: CxId,
    existing_id: CxId,
    anchor_type: calyx_core::AnchorKind,
    reason: ConflictReason,
) -> Result<()>
where
    C: Clock,
{
    let new_value = ContestedWith {
        contested_with: existing_id,
        anchor_type: anchor_type.clone(),
        reason: reason.clone(),
    };
    let existing_value = ContestedWith {
        contested_with: new_id,
        anchor_type,
        reason,
    };
    vault.commit_online_rows([
        (
            contested_with_key(new_id),
            encode_contested_with(&new_value)?,
        ),
        (
            contested_with_key(existing_id),
            encode_contested_with(&existing_value)?,
        ),
    ])?;
    Ok(())
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
