use std::collections::BTreeMap;

use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, Slot, SlotId, SlotVector, VaultId,
};
use calyx_registry::frozen::sha256_digest;

use super::{FSV_TS, ROWS};

pub fn cross_modal_samples() -> (Vec<Vec<f32>>, Vec<Vec<f32>>, Vec<bool>) {
    let mut left = Vec::with_capacity(ROWS);
    let mut right = Vec::with_capacity(ROWS);
    let mut labels = Vec::with_capacity(ROWS);
    for _ in 0..48 {
        left.push(vec![1.0]);
        right.push(vec![1.0]);
        labels.push(true);
    }
    for (a, b) in [(1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        for _ in 0..16 {
            left.push(vec![a]);
            right.push(vec![b]);
            labels.push(false);
        }
    }
    (left, right, labels)
}

pub fn slot_map_for(
    index: usize,
    slots: &[Slot],
    left_samples: &[Vec<f32>],
    right_samples: &[Vec<f32>],
) -> BTreeMap<SlotId, Vec<f32>> {
    let mut map = BTreeMap::new();
    for slot in slots {
        let scalar = match slot.axis.as_deref() {
            Some("image_visual") => left_samples[index][0],
            Some("protein_sequence") => right_samples[index][0],
            _ => ((index as f32 + f32::from(slot.slot_id.get()) + 1.0).sin()).max(-0.95),
        };
        map.insert(slot.slot_id, expanded_vector(scalar, slot.slot_id.get()));
    }
    map
}

pub fn constellation(
    index: usize,
    slots: &BTreeMap<SlotId, Vec<f32>>,
    vault_id: VaultId,
) -> Constellation {
    let slot_vectors = slots
        .iter()
        .map(|(slot, data)| {
            (
                *slot,
                SlotVector::Dense {
                    dim: data.len() as u32,
                    data: data.clone(),
                },
            )
        })
        .collect();
    Constellation {
        cx_id: cx(index),
        vault_id,
        panel_version: 1,
        created_at: FSV_TS + index as u64,
        input_ref: InputRef {
            hash: sha256_digest(&[format!("issue792-{index}").as_bytes()]),
            pointer: Some(format!("fixture://issue792/{index}")),
            redacted: false,
        },
        modality: Modality::Mixed,
        slots: slot_vectors,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: index as u64,
            hash: [index as u8; 32],
        },
        flags: CxFlags::default(),
    }
}

pub fn neff_for_active(n: usize) -> calyx_assay::NeffReport {
    let mut matrix = vec![vec![0.08; n]; n];
    for (idx, row) in matrix.iter_mut().enumerate() {
        row[idx] = 1.0;
    }
    calyx_assay::stable_rank(&matrix)
}

pub fn slot_by_axis<'a>(slots: &'a [Slot], axis: &str) -> &'a Slot {
    slots
        .iter()
        .find(|slot| slot.axis.as_deref() == Some(axis))
        .unwrap()
}

pub fn cx(index: usize) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..].copy_from_slice(&(index as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

fn expanded_vector(scalar: f32, salt: u16) -> Vec<f32> {
    (0..16)
        .map(|idx| scalar + ((idx + usize::from(salt)) as f32).sin() * 0.01)
        .collect()
}
