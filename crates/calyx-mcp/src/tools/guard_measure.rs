use std::collections::BTreeMap;

use calyx_core::{CalyxError, Input, Modality, SlotId, SlotState};
use calyx_registry::VaultPanelState;

use crate::server::ToolResult;

pub(super) fn required_dense_vectors(
    state: &VaultPanelState,
    text: &str,
    required_slots: &[SlotId],
) -> ToolResult<BTreeMap<SlotId, Vec<f32>>> {
    let input = Input::new(Modality::Text, text.as_bytes().to_vec());
    let mut out = BTreeMap::new();
    for required_slot in required_slots {
        let slot = state
            .panel
            .slots
            .iter()
            .find(|slot| slot.slot_id == *required_slot)
            .ok_or_else(|| {
                CalyxError::stale_derived(format!(
                    "required guard slot {required_slot} is absent from the persisted panel"
                ))
            })?;
        if slot.state != SlotState::Active {
            return Err(CalyxError::stale_derived(format!(
                "required guard slot {required_slot} is not active"
            ))
            .into());
        }
        if slot.modality != Modality::Text {
            return Err(CalyxError::stale_derived(format!(
                "required guard slot {required_slot} is not a text slot"
            ))
            .into());
        }
        if !state.registry.contains(slot.lens_id) {
            return Err(CalyxError::stale_derived(format!(
                "required guard slot {required_slot} has no registered lens"
            ))
            .into());
        }
        let vector = state.registry.measure(slot.lens_id, &input)?;
        let values = vector.as_dense().ok_or_else(|| {
            CalyxError::stale_derived(format!(
                "required guard slot {required_slot} did not produce a dense vector"
            ))
        })?;
        if values.is_empty() {
            return Err(CalyxError::stale_derived(format!(
                "required guard slot {required_slot} produced an empty dense vector"
            ))
            .into());
        }
        out.insert(*required_slot, values.to_vec());
    }
    Ok(out)
}
