//! Ward guard profile types for per-slot cosine policy enforcement.

pub mod calibrate;
pub mod error;
pub mod guard;
pub mod profile;
pub mod query;
pub mod required;
pub mod verdict;

pub use calibrate::{
    CalibrationInput, ESTIMATOR, MIN_BAD_SCORES, SlotKind, TAU_COLD_START, calibrate,
    calibrate_slot,
};
pub use error::{
    CALYX_GUARD_MISSING_SLOT, CALYX_GUARD_OOD, CALYX_GUARD_POLICY_VIOLATION,
    CALYX_GUARD_PROVISIONAL, WardError,
};
pub use guard::{
    DEFAULT_TAU, MatchedSlots, ProducedSlots, guard, guard_non_high_stakes, guard_result,
    guard_result_with_stakes,
};
pub use profile::{CalibrationMeta, GuardId, GuardPolicy, GuardProfile, NoveltyAction};
pub use query::{
    KernelFirstQueryVerdict, QueryVerdict, RegionSource, TrustedRegion, guard_query,
    guard_query_kernel_first,
};
pub use required::{
    LOAD_BEARING_MIN_BITS, RequiredSlotDerivation, RequiredSlotEvidence, derive_required_profile,
    derive_required_slots,
};
pub use verdict::{GuardVerdict, SlotVerdict};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-ward");
    }
}
