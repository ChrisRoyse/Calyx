use calyx_core::{CalyxError, Input, LensId, Result, SlotVector};
use calyx_ledger::ReproduceLensRegistry;

use super::Registry;

impl ReproduceLensRegistry for Registry {
    fn frozen_weights_sha256(&self, lens_id: LensId) -> Result<[u8; 32]> {
        self.frozen_contract(lens_id)
            .map(|contract| contract.weights_sha256())
            .ok_or_else(|| {
                CalyxError::lens_frozen_violation(format!(
                    "lens {lens_id} has no frozen registry snapshot"
                ))
            })
    }

    fn measure_frozen(&self, lens_id: LensId, input: &Input) -> Result<SlotVector> {
        self.measure(lens_id, input)
    }
}
