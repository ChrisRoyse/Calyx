use calyx_assay::store::{AssayCacheKey, AssayStore, AssaySubject};
use calyx_core::{CalyxError, LensId, Modality, Panel, Slot, SlotId, SlotKey, SlotState, Ts};
use serde::{Deserialize, Serialize};

use crate::Registry;
use crate::panels::{PanelLensRuntime, PanelTemplate, instantiate_panel};
use crate::profile::{CapabilityGateDecision, CapabilityGateEvaluation};
use crate::spec::LensHealth;
use crate::swap::{LifecycleOutcome, SwapController};

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelCapabilityGateOutcome {
    pub slot_id: SlotId,
    pub lens_id: LensId,
    pub decision: CapabilityGateDecision,
    pub state: SlotState,
    pub panel_version: u32,
    pub reason: String,
}

pub const CALYX_PANEL_LENS_MISSING: &str = "CALYX_PANEL_LENS_MISSING";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppliedPanelTemplate {
    pub template_name: String,
    pub diff: PanelDiff,
    pub resolved_lenses: Vec<ResolvedPanelLens>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedPanelLens {
    pub slot_key: String,
    pub lens_name: String,
    pub lens_id: LensId,
}

pub fn list_panel(panel: &Panel, registry: &Registry) -> Vec<PanelSlotListing> {
    panel
        .slots
        .iter()
        .map(|slot| listing_for_slot(slot, registry))
        .collect()
}

pub fn apply_panel_template(
    panel: &mut Panel,
    registry: &Registry,
    template: &PanelTemplate,
    now: Ts,
) -> calyx_core::Result<AppliedPanelTemplate> {
    let mut target = instantiate_panel(template, now);
    let resolved_lenses = resolve_registry_slots(&mut target.panel, registry, template)?;
    let diff = swap_panel_to_target(panel, &target.panel, now);
    Ok(AppliedPanelTemplate {
        template_name: template.name.clone(),
        diff,
        resolved_lenses,
    })
}

pub fn apply_capability_gate(
    controller: &mut SwapController,
    slot_id: SlotId,
    evaluation: &CapabilityGateEvaluation,
    now: Ts,
) -> calyx_core::Result<PanelCapabilityGateOutcome> {
    let slot = controller
        .panel()
        .slots
        .iter()
        .find(|slot| slot.slot_id == slot_id)
        .ok_or_else(|| CalyxError::registry_unavailable(format!("slot {slot_id} not in panel")))?;
    if slot.lens_id != evaluation.lens_id {
        return Err(CalyxError::registry_unavailable(format!(
            "capability gate lens {} does not match slot {} lens {}",
            evaluation.lens_id, slot_id, slot.lens_id
        )));
    }

    let lifecycle = match evaluation.decision {
        CapabilityGateDecision::Admit if slot.state == SlotState::Active => LifecycleOutcome {
            slot_id,
            lens_id: slot.lens_id,
            state: slot.state,
            panel_version: controller.panel().version,
        },
        CapabilityGateDecision::Admit => controller.unpark_lens(slot_id, now)?,
        CapabilityGateDecision::Park => controller.park_lens(slot_id, now)?,
        CapabilityGateDecision::Retire => controller.retire_lens(slot_id, now)?,
    };

    Ok(PanelCapabilityGateOutcome {
        slot_id,
        lens_id: lifecycle.lens_id,
        decision: evaluation.decision,
        state: lifecycle.state,
        panel_version: lifecycle.panel_version,
        reason: evaluation.reason.clone(),
    })
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
    swap_panel_to_target(panel, &target.panel, now)
}

pub fn swap_panel_to_target(panel: &mut Panel, target: &Panel, now: Ts) -> PanelDiff {
    let target_ids = target
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
    for target_slot in &target.slots {
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

fn resolve_registry_slots(
    panel: &mut Panel,
    registry: &Registry,
    template: &PanelTemplate,
) -> calyx_core::Result<Vec<ResolvedPanelLens>> {
    let mut resolved = Vec::new();
    for (slot, spec) in panel.slots.iter_mut().zip(&template.slots) {
        let PanelLensRuntime::Registry { name } = &spec.runtime else {
            continue;
        };
        let lens_id = registry
            .find_lens_by_name(name)
            .ok_or_else(|| panel_lens_missing(&template.name, &spec.name, name))?;
        let contract = registry
            .frozen_contract(lens_id)
            .ok_or_else(|| panel_lens_missing(&template.name, &spec.name, name))?;
        if contract.shape() != spec.output {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "panel {} slot {} shape {:?} != lens {} frozen {:?}",
                template.name,
                spec.name,
                spec.output,
                name,
                contract.shape()
            )));
        }
        if contract.modality() != spec.modality {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "panel {} slot {} modality {:?} != lens {} frozen {:?}",
                template.name,
                spec.name,
                spec.modality,
                name,
                contract.modality()
            )));
        }
        slot.lens_id = lens_id;
        resolved.push(ResolvedPanelLens {
            slot_key: spec.name.clone(),
            lens_name: name.clone(),
            lens_id,
        });
    }
    Ok(resolved)
}

