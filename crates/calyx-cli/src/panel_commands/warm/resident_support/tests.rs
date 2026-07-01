use super::*;
use calyx_core::{Asymmetry, LensId, QuantPolicy, SlotKey, SlotResource, SlotShape};
use std::collections::BTreeMap;
use std::str::FromStr;

fn slot(id: u16, placement: Placement, state: SlotState, modality: Modality) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("slot_{id:02}")),
        lens_id: LensId::from_str("11111111111111111111111111111111").unwrap(),
        shape: SlotShape::Dense(4),
        modality,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: SlotResource {
            placement,
            ..SlotResource::default()
        },
        axis: None,
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state,
        added_at_panel_version: 1,
    }
}

fn panel(slots: Vec<Slot>) -> Panel {
    Panel {
        version: 1,
        slots,
        created_at: 0,
        kernel_ref: None,
        guard_ref: None,
    }
}

#[test]
fn resident_slot_scope_prunes_to_selected_gpu_slots() {
    let mut panel = panel(vec![
        slot(1, Placement::Gpu, SlotState::Active, Modality::Text),
        slot(2, Placement::Cpu, SlotState::Active, Modality::Text),
        slot(3, Placement::Gpu, SlotState::Active, Modality::Code),
    ]);

    apply_resident_slot_scope(
        "vault:/tmp/scope",
        &mut panel,
        &[SlotId::new(1)],
        Some(Modality::Text),
    )
    .unwrap();

    assert_eq!(panel.slots.len(), 1);
    assert_eq!(panel.slots[0].slot_id, SlotId::new(1));
    assert_eq!(panel.slots[0].resource.placement, Placement::Gpu);
}

#[test]
fn resident_slot_scope_refuses_selected_cpu_slot() {
    let mut panel = panel(vec![slot(
        21,
        Placement::Cpu,
        SlotState::Active,
        Modality::Text,
    )]);

    let error = apply_resident_slot_scope(
        "vault:/tmp/scope",
        &mut panel,
        &[SlotId::new(21)],
        Some(Modality::Text),
    )
    .unwrap_err();

    assert_eq!(error.code(), RESIDENT_CPU_LENS_REFUSED);
    assert!(error.message().contains("selected CPU/non-GPU"));
}

#[test]
fn resident_slot_scope_refuses_missing_or_duplicate_slots() {
    let mut panel = panel(vec![slot(
        1,
        Placement::Gpu,
        SlotState::Active,
        Modality::Text,
    )]);

    let duplicate = normalized_slot_scope("vault:/tmp/scope", vec![SlotId::new(1), SlotId::new(1)])
        .unwrap_err();
    assert_eq!(duplicate.code(), "CALYX_PANEL_RESIDENT_SLOT_SCOPE_INVALID");

    let missing =
        apply_resident_slot_scope("vault:/tmp/scope", &mut panel, &[SlotId::new(9)], None)
            .unwrap_err();
    assert_eq!(missing.code(), "CALYX_PANEL_RESIDENT_SLOT_SCOPE_INVALID");
}

#[test]
fn resident_slot_scope_refuses_modality_mismatch() {
    let mut panel = panel(vec![slot(
        3,
        Placement::Gpu,
        SlotState::Active,
        Modality::Code,
    )]);

    let error = apply_resident_slot_scope(
        "vault:/tmp/scope",
        &mut panel,
        &[SlotId::new(3)],
        Some(Modality::Text),
    )
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_PANEL_RESIDENT_SLOT_SCOPE_INVALID");
    assert!(error.message().contains("does not match --modality"));
}
