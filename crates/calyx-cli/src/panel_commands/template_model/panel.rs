use calyx_core::{
    Asymmetry, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotResource,
    SlotShape, SlotState, content_address,
};

use super::{SavedPanelTemplate, TemplateTimeControl};

impl SavedPanelTemplate {
    pub(in crate::panel_commands) fn to_target_panel(&self, created_at: u64) -> Panel {
        let mut slots = Vec::with_capacity(self.lenses.len() + self.time_controls.len());
        for lens in &self.lenses {
            let slot_id = SlotId::new(slots.len() as u16);
            slots.push(Slot {
                slot_id,
                slot_key: SlotKey::new(slot_id, lens.slot_key.clone()),
                lens_id: lens.runtime_lens_id.unwrap_or(lens.lens_id),
                shape: lens.shape,
                modality: lens.modality,
                asymmetry: Asymmetry::None,
                quant: QuantPolicy::turboquant_default(),
                resource: SlotResource {
                    cost: lens.cost,
                    placement: lens.placement,
                },
                axis: Some(lens.slot_key.clone()),
                retrieval_only: false,
                excluded_from_dedup: false,
                bits_about: Default::default(),
                state: SlotState::Active,
                added_at_panel_version: (slots.len() + 1) as u32,
            });
        }
        // Temporal sidecars stay Active and carry no frozen registry entry.
        //
        // These are compute-on-recall slots: `retrieval_only` + `excluded_from_dedup`,
        // so they contribute to neither content identity nor dedup. Their encoders
        // (`E2RecencyLens`/`E3PeriodicLens`/`E4PositionalLens`) cannot be frozen at
        // swap time because their identity derives from an execution-time
        // `TemporalContext` (`reference_time`, `max_age_secs`, `total_position`) and
        // they consume a packed timestamp / position-and-total rather than document
        // bytes. Freezing one would content-address an execution-time constant into
        // an immutable lens id.
        //
        // Active is the correct state: `Slot::counts_toward_degraded_for` requires
        // `Active` and then exempts `retrieval_only`, which is precisely the "present
        // but benignly absent" lifecycle these slots occupy. Consumers must therefore
        // treat an unregistered compute-on-recall sidecar as `NotApplicable` rather
        // than demand a frozen registry model (#1947).
        for control in &self.time_controls {
            let slot_id = SlotId::new(slots.len() as u16);
            slots.push(Slot {
                slot_id,
                slot_key: SlotKey::new(slot_id, control.slot_key.clone()),
                lens_id: time_control_id(&self.name, control),
                shape: control.shape,
                modality: Modality::Structured,
                asymmetry: Asymmetry::None,
                quant: QuantPolicy::None,
                resource: SlotResource::default(),
                axis: Some(control.slot_key.clone()),
                retrieval_only: true,
                excluded_from_dedup: true,
                bits_about: Default::default(),
                state: SlotState::Active,
                added_at_panel_version: (slots.len() + 1) as u32,
            });
        }
        Panel {
            version: slots.len() as u32,
            slots,
            created_at,
            kernel_ref: None,
            guard_ref: None,
        }
    }
}

pub(in crate::panel_commands) fn default_time_controls() -> Vec<TemplateTimeControl> {
    vec![
        time_control("E2_recency", "temporal_recent", SlotShape::Dense(1)),
        time_control("E3_periodic", "temporal_periodic", SlotShape::Dense(2)),
        time_control("E4_positional", "temporal_positional", SlotShape::Dense(4)),
    ]
}

fn time_control(slot_key: &str, kind: &str, shape: SlotShape) -> TemplateTimeControl {
    TemplateTimeControl {
        slot_key: slot_key.to_string(),
        kind: kind.to_string(),
        shape,
        purpose: "walk_forward_backward_as_of_time_control".to_string(),
        counts_toward_a35: false,
    }
}

fn time_control_id(template: &str, control: &TemplateTimeControl) -> LensId {
    LensId::from_bytes(content_address([
        b"panel-template-time-control-v1".as_slice(),
        template.as_bytes(),
        control.slot_key.as_bytes(),
        control.kind.as_bytes(),
    ]))
}
