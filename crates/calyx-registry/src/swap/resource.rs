use calyx_core::{CalyxError, LensCost, LensId, Result, Slot, SlotResource};

use crate::{CALYX_REGISTRY_CONTRACT_MISSING_SPEC, Registry, runtime_default_placement};

pub const CALYX_PANEL_RESOURCE_CONTRADICTION: &str = "CALYX_PANEL_RESOURCE_CONTRADICTION";

pub(super) fn registered_slot_resource(
    registry: &Registry,
    lens_id: LensId,
) -> Result<SlotResource> {
    let spec = registry.lens_spec(lens_id).ok_or_else(|| CalyxError {
        code: CALYX_REGISTRY_CONTRACT_MISSING_SPEC,
        message: format!(
            "lens {lens_id} has no LensSpec, so hot-add cannot prove its CPU/GPU placement"
        ),
        remediation: "register the frozen lens with exact LensSpec runtime metadata before mutating the panel",
    })?;
    Ok(SlotResource {
        cost: LensCost::zero(),
        placement: runtime_default_placement(&spec.runtime),
    })
}

pub(super) fn ensure_slot_resource(slot: &Slot, expected: &SlotResource) -> Result<()> {
    if slot.resource.placement == expected.placement {
        return Ok(());
    }
    Err(CalyxError {
        code: CALYX_PANEL_RESOURCE_CONTRADICTION,
        message: format!(
            "slot {} lens {} persists placement {:?}, but its frozen runtime requires {:?}",
            slot.slot_id, slot.lens_id, slot.resource.placement, expected.placement
        ),
        remediation: "retire the contradictory slot and hot-add the frozen lens under a new panel version",
    })
}
