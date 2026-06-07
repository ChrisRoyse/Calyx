use calyx_core::{LensId, Panel, Slot, SlotId, SlotKey, SlotState, Ts};
use serde::{Deserialize, Serialize};

use crate::Registry;
use crate::panels::{PanelTemplate, instantiate_panel};
use crate::spec::LensHealth;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelDiff {
    pub added: Vec<SlotId>,
    pub retired: Vec<SlotId>,
    pub unchanged: Vec<SlotId>,
    pub panel_version: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelSlotListing {
    pub slot_id: SlotId,
    pub key: String,
    pub lens_id: LensId,
    pub state: SlotState,
    pub bits_about: Option<f32>,
    pub health: LensHealth,
}

pub fn list_panel(panel: &Panel, registry: &Registry) -> Vec<PanelSlotListing> {
    panel
        .slots
        .iter()
        .map(|slot| PanelSlotListing {
            slot_id: slot.slot_id,
            key: slot.slot_key.key().to_string(),
            lens_id: slot.lens_id,
            state: slot.state,
            bits_about: None,
            health: registry
                .health(slot.lens_id)
                .unwrap_or_else(|err| LensHealth::Failing {
                    code: "CALYX_LENS_UNREACHABLE".to_string(),
                    reason: err.message,
                }),
        })
        .collect()
}

pub fn swap_panel(panel: &mut Panel, template: &PanelTemplate, now: Ts) -> PanelDiff {
    let target = instantiate_panel(template, now);
    let target_ids = target
        .panel
        .slots
        .iter()
        .map(|slot| slot.lens_id)
        .collect::<Vec<_>>();
    let mut added = Vec::new();
    let mut retired = Vec::new();
    let mut unchanged = Vec::new();

    for slot in &mut panel.slots {
        if target_ids.contains(&slot.lens_id) && slot.state != SlotState::Retired {
            unchanged.push(slot.slot_id);
        } else if slot.state != SlotState::Retired {
            slot.state = SlotState::Retired;
            retired.push(slot.slot_id);
        }
    }

    let mut next_id = panel
        .slots
        .iter()
        .map(|slot| slot.slot_id.get())
        .max()
        .map_or(0, |id| id.saturating_add(1));
    for target_slot in &target.panel.slots {
        let exists = panel
            .slots
            .iter()
            .any(|slot| slot.lens_id == target_slot.lens_id && slot.state != SlotState::Retired);
        if exists {
            continue;
        }
        let slot_id = SlotId::new(next_id);
        next_id = next_id.saturating_add(1);
        panel.slots.push(cloned_target_slot(target_slot, slot_id));
        added.push(slot_id);
    }

    if !added.is_empty() || !retired.is_empty() {
        panel.version = panel.version.saturating_add(1);
        panel.created_at = now;
        for slot in &mut panel.slots {
            if added.contains(&slot.slot_id) {
                slot.added_at_panel_version = panel.version;
            }
        }
    }

    PanelDiff {
        added,
        retired,
        unchanged,
        panel_version: panel.version,
    }
}

fn cloned_target_slot(target: &Slot, slot_id: SlotId) -> Slot {
    let mut slot = target.clone();
    slot.slot_id = slot_id;
    slot.slot_key = SlotKey::new(slot_id, target.slot_key.key().to_string());
    slot
}
