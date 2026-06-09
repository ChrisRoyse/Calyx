//! Ward guard profile types for per-slot cosine policy enforcement.

pub mod error;
pub mod guard;
pub mod profile;
pub mod verdict;

pub use error::{
    CALYX_GUARD_MISSING_SLOT, CALYX_GUARD_OOD, CALYX_GUARD_POLICY_VIOLATION,
    CALYX_GUARD_PROVISIONAL, WardError,
};
pub use guard::{DEFAULT_TAU, MatchedSlots, ProducedSlots, guard};
pub use profile::{CalibrationMeta, GuardId, GuardPolicy, GuardProfile, NoveltyAction};
pub use verdict::{GuardVerdict, SlotVerdict};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-ward");
    }
}
