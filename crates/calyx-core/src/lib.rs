//! Core Calyx identifiers, model contracts, and shared types.

pub mod cosine;
pub mod enums;
pub mod error;
pub mod ids;
pub mod model;
pub mod temporal;
pub mod time;
pub mod traits;

pub use cosine::{GuardTauProfile, dense_cosine};
pub use enums::{AbsentReason, AnchorKind, Asymmetry, Modality, QuantPolicy, SlotShape, SlotState};
pub use error::{CALYX_ERROR_CODES, CalyxError, CalyxErrorCode, CalyxWarning, Result};
pub use ids::{CxId, LensId, ParseIdError, SlotId, SlotKey, VaultId, content_address};
pub use model::{
    Anchor, AnchorValue, CALYX_RECORD_SCHEMA_VIOLATION, ConfidenceInterval, Constellation, CxFlags,
    InputRef, LedgerRef, METADATA_CHUNK_ID, METADATA_DATABASE_NAME, Panel, Signal, Slot,
    SlotVector, SparseEntry,
};
pub use temporal::{
    BoostConfig, CALYX_TEMPORAL_AP60_VIOLATION, CALYX_TEMPORAL_INVALID_BOOST_CONFIG,
    CALYX_TEMPORAL_INVALID_PERIOD, CALYX_TEMPORAL_INVALID_WINDOW, CALYX_TEMPORAL_NEGATIVE_WEIGHT,
    CALYX_TEMPORAL_WEIGHT_SUM, DecayFunction, FusionWeights, MultiAnchorMode, PeriodicOptions,
    RecurrenceBoostConfig, SequenceDirection, SequenceOptions, TemporalPolicy,
};
pub use time::{Clock, FixedClock, Seq, SystemClock, Ts};
pub use traits::{Estimator, Index, Input, Lens, VaultStore};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-core");
    }
}
