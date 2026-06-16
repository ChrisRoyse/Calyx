use calyx_core::{Constellation, SlotState};
use calyx_registry::VaultPanelState;
use serde_json::{Value, json};

pub(super) fn constellation_report(cx: &Constellation, state: &VaultPanelState) -> Value {
    let slots = state
        .panel
        .slots
        .iter()
        .map(|slot| {
            json!({
                "slot": slot.slot_id.get(),
                "name": slot.slot_key.key(),
                "state": state_name(slot.state),
                "lens_id": slot.lens_id.to_string(),
                "vector": cx.slots.get(&slot.slot_id),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "cx_id": cx.cx_id.to_string(),
        "vault_id": cx.vault_id.to_string(),
        "panel_version": cx.panel_version,
        "created_at": cx.created_at,
        "modality": cx.modality,
        "input_ref": cx.input_ref,
        "slots": slots,
        "scalars": cx.scalars,
        "anchors": cx.anchors,
        "flags": cx.flags,
    })
}

fn state_name(state: SlotState) -> &'static str {
    match state {
        SlotState::Active => "active",
        SlotState::Parked => "parked",
        SlotState::Retired => "retired",
    }
}
