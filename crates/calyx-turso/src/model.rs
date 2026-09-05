use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{Anchor, LensId, Modality, SlotId, SlotVector};
use serde::{Deserialize, Serialize};

use crate::error;

pub const MAX_BATCH_SIZE: usize = 512;
pub const MAX_ID_BYTES: usize = 255;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SlotInput {
    pub slot_id: SlotId,
    pub lens_id: LensId,
    pub vector: SlotVector,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RecordInput {
    pub record_id: String,
    pub modality: Modality,
    #[serde(default)]
    pub content_utf8: Option<String>,
    #[serde(default)]
    pub content_hex: Option<String>,
    #[serde(default)]
    pub event_time_secs: Option<u64>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub scalars: BTreeMap<String, f64>,
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    pub slots: Vec<SlotInput>,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedRecord {
    pub record_id: String,
    pub modality_json: String,
    pub content: Vec<u8>,
    pub content_hash: [u8; 32],
    pub source_hash: [u8; 32],
    pub slot_set_hash: [u8; 32],
    pub created_at_secs: u64,
    pub metadata_json: Vec<u8>,
    pub scalars_json: Vec<u8>,
    pub anchors_json: Vec<u8>,
    pub slots: Vec<PreparedSlot>,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedSlot {
    pub slot_id: u16,
    pub lens_id: String,
    pub vector_json: Vec<u8>,
    pub vector_hash: [u8; 32],
}

#[derive(Clone, Debug, Serialize)]
pub struct DatasetInfo {
    pub dataset_id: String,
    pub vault_id: String,
    pub panel_version: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct InitReport {
    pub database_path: String,
    pub engine: &'static str,
    pub engine_version: String,
    pub journal_mode: String,
    pub schema_version: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct IngestReport {
    pub dataset_id: String,
    pub submitted: usize,
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub checkpointed_frames: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectionReport {
    pub dataset_id: String,
    pub projected: usize,
    pub batches: usize,
    pub checkpoints: usize,
    pub checkpointed_frames: i64,
    pub aster_snapshot: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct StatusReport {
    pub dataset_id: String,
    pub engine: String,
    pub engine_version: String,
    pub schema_version: i64,
    pub journal_mode: String,
    pub records: u64,
    pub slots: u64,
    pub projected: u64,
    pub pending: u64,
    pub projection_failures: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SlotState {
    pub slot_id: u16,
    pub lens_id: String,
    pub vector_hash: String,
    pub vector: SlotVector,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecordState {
    pub dataset_id: String,
    pub record_id: String,
    pub source_revision: u64,
    pub modality: Modality,
    pub content_hex: String,
    pub content_hash: String,
    pub source_hash: String,
    pub slot_set_hash: String,
    pub created_at_secs: u64,
    pub metadata: BTreeMap<String, String>,
    pub scalars: BTreeMap<String, f64>,
    pub anchors: Vec<Anchor>,
    pub slots: Vec<SlotState>,
    pub projected_cx_id: Option<String>,
    pub projected_aster_snapshot: Option<u64>,
    pub aster_match: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct VerifyReport {
    pub dataset_id: String,
    pub records_verified: u64,
    pub slots_verified: u64,
    pub turso_quick_check: String,
    pub aster_snapshot: u64,
    pub physical_match: bool,
}

impl RecordInput {
    pub(crate) fn prepare(self, now_secs: u64) -> calyx_core::Result<PreparedRecord> {
        validate_id("record_id", &self.record_id)?;
        let content = match (self.content_utf8, self.content_hex) {
            (Some(text), None) => text.into_bytes(),
            (None, Some(encoded)) => decode_hex(&encoded)?,
            (Some(_), Some(_)) => {
                return Err(error::record(
                    "record must provide exactly one of content_utf8 or content_hex",
                ));
            }
            (None, None) => {
                return Err(error::record(
                    "record must provide exactly one of content_utf8 or content_hex",
                ));
            }
        };
        if content.is_empty() {
            return Err(error::record("record content must not be empty"));
        }
        if self.slots.is_empty() {
            return Err(error::record("record slots must not be empty"));
        }
        let mut seen_slots = BTreeSet::new();
        let mut slots = Vec::with_capacity(self.slots.len());
        let mut has_indexable = false;
        for slot in self.slots {
            if !seen_slots.insert(slot.slot_id) {
                return Err(error::record(format!(
                    "record {:?} duplicates slot {}",
                    self.record_id, slot.slot_id
                )));
            }
            slot.vector.validate_schema()?;
            has_indexable |= slot.vector.is_indexable();
            let vector_json = serde_json::to_vec(&slot.vector)
                .map_err(|source| error::record(format!("serialize slot vector: {source}")))?;
            slots.push(PreparedSlot {
                slot_id: slot.slot_id.get(),
                lens_id: slot.lens_id.to_string(),
                vector_hash: *blake3::hash(&vector_json).as_bytes(),
                vector_json,
            });
        }
        if !has_indexable {
            return Err(error::record(
                "record must have at least one indexable slot; all slots were absent",
            ));
        }
        slots.sort_by_key(|slot| slot.slot_id);
        for (key, value) in &self.scalars {
            if key.is_empty() || !value.is_finite() {
                return Err(error::record(format!(
                    "scalar {key:?} must have a nonempty key and finite value"
                )));
            }
        }
        for key in self.metadata.keys() {
            if key.is_empty() || key.starts_with("calyx_turso_") {
                return Err(error::record(format!(
                    "metadata key {key:?} is empty or reserved"
                )));
            }
        }
        for anchor in &self.anchors {
            anchor.validate_schema()?;
        }
        let metadata_json = json_bytes("metadata", &self.metadata)?;
        let scalars_json = json_bytes("scalars", &self.scalars)?;
        let anchors_json = json_bytes("anchors", &self.anchors)?;
        let modality_json = serde_json::to_string(&self.modality)
            .map_err(|source| error::record(format!("serialize modality: {source}")))?;
        let content_hash = *blake3::hash(&content).as_bytes();
        let slot_set_hash = hash_slots(&slots);
        let source_hash = hash_source(
            &self.record_id,
            &modality_json,
            &content,
            self.event_time_secs,
            &metadata_json,
            &scalars_json,
            &anchors_json,
            &slots,
        );
        Ok(PreparedRecord {
            record_id: self.record_id,
            modality_json,
            content,
            content_hash,
            source_hash,
            slot_set_hash,
            created_at_secs: self.event_time_secs.unwrap_or(now_secs),
            metadata_json,
            scalars_json,
            anchors_json,
            slots,
        })
    }
}

pub(crate) fn validate_id(field: &str, value: &str) -> calyx_core::Result<()> {
    if value.trim().is_empty() || value.len() > MAX_ID_BYTES {
        return Err(error::record(format!(
            "{field} must contain 1..={MAX_ID_BYTES} bytes"
        )));
    }
    Ok(())
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(value: &str) -> calyx_core::Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(error::record("content_hex must have an even byte length"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .enumerate()
        .map(|(index, pair)| {
            let high = hex_value(pair[0]).ok_or_else(|| {
                error::record(format!("content_hex invalid at byte {}", index * 2))
            })?;
            let low = hex_value(pair[1]).ok_or_else(|| {
                error::record(format!("content_hex invalid at byte {}", index * 2 + 1))
            })?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn json_bytes(field: &str, value: &impl Serialize) -> calyx_core::Result<Vec<u8>> {
    serde_json::to_vec(value)
        .map_err(|source| error::record(format!("serialize {field}: {source}")))
}

fn hash_slots(slots: &[PreparedSlot]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for slot in slots {
        hasher.update(&slot.slot_id.to_be_bytes());
        hasher.update(&(slot.lens_id.len() as u64).to_be_bytes());
        hasher.update(slot.lens_id.as_bytes());
        hasher.update(&slot.vector_hash);
    }
    *hasher.finalize().as_bytes()
}

#[allow(clippy::too_many_arguments)]
fn hash_source(
    record_id: &str,
    modality_json: &str,
    content: &[u8],
    event_time_secs: Option<u64>,
    metadata_json: &[u8],
    scalars_json: &[u8],
    anchors_json: &[u8],
    slots: &[PreparedSlot],
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for part in [
        record_id.as_bytes(),
        modality_json.as_bytes(),
        content,
        &event_time_secs.unwrap_or_default().to_be_bytes(),
        &[u8::from(event_time_secs.is_some())],
        metadata_json,
        scalars_json,
        anchors_json,
    ] {
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    for slot in slots {
        hasher.update(&slot.slot_id.to_be_bytes());
        hasher.update(&(slot.lens_id.len() as u64).to_be_bytes());
        hasher.update(slot.lens_id.as_bytes());
        hasher.update(&(slot.vector_json.len() as u64).to_be_bytes());
        hasher.update(&slot.vector_json);
    }
    *hasher.finalize().as_bytes()
}
