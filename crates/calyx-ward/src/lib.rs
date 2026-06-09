//! Ward guard profile types for per-slot cosine policy enforcement.

pub mod calibrate;
pub mod drift;
pub mod error;
pub mod guard;
pub mod identity;
pub mod ledger;
pub mod novelty;
pub mod profile;
pub mod query;
pub mod required;
pub mod speaker_lens;
pub mod style_lens;
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
    CALYX_GUARD_ID_MISMATCH, CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED, CALYX_GUARD_MISSING_SLOT,
    CALYX_GUARD_NOT_A_FAILURE, CALYX_GUARD_NOVELTY_SINK, CALYX_GUARD_OOD,
    CALYX_GUARD_POLICY_VIOLATION, CALYX_GUARD_PROVISIONAL, CALYX_WARD_INVALID_INPUT,
    CALYX_WARD_MODEL_DIM_MISMATCH, CALYX_WARD_MODEL_NOT_FOUND, CALYX_WARD_RUNTIME_ERROR, WardError,
};
pub use guard::{
    DEFAULT_TAU, MatchedSlots, ProducedSlots, guard, guard_non_high_stakes, guard_result,
    guard_result_with_stakes,
};
pub use identity::{IdentityProfile, IdentitySlotConfig};
pub use ledger::{
    WardLedgerError, WardLedgerResult, append_calibration_provenance, append_guard_verdict,
    calibrate_with_ledger, guard_with_ledger,
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
pub use speaker_lens::{
    DEFAULT_WAVLM_MODEL_PATH, SpeakerEmbeddingBackend, SpeakerLens, SpeakerProviderPolicy,
    WAVLM_DIM, WAVLM_SAMPLE_RATE,
};
pub use style_lens::{
    DEFAULT_STYLE_MODEL_PATH, DEFAULT_STYLE_TOKENIZER_PATH, STYLE_DIM, STYLE_MAX_TOKENS,
    StyleEmbeddingBackend, StyleLens, StyleProviderPolicy,
};
pub use verdict::{GuardVerdict, SlotVerdict};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-ward");
    }
}
