#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_core::{CxId, Input, LensId, SlotVector};
use calyx_ledger::{ForgeBackend, RecordedSlot, ReproduceInputResolver, ReproduceLensRegistry};

#[derive(Default)]
pub struct RecordingForge {
    pub seeds: Vec<u64>,
}

impl ForgeBackend for RecordingForge {
    fn activate_determinism(&mut self, seed: u64) -> calyx_core::Result<()> {
        self.seeds.push(seed);
        Ok(())
    }
}

#[derive(Default)]
pub struct RecordingRegistry {
    pub weights: BTreeMap<LensId, [u8; 32]>,
    vectors: BTreeMap<LensId, SlotVector>,
    input_measure: Option<fn(&Input) -> SlotVector>,
}

impl RecordingRegistry {
    pub fn from_slots_with_input_fn(
        slots: &[RecordedSlot],
        input_measure: fn(&Input) -> SlotVector,
    ) -> Self {
        Self {
            weights: weights_from_slots(slots),
            vectors: BTreeMap::new(),
            input_measure: Some(input_measure),
        }
    }

    pub fn from_slots_with_vectors<F>(slots: &[RecordedSlot], vector_for_slot: F) -> Self
    where
        F: Fn(&RecordedSlot) -> SlotVector,
    {
        Self {
            weights: weights_from_slots(slots),
            vectors: slots
                .iter()
                .map(|slot| (slot.lens_id, vector_for_slot(slot)))
                .collect(),
            input_measure: None,
        }
    }
}

impl ReproduceLensRegistry for RecordingRegistry {
    fn frozen_weights_sha256(&self, lens_id: LensId) -> calyx_core::Result<[u8; 32]> {
        self.weights.get(&lens_id).copied().ok_or_else(|| {
            calyx_core::CalyxError::lens_frozen_violation(format!(
                "lens {lens_id} has no frozen snapshot"
            ))
        })
    }

    fn measure_frozen(&self, lens_id: LensId, input: &Input) -> calyx_core::Result<SlotVector> {
        if let Some(vector) = self.vectors.get(&lens_id) {
            return Ok(vector.clone());
        }
        if let Some(input_measure) = self.input_measure {
            return Ok(input_measure(input));
        }
        Err(calyx_core::CalyxError::lens_unreachable("missing vector"))
    }
}

pub struct SlotInputResolver {
    inputs: BTreeMap<[u8; 32], Input>,
    missing_message: &'static str,
}

impl SlotInputResolver {
    pub fn from_slots(slots: &[RecordedSlot], missing_message: &'static str) -> Self {
        Self {
            inputs: slots
                .iter()
                .map(|slot| (slot.input_hash, slot.input.clone().unwrap()))
                .collect(),
            missing_message,
        }
    }
}

impl ReproduceInputResolver for SlotInputResolver {
    fn resolve_input(&self, slot: &RecordedSlot) -> calyx_core::Result<Input> {
        self.inputs
            .get(&slot.input_hash)
            .cloned()
            .ok_or_else(|| calyx_core::CalyxError::ledger_corrupt(self.missing_message))
    }
}

pub fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

pub fn dense(scores: &[f32]) -> SlotVector {
    SlotVector::Dense {
        dim: scores.len() as u32,
        data: scores.to_vec(),
    }
}

pub fn rrf(weight: f32, rank: usize) -> f32 {
    weight / (rank as f32 + 60.0)
}

pub fn encode_vector_bytes(label: &[u8], vector: &SlotVector) -> Vec<u8> {
    let mut bytes = label.to_vec();
    bytes.push(0xff);
    bytes.extend_from_slice(&serde_json::to_vec(vector).unwrap());
    bytes
}

pub fn decode_vector_bytes(bytes: &[u8]) -> SlotVector {
    let json = bytes.split(|byte| *byte == 0xff).nth(1).unwrap();
    serde_json::from_slice(json).unwrap()
}

pub fn reset_child_dir(root: &Path, child: &Path) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if child.exists() {
        let child_canonical = child.canonicalize().expect("canonical child path");
        assert!(child_canonical.starts_with(&root));
        fs::remove_dir_all(&child_canonical).expect("remove stale child dir");
    }
    fs::create_dir_all(child).expect("create child dir");
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn weights_from_slots(slots: &[RecordedSlot]) -> BTreeMap<LensId, [u8; 32]> {
    slots
        .iter()
        .map(|slot| (slot.lens_id, slot.weights_sha256))
        .collect()
}
