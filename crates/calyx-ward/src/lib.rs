//! Ward guard profile types for per-slot cosine policy enforcement.

pub mod calibrate;
pub mod drift;
pub mod error;
pub mod guard;
pub mod novelty;
pub mod profile;
pub mod query;
pub mod required;
pub mod verdict;

pub use calibrate::{
    CalibrationInput, ESTIMATOR, MIN_BAD_SCORES, SlotKind, TAU_COLD_START, calibrate,
    calibrate_slot,
};
pub use drift::{
    AnnealHook, DEFAULT_DRIFT_CHANNEL_CAPACITY, DEFAULT_DRIFT_WINDOW, DriftEvent, DriftMonitor,
    GuardHealth, REJECTION_RATE_DRIFT_MULTIPLIER, guard_health,
};
pub use error::{
    CALYX_GUARD_ID_MISMATCH, CALYX_GUARD_MISSING_SLOT, CALYX_GUARD_NOT_A_FAILURE,
    CALYX_GUARD_NOVELTY_SINK, CALYX_GUARD_OOD, CALYX_GUARD_POLICY_VIOLATION,
    CALYX_GUARD_PROVISIONAL, WardError,
};
pub use guard::{
    DEFAULT_TAU, MatchedSlots, ProducedSlots, guard, guard_non_high_stakes, guard_result,
    guard_result_with_stakes,
};
pub use novelty::{
    NovelId, NoveltyHandler, NoveltyRecord, NoveltyStatus, VaultSink, novel_regions,
};
pub use profile::{
    CalibrationMeta, GuardId, GuardPolicy, GuardProfile, NoveltyAction, SlotCalibrationMeta,
};
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
