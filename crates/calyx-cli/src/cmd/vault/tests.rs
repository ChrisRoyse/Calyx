use super::*;

#[test]
fn already_retired_slot_is_structured_error() {
    let mut panel = calyx_registry::instantiate_panel(&text_default(), 0).panel;
    let slot = panel.slots[0].slot_id;
    panel.slots[0].state = SlotState::Retired;

    let err = ensure_slot_can_transition(&panel, slot, LensStateAction::Retire).unwrap_err();
    assert_eq!(err.code(), "CALYX_LENS_FROZEN_VIOLATION");
}
