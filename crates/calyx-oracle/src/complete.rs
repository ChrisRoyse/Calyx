//! Unified Oracle completion primitive over partial constellations.

use std::collections::BTreeMap;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    CalyxError, Clock, Constellation, LedgerRef, LensId, Panel, SlotVector, content_address,
};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use calyx_ward::TrustedRegion;
use serde::Serialize;

use crate::{
    AnnealConfig, CompletionResult, CompletionSlotPartition, DomainId, MAX_STEPS, OracleError,
    OracleSelfConsistency, SlotSet, SlotTag, SufficiencyAssay, TaggedSlot, VaultSufficiencyAssay,
    check_sufficiency_with_assay, descend, get_beta,
};

pub const COMPLETION_LEDGER_TAG: &str = "oracle_completion_v1";

const LEDGER_ACTOR: &str = "calyx-oracle";

/// Supplies trusted-region attractors for each free lens.
pub trait CompletionRegion {
    fn members_for_lens(
        &self,
        domain: &DomainId,
        cx: &Constellation,
        lens_id: LensId,
    ) -> Result<Vec<Vec<f32>>, OracleError>;
}

/// Writes completion provenance to the append-only ledger.
pub trait CompletionLedger {
    fn append_completion(&self, payload: CompletionLedgerPayload)
    -> Result<LedgerRef, OracleError>;
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CompletionLedgerPayload {
    pub tag: &'static str,
    pub domain_id: String,
    pub cx_id: String,
    pub clamp: Vec<String>,
    pub free: Vec<String>,
    pub confidence: f32,
    pub energy: f32,
    pub converged: bool,
    pub ceiling: f32,
    pub ts: u64,
}

impl CompletionLedgerPayload {
    fn new(
        domain: &DomainId,
        cx: &Constellation,
        clamp: &SlotSet,
        free: &SlotSet,
        result: &CompletionDraft,
        ceiling: f32,
        clock: &dyn Clock,
    ) -> Self {
        Self {
            tag: COMPLETION_LEDGER_TAG,
            domain_id: domain.to_string(),
            cx_id: cx.cx_id.to_string(),
            clamp: sorted_lens_strings(clamp),
            free: sorted_lens_strings(free),
            confidence: result.confidence,
            energy: result.energy,
            converged: result.converged,
            ceiling,
            ts: clock.now(),
        }
    }
}

/// Completion region adapter backed by Ward trusted regions.
pub struct WardCompletionRegion<'a> {
    panel: &'a Panel,
    regions: &'a [TrustedRegion],
}

impl<'a> WardCompletionRegion<'a> {
    pub const fn new(panel: &'a Panel, regions: &'a [TrustedRegion]) -> Self {
        Self { panel, regions }
    }
}

