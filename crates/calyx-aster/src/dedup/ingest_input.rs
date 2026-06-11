use std::collections::BTreeMap;

use calyx_core::{
    Anchor, Clock, Constellation, CxFlags, InputRef, LedgerRef, Modality, Result, SlotId,
    SlotVector,
};
use serde::{Deserialize, Serialize};

use super::dedup_error;
use crate::vault::AsterVault;

pub const CALYX_DEDUP_INVALID_EVENT_TIME: &str = "CALYX_DEDUP_INVALID_EVENT_TIME";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EpochSecs(pub i64);

impl EpochSecs {
    pub fn to_u64(self) -> Result<u64> {
        u64::try_from(self.0).map_err(|_| {
            dedup_error(
                CALYX_DEDUP_INVALID_EVENT_TIME,
                format!("event time {} is before Unix epoch", self.0),
            )
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IngestInput {
    pub raw_bytes: Vec<u8>,
    pub panel_version: u32,
    pub modality: Modality,
    pub slots: BTreeMap<SlotId, SlotVector>,
    pub scalars: BTreeMap<String, f64>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub anchors: Vec<Anchor>,
    pub input_pointer: Option<String>,
    pub redacted: bool,
    #[serde(default)]
    pub temporal_slot_ids: Vec<SlotId>,
}

impl IngestInput {
    pub fn new(raw_bytes: impl Into<Vec<u8>>, panel_version: u32, modality: Modality) -> Self {
        Self {
            raw_bytes: raw_bytes.into(),
            panel_version,
            modality,
            slots: BTreeMap::new(),
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: Vec::new(),
            input_pointer: None,
            redacted: true,
            temporal_slot_ids: Vec::new(),
        }
    }

    pub fn with_slot(mut self, slot: SlotId, vector: SlotVector) -> Self {
        self.slots.insert(slot, vector);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    pub fn with_anchor(mut self, anchor: Anchor) -> Self {
        self.anchors.push(anchor);
        self
    }

    pub fn with_temporal_slot(mut self, slot: SlotId) -> Self {
        if !self.temporal_slot_ids.contains(&slot) {
            self.temporal_slot_ids.push(slot);
        }
        self
    }

    pub fn with_temporal_slots(mut self, slots: impl IntoIterator<Item = SlotId>) -> Self {
        for slot in slots {
            if !self.temporal_slot_ids.contains(&slot) {
                self.temporal_slot_ids.push(slot);
            }
        }
        self
    }

    pub fn temporal_slot_ids(&self) -> &[SlotId] {
        &self.temporal_slot_ids
    }

    pub(crate) fn to_constellation<C>(
        &self,
        vault: &AsterVault<C>,
        at: EpochSecs,
    ) -> Result<Constellation>
    where
        C: Clock,
    {
        let event_time = at.to_u64()?;
        let input_hash = *blake3::hash(&self.raw_bytes).as_bytes();
        let mut scalars = self.scalars.clone();
        scalars.insert("event_time_secs".to_string(), at.0 as f64);
        Ok(Constellation {
            cx_id: vault.cx_id_for_input(&self.raw_bytes, self.panel_version),
            vault_id: vault.vault_id(),
            panel_version: self.panel_version,
            created_at: event_time,
            input_ref: InputRef {
                hash: input_hash,
                pointer: self.input_pointer.clone(),
                redacted: self.redacted,
            },
            modality: self.modality,
            slots: self.slots.clone(),
            scalars,
            metadata: self.metadata.clone(),
            anchors: self.anchors.clone(),
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
            flags: CxFlags {
                ungrounded: self.anchors.is_empty(),
                redacted_input: self.redacted,
                ..CxFlags::default()
            },
        })
    }
}