fn panel_lens_missing(template: &str, slot: &str, lens_name: &str) -> CalyxError {
    CalyxError {
        code: CALYX_PANEL_LENS_MISSING,
        message: format!(
            "panel {template} slot {slot} references missing or unconverted lens {lens_name}"
        ),
        remediation: "convert and register the missing lens before applying the panel",
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
            .unwrap_or_else(|err| missing_slot_health(slot, err)),
    }
}

fn missing_slot_health(slot: &Slot, err: CalyxError) -> LensHealth {
    if is_builtin_temporal_slot(slot) {
        return LensHealth::Loaded;
    }
    LensHealth::Failing {
        code: "CALYX_LENS_UNREACHABLE".to_string(),
        reason: err.message,
    }
}

fn is_builtin_temporal_slot(slot: &Slot) -> bool {
    slot.modality == Modality::Structured
        && slot.retrieval_only
        && slot.excluded_from_dedup
        && matches!(
            slot.slot_key.key(),
            "E2_recency" | "E3_periodic" | "E4_positional"
        )
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
    use crate::{
        CapabilityCard, CapabilityGateThresholds, CostMetrics, CoverageMetrics, MetricSource,
        SeparationMetrics, SpreadMetrics,
    };

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

    #[test]
    fn apply_capability_gate_uses_existing_lifecycle_states() {
        let (registry, lens_id) = registry_with_lens();
        let panel = panel_with_slot(lens_id, None);
        let slot_id = panel.slots[0].slot_id;
        let mut controller = SwapController::new(panel);

        let parked = apply_capability_gate(
            &mut controller,
            slot_id,
            &evaluation(lens_id, CapabilityGateDecision::Park),
            20,
        )
        .expect("park from gate");
        let retired = apply_capability_gate(
            &mut controller,
            slot_id,
            &evaluation(lens_id, CapabilityGateDecision::Retire),
            21,
        )
        .expect("retire from gate");

        assert_eq!(registry.health(lens_id).unwrap(), LensHealth::Loaded);
        assert_eq!(parked.state, SlotState::Parked);
        assert_eq!(retired.state, SlotState::Retired);
        assert_eq!(controller.panel().slots[0].state, SlotState::Retired);
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

    fn evaluation(lens_id: LensId, decision: CapabilityGateDecision) -> CapabilityGateEvaluation {
        CapabilityGateEvaluation {
            lens_id,
            decision,
            signal_bits: 0.08,
            signal_grounded: true,
            max_pairwise_corr: 0.1,
            thresholds: CapabilityGateThresholds::default(),
            reason: "unit gate".to_string(),
            card: CapabilityCard {
                lens_id,
                probe_count: 4,
                signal: Some(0.08),
                signal_source: MetricSource::AssayStore,
                proxy_signal: 0.08,
                differentiation: Some(0.07),
                differentiation_source: MetricSource::AssayStore,
                proxy_differentiation: 0.7,
                spread: SpreadMetrics {
                    participation_ratio: 2.0,
                    normalized_participation_ratio: 0.5,
                    stable_rank: 2.0,
                    total_variance: 1.0,
                    mean_pairwise_distance: 1.0,
                },
                separation: SeparationMetrics {
                    score: 0.5,
                    silhouette: 0.5,
                    mean_pairwise_distance: 1.0,
                    labeled_groups: 2,
                    used_labels: true,
                },
                cost: CostMetrics {
                    total_ms: 1.0,
                    ms_per_input: 1.0,
                    vram_bytes: 0,
                    ram_bytes: 0,
                    batch_ceiling: 1_000,
                },
                coverage: CoverageMetrics {
                    requested: 4,
                    measured: 4,
                    failed: 0,
                    rate: 1.0,
                },
                health: LensHealth::Loaded,
                low_spread: false,
            },
        }
    }
}