impl CompletionRegion for WardCompletionRegion<'_> {
    fn members_for_lens(
        &self,
        _domain: &DomainId,
        _cx: &Constellation,
        lens_id: LensId,
    ) -> Result<Vec<Vec<f32>>, OracleError> {
        let Some(slot_id) = slot_id_for_lens(self.panel, lens_id) else {
            return Ok(Vec::new());
        };
        Ok(self
            .regions
            .iter()
            .filter_map(|region| region.slots.get(&slot_id).cloned())
            .collect())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn complete<C, R>(
    vault: &AsterVault<C>,
    cx: &Constellation,
    panel: &Panel,
    domain: DomainId,
    clamp: SlotSet,
    free: SlotSet,
    region: &R,
    self_consistency: OracleSelfConsistency,
    anneal: &dyn AnnealConfig,
    clock: &dyn Clock,
) -> Result<CompletionResult, OracleError>
where
    C: Clock,
    R: CompletionRegion,
{
    let assay = VaultSufficiencyAssay::new(vault);
    let ledger = AsterCompletionLedger { vault };
    complete_with_assay_and_region(
        &assay,
        &ledger,
        cx,
        panel,
        domain,
        clamp,
        free,
        region,
        self_consistency,
        anneal,
        clock,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn complete_with_assay_and_region<A, L, R>(
    assay: &A,
    ledger: &L,
    cx: &Constellation,
    panel: &Panel,
    domain: DomainId,
    clamp: SlotSet,
    free: SlotSet,
    region: &R,
    self_consistency: OracleSelfConsistency,
    anneal: &dyn AnnealConfig,
    clock: &dyn Clock,
) -> Result<CompletionResult, OracleError>
where
    A: SufficiencyAssay,
    L: CompletionLedger,
    R: CompletionRegion,
{
    let all_slots = validate_request(cx, panel, &clamp, &free)?;
    let sufficiency = check_sufficiency_with_assay(assay, panel, domain.clone(), clock)?;
    let slot_vectors = dense_vectors_by_lens(cx, panel)?;
    let mut filled = measured_slots(&clamp, &slot_vectors)?;
    let mut descents = Vec::new();

    for lens_id in sorted_lenses(&free) {
        let members = region.members_for_lens(&domain, cx, lens_id)?;
        let member_refs = members.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let mut vector = slot_vectors
            .get(&lens_id)
            .cloned()
            .unwrap_or_else(|| mean_vector(&members));
        let beta = get_beta(domain.clone(), anneal);
        let descent = descend(
            &mut vector,
            &member_refs,
            beta,
            MAX_STEPS,
            crate::DEFAULT_EPS,
        )?;
        let tag = if descent.converged && sufficiency.sufficient {
            SlotTag::Inferred
        } else {
            SlotTag::Provisional
        };
        filled.push(TaggedSlot {
            lens_id,
            vector,
            tag,
        });
        descents.push(SlotDescent {
            final_energy: descent.final_energy,
            converged: descent.converged,
            member_count: members.len(),
        });
    }

    let draft = CompletionDraft::from_descents(&descents, self_consistency.ceiling);
    let payload = CompletionLedgerPayload::new(
        &domain,
        cx,
        &clamp,
        &free,
        &draft,
        self_consistency.ceiling,
        clock,
    );
    let provenance = ledger.append_completion(payload)?;
    CompletionResult::new(
        sort_tagged_slots(filled),
        draft.confidence,
        draft.converged,
        draft.energy,
        provenance,
        CompletionSlotPartition::new(&all_slots, &clamp, &free),
    )
}

struct AsterCompletionLedger<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<C> CompletionLedger for AsterCompletionLedger<'_, C>
where
    C: Clock,
{
    fn append_completion(
        &self,
        payload: CompletionLedgerPayload,
    ) -> Result<LedgerRef, OracleError> {
        let subject = SubjectId::Query(completion_subject(&payload).to_vec());
        let bytes = serde_json::to_vec(&payload).map_err(|_| OracleError::LedgerWriteFailure)?;
        self.vault
            .append_ledger_entry(
                EntryKind::Answer,
                subject,
                bytes,
                ActorId::Service(LEDGER_ACTOR.to_string()),
            )
            .map_err(|_| OracleError::LedgerWriteFailure)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SlotDescent {
    final_energy: f32,
    converged: bool,
    member_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CompletionDraft {
    confidence: f32,
    converged: bool,
    energy: f32,
}

impl CompletionDraft {
    fn from_descents(descents: &[SlotDescent], ceiling: f32) -> Self {
        if descents.is_empty() {
            return Self {
                confidence: 1.0_f32.min(valid_ceiling(ceiling)),
                converged: true,
                energy: 0.0,
            };
        }
        let mean_energy =
            descents.iter().map(|item| item.final_energy).sum::<f32>() / descents.len() as f32;
        let mean_log_members = descents
            .iter()
            .map(|item| (item.member_count as f32).ln())
            .sum::<f32>()
            / descents.len() as f32;
        let raw_confidence = if mean_log_members <= f32::EPSILON {
            1.0
        } else {
            1.0 - mean_energy / mean_log_members
        };
        Self {
            confidence: raw_confidence.clamp(0.0, 1.0).min(valid_ceiling(ceiling)),
            converged: descents.iter().all(|item| item.converged),
            energy: mean_energy,
        }
    }
}

fn validate_request(
    cx: &Constellation,
    panel: &Panel,
    clamp: &SlotSet,
    free: &SlotSet,
) -> Result<SlotSet, OracleError> {
    if cx.panel_version != panel.version {
        return Err(CalyxError::stale_derived(format!(
            "constellation panel {} does not match panel {}",
            cx.panel_version, panel.version
        ))
        .into());
    }
    let all_slots = panel
        .slots
        .iter()
        .map(|slot| slot.lens_id)
        .collect::<SlotSet>();
    let union = clamp.union(free).copied().collect::<SlotSet>();
    let present = present_lenses(cx, panel);
    let overlap = sorted_lenses(&clamp.intersection(free).copied().collect());
    let mut missing = all_slots.difference(&union).copied().collect::<SlotSet>();
    missing.extend(union.difference(&present));
    let extra = union.difference(&all_slots).copied().collect::<SlotSet>();

    if overlap.is_empty() && missing.is_empty() && extra.is_empty() {
        Ok(all_slots)
    } else {
        Err(OracleError::SlotConflict {
            overlap,
            missing: sorted_lenses(&missing),
            extra: sorted_lenses(&extra),
            tag_mismatch: Vec::new(),
        })
    }
}

fn present_lenses(cx: &Constellation, panel: &Panel) -> SlotSet {
    panel
        .slots
        .iter()
        .filter(|slot| cx.slots.contains_key(&slot.slot_id))
        .map(|slot| slot.lens_id)
        .collect()
}

fn dense_vectors_by_lens(
    cx: &Constellation,
    panel: &Panel,
) -> Result<BTreeMap<LensId, Vec<f32>>, OracleError> {
    let mut vectors = BTreeMap::new();
    for slot in &panel.slots {
        let Some(vector) = cx.slots.get(&slot.slot_id) else {
            continue;
        };
        match vector {
            SlotVector::Dense { data, .. } => {
                vectors.insert(slot.lens_id, data.clone());
            }
            SlotVector::Absent { .. } => {}
            SlotVector::Sparse { .. } | SlotVector::Multi { .. } => {
                return Err(CalyxError::lens_dim_mismatch(format!(
                    "complete requires dense vectors for lens {}",
                    slot.lens_id
                ))
                .into());
            }
        }
    }
    Ok(vectors)
}

fn measured_slots(
    clamp: &SlotSet,
    slot_vectors: &BTreeMap<LensId, Vec<f32>>,
) -> Result<Vec<TaggedSlot>, OracleError> {
    let mut slots = Vec::new();
    for lens_id in sorted_lenses(clamp) {
        let Some(vector) = slot_vectors.get(&lens_id) else {
            return Err(OracleError::SlotConflict {
                overlap: Vec::new(),
                missing: vec![lens_id],
                extra: Vec::new(),
                tag_mismatch: Vec::new(),
            });
        };
        slots.push(TaggedSlot {
            lens_id,
            vector: vector.clone(),
            tag: SlotTag::Measured,
        });
    }
    Ok(slots)
}

fn mean_vector(members: &[Vec<f32>]) -> Vec<f32> {
    let Some(first) = members.first() else {
        return Vec::new();
    };
    let mut mean = vec![0.0; first.len()];
    for member in members {
        for (dst, src) in mean.iter_mut().zip(member) {
            *dst += *src;
        }
    }
    for value in &mut mean {
        *value /= members.len() as f32;
    }
    mean
}

fn sort_tagged_slots(mut slots: Vec<TaggedSlot>) -> Vec<TaggedSlot> {
    slots.sort_by_key(|slot| slot.lens_id);
    slots
}

fn sorted_lenses(ids: &SlotSet) -> Vec<LensId> {
    let mut sorted = ids.iter().copied().collect::<Vec<_>>();
    sorted.sort_unstable();
    sorted
}

fn sorted_lens_strings(ids: &SlotSet) -> Vec<String> {
    sorted_lenses(ids)
        .into_iter()
        .map(|lens| lens.to_string())
        .collect()
}

fn slot_id_for_lens(panel: &Panel, lens_id: LensId) -> Option<calyx_core::SlotId> {
    panel
        .slots
        .iter()
        .find(|slot| slot.lens_id == lens_id)
        .map(|slot| slot.slot_id)
}

fn valid_ceiling(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn completion_subject(payload: &CompletionLedgerPayload) -> [u8; 16] {
    let mut slot_bytes = Vec::new();
    for lens in payload.clamp.iter().chain(payload.free.iter()) {
        slot_bytes.extend_from_slice(lens.as_bytes());
        slot_bytes.push(0);
    }
    content_address([
        COMPLETION_LEDGER_TAG.as_bytes(),
        payload.domain_id.as_bytes(),
        payload.cx_id.as_bytes(),
        slot_bytes.as_slice(),
    ])
}

#[allow(dead_code)]
#[cfg(test)]
#[path = "complete_tests.rs"]
mod tests;
