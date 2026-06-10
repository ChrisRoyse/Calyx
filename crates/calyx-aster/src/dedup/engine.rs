//! Dedup decision engine for PH41 T02.

use crate::cf::{ColumnFamily, base_key};
use crate::dedup::{
    CALYX_DEDUP_DPI_EXCEEDED, CALYX_DEDUP_MISSING_GUARD_PROFILE,
    CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION, CALYX_DEDUP_SLOT_NOT_IN_TAU, DedupPolicy, TauStrategy,
    TctCosineConfig, dedup_error,
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
    match &config.tau {
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
    }
}

pub fn cosine_passes_all_required(
    new_cx: &Constellation,
    existing_cx: &Constellation,
    config: &TctCosineConfig,
    guard_profile: Option<&dyn GuardTauProfile>,
) -> Result<Option<Vec<(SlotId, f32)>>> {
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
    check_dedup_with_limit(
        new_cx,
        vault,
        policy,
        guard_profile,
        DEFAULT_DEDUP_DPI_CANDIDATE_LIMIT,
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
    match policy {
        DedupPolicy::Off => Ok(DedupDecision::NoMatch),
        DedupPolicy::Exact => exact_match(new_cx, vault),
        DedupPolicy::TctCosine(config) => {
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
                let existing = vault.get(existing_id, snapshot)?;
                if anchor_conflict_placeholder(new_cx, &existing) {
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
        Ok(DedupDecision::Match {
            existing: new_cx.cx_id,
            per_slot_cos: Vec::new(),
        })
    } else {
        Ok(DedupDecision::NoMatch)
    }
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

fn cx_id_from_base_key(key: &[u8]) -> Result<CxId> {
    let bytes: [u8; 16] = key.try_into().map_err(|_| {
        calyx_core::CalyxError::aster_corrupt_shard("base CF key is not a 16-byte CxId")
    })?;
    Ok(CxId::from_bytes(bytes))
}

fn anchor_conflict_placeholder(_new_cx: &Constellation, _existing_cx: &Constellation) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use calyx_core::{
        CxFlags, FixedClock, InputRef, LedgerRef, Modality, SlotVector, VaultId, VaultStore,
    };
    use proptest::prelude::*;

    use super::*;
    use crate::dedup::{CALYX_DEDUP_DPI_EXCEEDED, DedupAction};

    #[test]
    fn off_policy_returns_no_match_even_when_exact_exists() {
        let vault = sample_vault();
        let cx = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
        vault.put(cx.clone()).expect("put existing");

        let decision = check_dedup(&cx, &vault, &DedupPolicy::Off, None).expect("dedup");

        assert_eq!(decision, DedupDecision::NoMatch);
    }

    #[test]
    fn identical_vectors_match_with_per_slot_cosine() {
        let vault = sample_vault();
        let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
        let new = sample_cx(2, [(slot(0), dense(vec![1.0, 0.0]))]);
        vault.put(existing.clone()).expect("put existing");

        let decision = check_dedup(&new, &vault, &policy([(slot(0), 0.9)], vec![slot(0)]), None)
            .expect("dedup");

        assert_eq!(
            decision,
            DedupDecision::Match {
                existing: existing.cx_id,
                per_slot_cos: vec![(slot(0), 1.0)]
            }
        );
    }

    #[test]
    fn below_tau_returns_no_match() {
        let vault = sample_vault();
        let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
        let new = sample_cx(2, [(slot(0), dense(cos_vector(0.88)))]);
        vault.put(existing).expect("put existing");

        let decision = check_dedup(&new, &vault, &policy([(slot(0), 0.9)], vec![slot(0)]), None)
            .expect("dedup");

        assert_eq!(decision, DedupDecision::NoMatch);
    }

    #[test]
    fn all_required_slots_must_pass_independently() {
        let vault = sample_vault();
        let existing = sample_cx(
            1,
            [
                (slot(0), dense(vec![1.0, 0.0])),
                (slot(1), dense(vec![1.0, 0.0])),
            ],
        );
        let new = sample_cx(
            2,
            [
                (slot(0), dense(cos_vector(0.95))),
                (slot(1), dense(cos_vector(0.80))),
            ],
        );
        vault.put(existing).expect("put existing");

        let decision = check_dedup(
            &new,
            &vault,
            &policy([(slot(0), 0.9), (slot(1), 0.9)], vec![slot(0), slot(1)]),
            None,
        )
        .expect("dedup");

        assert_eq!(decision, DedupDecision::NoMatch);
    }

    #[test]
    fn calibrated_missing_profile_fails_closed() {
        let config = TctCosineConfig::new(
            vec![slot(0)],
            TauStrategy::Calibrated,
            DedupAction::Collapse,
        )
        .expect("config");

        let error = resolve_tau(slot(0), &config, None).expect_err("missing profile");

        assert_eq!(error.code, CALYX_DEDUP_MISSING_GUARD_PROFILE);
    }

    #[test]
    fn calibrated_profile_tau_matches_without_aster_ward_dependency() {
        let vault = sample_vault();
        let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
        let new = sample_cx(2, [(slot(0), dense(cos_vector(0.95)))]);
        vault.put(existing.clone()).expect("put existing");
        let mut profile = BTreeMap::new();
        profile.insert(slot(0), 0.9);
        let policy = DedupPolicy::TctCosine(
            TctCosineConfig::new(
                vec![slot(0)],
                TauStrategy::Calibrated,
                DedupAction::Collapse,
            )
            .expect("policy"),
        );

        let decision = check_dedup(&new, &vault, &policy, Some(&profile)).expect("dedup");

        assert_eq!(decision_existing(&decision), Some(existing.cx_id));
        assert!(slot_cosine_close(&decision, slot(0), 0.95));
    }

    #[test]
    fn missing_required_slot_fails_closed() {
        let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
        let new = sample_cx(2, [(slot(1), dense(vec![1.0, 0.0]))]);
        let error = cosine_passes_all_required(
            &new,
            &existing,
            &policy([(slot(0), 0.9)], vec![slot(0)]).tct_config(),
            None,
        )
        .expect_err("missing slot");

        assert_eq!(error.code, CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION);
    }

    #[test]
    fn empty_vault_returns_no_match() {
        let vault = sample_vault();
        let new = sample_cx(2, [(slot(0), dense(vec![1.0, 0.0]))]);

        let decision = check_dedup(&new, &vault, &policy([(slot(0), 0.9)], vec![slot(0)]), None)
            .expect("dedup");

        assert_eq!(decision, DedupDecision::NoMatch);
    }

    #[test]
    fn candidate_set_over_dpi_fails_closed_when_exact_not_found() {
        let vault = sample_vault();
        let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
        let new = sample_cx(2, [(slot(0), dense(vec![1.0, 0.0]))]);
        vault.put(existing).expect("put existing");

        let error = check_dedup_with_limit(
            &new,
            &vault,
            &policy([(slot(0), 0.9)], vec![slot(0)]),
            None,
            0,
        )
        .expect_err("dpi exceeded");

        assert_eq!(error.code, CALYX_DEDUP_DPI_EXCEEDED);
    }

    proptest! {
        #[test]
        fn identical_constellations_always_match(seed in 1u8..=u8::MAX) {
            let vault = sample_vault();
            let slots = [(slot(0), dense(vec![0.25, 0.50, 0.75]))];
            let existing = sample_cx(seed, slots.clone());
            let new = sample_cx(seed.wrapping_add(1), slots);
            vault.put(existing.clone()).expect("put existing");

            let decision = check_dedup(
                &new,
                &vault,
                &policy([(slot(0), 0.9)], vec![slot(0)]),
                None,
            )
            .expect("dedup");

            prop_assert_eq!(
                decision_existing(&decision),
                Some(existing.cx_id)
            );
            prop_assert!(slot_cosine_close(&decision, slot(0), 1.0));
        }
    }

    trait PolicyConfig {
        fn tct_config(&self) -> TctCosineConfig;
    }

    impl PolicyConfig for DedupPolicy {
        fn tct_config(&self) -> TctCosineConfig {
            match self {
                DedupPolicy::TctCosine(config) => config.clone(),
                DedupPolicy::Off | DedupPolicy::Exact => unreachable!("test policy is tct"),
            }
        }
    }

    fn decision_existing(decision: &DedupDecision) -> Option<CxId> {
        match decision {
            DedupDecision::Match { existing, .. } => Some(*existing),
            DedupDecision::NoMatch | DedupDecision::AnchorConflict { .. } => None,
        }
    }

    fn slot_cosine_close(decision: &DedupDecision, slot: SlotId, expected: f32) -> bool {
        match decision {
            DedupDecision::Match { per_slot_cos, .. } => per_slot_cos
                .iter()
                .any(|(actual_slot, actual)| *actual_slot == slot && close(*actual, expected)),
            DedupDecision::NoMatch | DedupDecision::AnchorConflict { .. } => false,
        }
    }

    fn close(actual: f32, expected: f32) -> bool {
        (actual - expected).abs() <= 1.0e-5
    }

    fn sample_vault() -> AsterVault<FixedClock> {
        AsterVault::with_clock(
            vault_id(),
            b"dedup-engine-test-salt".to_vec(),
            FixedClock::new(1),
        )
    }

    fn sample_cx<const N: usize>(seed: u8, slots: [(SlotId, SlotVector); N]) -> Constellation {
        Constellation {
            cx_id: CxId::from_bytes([seed; 16]),
            vault_id: vault_id(),
            panel_version: 1,
            created_at: u64::from(seed),
            input_ref: InputRef {
                hash: [seed; 32],
                pointer: Some(format!("zfs://calyx/dedup-engine/{seed}")),
                redacted: false,
            },
            modality: Modality::Text,
            slots: slots.into_iter().collect(),
            scalars: BTreeMap::new(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: u64::from(seed),
                hash: [seed; 32],
            },
            flags: CxFlags::default(),
        }
    }

    fn policy<const N: usize>(tau: [(SlotId, f32); N], required: Vec<SlotId>) -> DedupPolicy {
        DedupPolicy::TctCosine(
            TctCosineConfig::new(
                required,
                TauStrategy::PerSlot(tau.into_iter().collect()),
                DedupAction::Collapse,
            )
            .expect("policy"),
        )
    }

    fn dense(data: Vec<f32>) -> SlotVector {
        SlotVector::Dense {
            dim: data.len() as u32,
            data,
        }
    }

    fn cos_vector(cos: f32) -> Vec<f32> {
        vec![cos, (1.0 - cos * cos).sqrt()]
    }

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
    }

    const fn slot(value: u16) -> SlotId {
        SlotId::new(value)
    }
}
