//! #1120 — calibration slots must be active dense panel slots, or the
//! persisted profile would fail every guarded query at query time instead of
//! failing at calibrate time. Real `validate_calibration_slots()`, real
//! `Panel`, no mocks.

use std::collections::BTreeMap;

use calyx_core::{
    Asymmetry, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState,
};
use calyx_ward::{CalibrationInput, SlotKind, validate_calibration_slots};

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn calibration_input(slot: SlotId) -> CalibrationInput {
    CalibrationInput {
        slot,
        good_scores: (0..100).map(|i| 0.80 + i as f32 * 0.001).collect(),
        bad_scores: (0..100).map(|i| 0.30 + i as f32 * 0.003).collect(),
        slot_kind: SlotKind::Identity,
        target_far: 0.01,
    }
}

fn panel_slot(id: u16, shape: SlotShape, state: SlotState) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("slot-{id}")),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape,
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: None,
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state,
        added_at_panel_version: 1,
    }
}

/// Panel mirroring the text-default layout that triggered #1120: a dense
/// semantic slot, a sparse keywords slot (slot 1), and a multi ColBERT-style
/// slot (slot 22), plus inactive dense slots.
fn mixed_shape_panel() -> Panel {
    Panel {
        version: 7,
        slots: vec![
            panel_slot(0, SlotShape::Dense(384), SlotState::Active),
            panel_slot(1, SlotShape::Sparse(30522), SlotState::Active),
            panel_slot(22, SlotShape::Multi { token_dim: 128 }, SlotState::Active),
            panel_slot(3, SlotShape::Dense(384), SlotState::Parked),
            panel_slot(4, SlotShape::Dense(384), SlotState::Retired),
        ],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

#[test]
fn validate_accepts_active_dense_calibration_slots() {
    let inputs = vec![calibration_input(slot(0))];

    validate_calibration_slots(&inputs, &mixed_shape_panel())
        .expect("active dense slot calibrates");
}

#[test]
fn validate_rejects_sparse_calibration_slot_naming_slot_and_shape() {
    let inputs = vec![calibration_input(slot(0)), calibration_input(slot(1))];

    let error = validate_calibration_slots(&inputs, &mixed_shape_panel())
        .expect_err("sparse slot fails closed");

    assert_eq!(error.code(), "CALYX_GUARD_CALIBRATION_SLOT_SHAPE");
    assert!(error.to_string().contains("slot 1"), "{error}");
    assert!(error.to_string().contains("sparse(30522)"), "{error}");
}

#[test]
fn validate_rejects_multi_calibration_slot_naming_slot_and_shape() {
    let inputs = vec![calibration_input(slot(22))];

    let error = validate_calibration_slots(&inputs, &mixed_shape_panel())
        .expect_err("multi slot fails closed");

    assert_eq!(error.code(), "CALYX_GUARD_CALIBRATION_SLOT_SHAPE");
    assert!(error.to_string().contains("slot 22"), "{error}");
    assert!(
        error.to_string().contains("multi(token_dim=128)"),
        "{error}"
    );
}

#[test]
fn validate_rejects_slot_missing_from_panel() {
    let inputs = vec![calibration_input(slot(99))];

    let error = validate_calibration_slots(&inputs, &mixed_shape_panel())
        .expect_err("unknown slot fails closed");

    assert_eq!(error.code(), "CALYX_GUARD_CALIBRATION_SLOT_UNKNOWN");
    assert!(error.to_string().contains("slot 99"), "{error}");
    assert!(error.to_string().contains("panel version 7"), "{error}");
}

#[test]
fn validate_rejects_parked_and_retired_dense_slots() {
    for (id, state) in [(3u16, "parked"), (4u16, "retired")] {
        let inputs = vec![calibration_input(slot(id))];

        let error = validate_calibration_slots(&inputs, &mixed_shape_panel())
            .expect_err("inactive slot fails closed");

        assert_eq!(error.code(), "CALYX_GUARD_CALIBRATION_SLOT_STATE");
        assert!(error.to_string().contains(&format!("slot {id}")), "{error}");
        assert!(error.to_string().contains(state), "{error}");
    }
}
