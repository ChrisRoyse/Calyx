use std::collections::BTreeMap;

use calyx_core::{
    Asymmetry, Constellation, CxFlags, InputRef, LedgerRef, LensId, Modality, Panel, QuantPolicy,
    Slot, SlotKey, SlotShape, SlotState, SlotVector, VaultId,
};
use proptest::prelude::*;

use super::*;
use crate::dedup::{DedupAction, TauStrategy};

#[test]
fn content_agrees_and_temporal_differs_is_recurrence_signature() {
    let existing = cx(1, content([1.0, 0.0]), temporal([1.0, 0.0]));
    let new = cx(2, content(cos_vector(0.95)), temporal([0.0, 1.0]));

    let result = detect_recurrence_signature(
        &new,
        &existing,
        &config(0.9),
        &[temporal_slot()],
        None,
        EpochSecs(200),
    )
    .expect("signature");

    assert_eq!(
        result,
        SignatureResult::RecurrenceSignature {
            same_action: existing.cx_id,
            new_time: EpochSecs(200)
        }
    );
}

#[test]
fn same_temporal_slot_is_same_time() {
    let existing = cx(1, content([1.0, 0.0]), temporal([1.0, 0.0]));
    let new = cx(2, content(cos_vector(0.95)), temporal([1.0, 0.0]));

    let result = detect_recurrence_signature(
        &new,
        &existing,
        &config(0.9),
        &[temporal_slot()],
        None,
        EpochSecs(200),
    )
    .expect("signature");

    assert_eq!(result, SignatureResult::SameTime);
}

#[test]
fn content_below_tau_is_content_mismatch() {
    let existing = cx(1, content([1.0, 0.0]), temporal([1.0, 0.0]));
    let new = cx(2, content(cos_vector(0.85)), temporal([0.0, 1.0]));

    let result = detect_recurrence_signature(
        &new,
        &existing,
        &config(0.9),
        &[temporal_slot()],
        None,
        EpochSecs(200),
    )
    .expect("signature");

    assert_eq!(result, SignatureResult::ContentMismatch);
}

#[test]
fn default_panel_temporal_slots_are_discovered() {
    let panel = Panel {
        version: 41,
        slots: vec![
            slot(0, "content_semantic", false),
            slot(5, "E2_recency", true),
            slot(6, "E3_periodic", true),
            slot(7, "E4_positional", true),
        ],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    };

    assert_eq!(
        temporal_slot_ids_for_panel(&panel),
        vec![SlotId::new(5), SlotId::new(6), SlotId::new(7)]
    );
}

#[test]
fn empty_temporal_slots_use_event_time_fallback() {
    let existing = cx(1, content([1.0, 0.0]), temporal([1.0, 0.0]));
    let new = cx(2, content(cos_vector(0.95)), temporal([0.0, 1.0]));

    let result =
        detect_recurrence_signature(&new, &existing, &config(0.9), &[], None, EpochSecs(200))
            .expect("signature");

    assert_eq!(
        result,
        SignatureResult::RecurrenceSignature {
            same_action: existing.cx_id,
            new_time: EpochSecs(200)
        }
    );
}

#[test]
fn near_one_temporal_cosine_still_counts_as_new_time() {
    let existing = cx(1, content([1.0, 0.0]), temporal([1.0, 0.0]));
    let new = cx(2, content([1.0, 0.0]), temporal(cos_vector(0.9999)));

    let result = detect_recurrence_signature(
        &new,
        &existing,
        &config(0.9),
        &[temporal_slot()],
        None,
        EpochSecs(200),
    )
    .expect("signature");

    assert!(matches!(
        result,
        SignatureResult::RecurrenceSignature { .. }
    ));
}

#[test]
fn missing_temporal_slot_fails_closed() {
    let existing = cx(1, content([1.0, 0.0]), BTreeMap::new());
    let new = cx(2, content([1.0, 0.0]), temporal([0.0, 1.0]));

    let error = detect_recurrence_signature(
        &new,
        &existing,
        &config(0.9),
        &[temporal_slot()],
        None,
        EpochSecs(200),
    )
    .expect_err("missing temporal slot");

    assert_eq!(error.code, CALYX_RECURRENCE_SLOT_MISSING);
}

proptest! {
    #[test]
    fn identical_content_and_temporal_never_recurrence(t in 0i64..10_000) {
        let existing = cx(1, content([1.0, 0.0]), temporal([1.0, 0.0]));
        let new = cx(2, content([1.0, 0.0]), temporal([1.0, 0.0]));

        let result = detect_recurrence_signature(
            &new,
            &existing,
            &config(0.9),
            &[temporal_slot()],
            None,
            EpochSecs(t),
        ).expect("signature");

        prop_assert_eq!(result, SignatureResult::SameTime);
    }
}

fn config(tau: f32) -> TctCosineConfig {
    TctCosineConfig::new(
        vec![content_slot()],
        TauStrategy::PerSlot(vec![(content_slot(), tau)]),
        DedupAction::RecurrenceSeries,
    )
    .expect("config")
}

fn cx(
    seed: u8,
    content: BTreeMap<SlotId, SlotVector>,
    temporal: BTreeMap<SlotId, SlotVector>,
) -> Constellation {
    let mut slots = content;
    slots.extend(temporal);
    Constellation {
        cx_id: calyx_core::CxId::from_bytes([seed; 16]),
        vault_id: vault_id(),
        panel_version: 41,
        created_at: u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: true,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn content(values: [f32; 2]) -> BTreeMap<SlotId, SlotVector> {
    [(content_slot(), dense(values))].into_iter().collect()
}

fn temporal(values: [f32; 2]) -> BTreeMap<SlotId, SlotVector> {
    [(temporal_slot(), dense(values))].into_iter().collect()
}

fn dense(values: [f32; 2]) -> SlotVector {
    SlotVector::Dense {
        dim: 2,
        data: values.to_vec(),
    }
}

fn cos_vector(cosine: f32) -> [f32; 2] {
    [cosine, (1.0 - cosine * cosine).sqrt()]
}

fn slot(id: u16, key: &str, temporal: bool) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, key),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape: SlotShape::Dense(2),
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some(key.to_string()),
        retrieval_only: temporal,
        excluded_from_dedup: temporal,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: u32::from(id) + 1,
    }
}

fn content_slot() -> SlotId {
    SlotId::new(0)
}

fn temporal_slot() -> SlotId {
    SlotId::new(20)
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
