//! Public Oracle contract types for consequence prediction.

use std::{collections::HashSet, fmt};

use calyx_core::{AnchorValue, LedgerRef, LensId};
use calyx_ward::GuardVerdict;
use serde::{Deserialize, Serialize};

pub const DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DomainId(String);

impl DomainId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DomainId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<&str> for DomainId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for DomainId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prediction {
    pub outcome: AnchorValue,
    pub confidence: f32,
    pub consequences: Vec<Consequence>,
    pub bound: SufficiencyBound,
    pub provenance: LedgerRef,
    pub guard: GuardVerdict,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SufficiencyBound {
    #[serde(rename = "I_panel_oracle")]
    pub i_panel_oracle: f32,
    pub dpi_ceiling: f32,
    pub sufficient: bool,
    pub per_sensor_deficit: Vec<(LensId, f32)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OracleSelfConsistency {
    pub flakiness: f32,
    pub validity: f32,
    pub ceiling: f32,
    #[serde(default)]
    pub provisional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<LedgerRef>,
}

impl OracleSelfConsistency {
    pub fn measured(flakiness: f32, validity: f32) -> Self {
        Self::with_provenance(flakiness, validity, false, None)
    }

    pub fn provisional(flakiness: f32, validity: f32) -> Self {
        Self::with_provenance(flakiness, validity, true, None)
    }

    pub fn with_provenance(
        flakiness: f32,
        validity: f32,
        provisional: bool,
        provenance: Option<LedgerRef>,
    ) -> Self {
        Self {
            flakiness,
            validity,
            ceiling: validity * (1.0 - flakiness),
            provisional,
            provenance,
        }
    }
}

pub type SlotSet = HashSet<LensId>;

#[derive(Clone, Copy, Debug)]
pub struct CompletionSlotPartition<'a> {
    pub all_slots: &'a SlotSet,
    pub clamp: &'a SlotSet,
    pub free: &'a SlotSet,
}

impl<'a> CompletionSlotPartition<'a> {
    pub fn new(all_slots: &'a SlotSet, clamp: &'a SlotSet, free: &'a SlotSet) -> Self {
        Self {
            all_slots,
            clamp,
            free,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotTag {
    Measured,
    Inferred,
    Provisional,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaggedSlot {
    pub lens_id: LensId,
    pub vector: Vec<f32>,
    pub tag: SlotTag,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompletionResult {
    pub filled_cx: Vec<TaggedSlot>,
    pub confidence: f32,
    pub converged: bool,
    pub energy: f32,
    pub provenance: LedgerRef,
}

impl CompletionResult {
    pub fn new(
        filled_cx: Vec<TaggedSlot>,
        confidence: f32,
        converged: bool,
        energy: f32,
        provenance: LedgerRef,
        partition: CompletionSlotPartition<'_>,
    ) -> Result<Self, crate::OracleError> {
        validate_completion_slots(&filled_cx, partition)?;
        Ok(Self {
            filled_cx,
            confidence,
            converged,
            energy,
            provenance,
        })
    }

    pub fn inferred_slots(&self) -> Vec<&TaggedSlot> {
        self.slots_with_tag(SlotTag::Inferred)
    }

    pub fn provisional_slots(&self) -> Vec<&TaggedSlot> {
        self.slots_with_tag(SlotTag::Provisional)
    }

    pub fn measured_slots(&self) -> Vec<&TaggedSlot> {
        self.slots_with_tag(SlotTag::Measured)
    }

    fn slots_with_tag(&self, tag: SlotTag) -> Vec<&TaggedSlot> {
        self.filled_cx
            .iter()
            .filter(|slot| slot.tag == tag)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Consequence {
    pub action_or_event: String,
    pub domain: DomainId,
    pub outcome: AnchorValue,
    pub confidence: f32,
    pub hop: u8,
    pub provenance: LedgerRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsequenceTree {
    pub root: Consequence,
    pub children: Vec<ConsequenceTree>,
    pub max_depth: u8,
}

impl ConsequenceTree {
    pub fn leaf(root: Consequence) -> Self {
        Self {
            root,
            children: Vec::new(),
            max_depth: DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH,
        }
    }
}

fn validate_completion_slots(
    filled_cx: &[TaggedSlot],
    partition: CompletionSlotPartition<'_>,
) -> Result<(), crate::OracleError> {
    let all_slots = partition.all_slots;
    let clamp = partition.clamp;
    let free = partition.free;
    let union: SlotSet = clamp.union(free).copied().collect();
    let filled: SlotSet = filled_cx.iter().map(|slot| slot.lens_id).collect();

    let overlap = sorted_lens_ids(clamp.intersection(free).copied());
    let mut missing: SlotSet = all_slots.difference(&union).copied().collect();
    missing.extend(all_slots.difference(&filled).copied());
    let mut extra: SlotSet = union.difference(all_slots).copied().collect();
    extra.extend(filled.difference(all_slots).copied());

    let tag_mismatch = sorted_lens_ids(filled_cx.iter().filter_map(|slot| {
        let clamped_wrong = clamp.contains(&slot.lens_id) && slot.tag != SlotTag::Measured;
        let free_wrong = free.contains(&slot.lens_id) && slot.tag == SlotTag::Measured;
        (clamped_wrong || free_wrong).then_some(slot.lens_id)
    }));

    if overlap.is_empty() && missing.is_empty() && extra.is_empty() && tag_mismatch.is_empty() {
        return Ok(());
    }

    Err(crate::OracleError::SlotConflict {
        overlap,
        missing: sorted_lens_ids(missing),
        extra: sorted_lens_ids(extra),
        tag_mismatch,
    })
}

fn sorted_lens_ids(ids: impl IntoIterator<Item = LensId>) -> Vec<LensId> {
    let mut ids: Vec<_> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
