//! Lens proposal primitives for Anneal.

pub mod deficit_localize;

pub use deficit_localize::{
    AnchorGap, AnchorId, AssayAttribution, CALYX_ANNEAL_DEFICIT_INVALID_CONFIG,
    CALYX_ASSAY_INVALID_METRIC, CALYX_ASSAY_UNAVAILABLE, DEFAULT_DEFICIT_THRESHOLD_BITS,
    DeficitLocalizer, DeficitLocalizerConfig, DeficitMap, MODALITY_COVERAGE_THRESHOLD_BITS,
    ModalityId, has_deficit, top_gap_description,
};
