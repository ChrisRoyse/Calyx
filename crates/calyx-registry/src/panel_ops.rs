use calyx_assay::store::{AssayCacheKey, AssayStore, AssaySubject};
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
        .map(|slot| listing_for_slot(slot, registry))
        .collect()
}

pub fn list_panel_with_assay(
    panel: &Panel,
    registry: &Registry,
    assay_store: &AssayStore,
    cache_key: &AssayCacheKey,
) -> Vec<PanelSlotListing> {
    panel
        .slots
        .iter()
        .map(|slot| {
            let mut listing = listing_for_slot(slot, registry);
            if let Some(row) =
                assay_store.get(cache_key, &AssaySubject::Lens { slot: slot.slot_id })
            {
                listing.bits_about = Some(row.estimate.bits);
            }
            listing
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

fn listing_for_slot(slot: &Slot, registry: &Registry) -> PanelSlotListing {
    PanelSlotListing {
        slot_id: slot.slot_id,
        key: slot.slot_key.key().to_string(),
        lens_id: slot.lens_id,
        state: slot.state,
        bits_about: slot_bits(slot),
        health: registry
            .health(slot.lens_id)
            .unwrap_or_else(|err| LensHealth::Failing {
                code: "CALYX_LENS_UNREACHABLE".to_string(),
                reason: err.message,
            }),
    }
}

fn slot_bits(slot: &Slot) -> Option<f32> {
    slot.bits_about
        .values()
        .map(|signal| signal.bits)
        .max_by(|left, right| left.total_cmp(right))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use calyx_assay::estimate::{EstimatorKind, MiEstimate, TrustTag};
    use calyx_assay::store::{AssayCacheKey, AssayStore, AssaySubject};
    use calyx_core::{
        AnchorKind, Asymmetry, ConfidenceInterval, Modality, Panel, QuantPolicy, Signal, SlotShape,
        VaultId,
    };

    use super::*;
    use crate::runtime::algorithmic::AlgorithmicLens;

    #[test]
    fn list_panel_uses_stored_slot_bits() {
        let (registry, lens_id) = registry_with_lens();
        let panel = panel_with_slot(lens_id, Some(0.31));

        let listing = list_panel(&panel, &registry);

        assert_eq!(listing[0].bits_about, Some(0.31));
    }

    #[test]
    fn list_panel_with_assay_overlays_scoped_assay_bits() {
        let (registry, lens_id) = registry_with_lens();
        let panel = panel_with_slot(lens_id, Some(0.31));
        let cache_key = assay_key();
        let mut store = AssayStore::default();
        store.put(
            cache_key.clone(),
            AssaySubject::Lens {
                slot: panel.slots[0].slot_id,
            },
            MiEstimate::point(0.47, 72, EstimatorKind::Ksg, TrustTag::Trusted),
            "panel assay bits",
            12,
        );

        let listing = list_panel_with_assay(&panel, &registry, &store, &cache_key);

        assert_eq!(listing[0].bits_about, Some(0.47));
    }

    fn registry_with_lens() -> (Registry, LensId) {
        let mut registry = Registry::new();
        let lens = AlgorithmicLens::byte_features("panel-assay-list", Modality::Text);
        let lens_id = registry
            .register_frozen(lens.clone(), lens.contract().clone())
            .unwrap();
        (registry, lens_id)
    }

    fn panel_with_slot(lens_id: LensId, bits: Option<f32>) -> Panel {
        let slot_id = SlotId::new(0);
        let mut bits_about = BTreeMap::new();
        if let Some(bits) = bits {
            bits_about.insert(
                AnchorKind::Reward,
                Signal {
                    bits,
                    ci: ConfidenceInterval {
                        low: bits - 0.01,
                        high: bits + 0.01,
                    },
                    n: 64,
                    estimator: "unit".to_string(),
                    ts: 1,
                },
            );
        }
        Panel {
            version: 1,
            slots: vec![Slot {
                slot_id,
                slot_key: SlotKey::new(slot_id, "panel-assay".to_string()),
                lens_id,
                shape: SlotShape::Dense(4),
                modality: Modality::Text,
                asymmetry: Asymmetry::None,
                quant: QuantPolicy::None,
                resource: Default::default(),
                axis: None,
                retrieval_only: false,
                excluded_from_dedup: false,
                bits_about,
                state: SlotState::Active,
                added_at_panel_version: 1,
            }],
            created_at: 1,
            kernel_ref: None,
            guard_ref: None,
        }
    }

    fn assay_key() -> AssayCacheKey {
        AssayCacheKey::scoped(1, "panel-unit", vault_id(), AnchorKind::Reward)
    }

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
    }
}
