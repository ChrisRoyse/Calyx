use std::ops::Range;

use calyx_ledger::QuarantineLookup;

pub(super) struct NoQuarantine;

impl QuarantineLookup for NoQuarantine {
    fn contains_quarantined(&self, _range: Range<u64>) -> calyx_core::Result<bool> {
        Ok(false)
    }
}
