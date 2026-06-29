use calyx_core::{CalyxError, Result};

use crate::frozen::FrozenLensContract;
use crate::spec::LensSpec;

pub(super) fn ensure_spec_declares_contract(
    contract: &FrozenLensContract,
    spec: &LensSpec,
) -> Result<()> {
    let declared = spec.declared_contract();
    if declared == *contract {
        return Ok(());
    }
    Err(CalyxError::lens_frozen_violation(format!(
        "LensSpec {} declares frozen contract {}, but registry contract is {}",
        spec.name,
        declared.lens_id(),
        contract.lens_id()
    )))
}
