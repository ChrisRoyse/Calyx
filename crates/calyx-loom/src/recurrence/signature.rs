//! Recurrence signature facade over the Aster ingest detector.

pub use calyx_aster::dedup::{
    CALYX_RECURRENCE_SLOT_MISSING, SignatureResult, detect_recurrence_signature,
    temporal_slot_ids_for_panel,
};
