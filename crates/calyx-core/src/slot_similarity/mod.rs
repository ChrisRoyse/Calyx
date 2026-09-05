//! Native similarity evidence for typed slot vectors.
//!
//! A3 forbids replacing `Multi` or `SegmentedMulti` with a pooled surrogate.
//! This module exposes the original typed blocks and compares them while
//! retaining every block-to-block cosine and both directional decompositions.

mod blocks;
mod math;
mod types;

pub use blocks::{
    SlotBlockValues, SlotBlockView, slot_ambient_dim, slot_blocks, slot_segmentation,
    slot_similarity_shape,
};
pub use math::{
    PreparedSlotSimilarity, prepare_slot_similarity, prepared_slot_similarity_score,
    slot_similarity,
};
pub use types::{
    DirectionalBlockMatch, SimilaritySlotShape, SlotBlockKind, SlotBlockRef, SlotSimilarityEvidence,
};

pub const CALYX_SLOT_SIMILARITY_SHAPE: &str = "CALYX_SLOT_SIMILARITY_SHAPE";
pub const CALYX_SLOT_SIMILARITY_ABSENT: &str = "CALYX_SLOT_SIMILARITY_ABSENT";
pub const CALYX_SLOT_SIMILARITY_NUMERICAL: &str = "CALYX_SLOT_SIMILARITY_NUMERICAL";
pub const SLOT_SIMILARITY_SCHEMA_VERSION: u16 = 1;

fn block_count_error() -> crate::CalyxError {
    similarity_error(
        CALYX_SLOT_SIMILARITY_SHAPE,
        "slot block count exceeds u32",
        "reduce the bounded block count or repair the stored slot",
    )
}

fn similarity_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> crate::CalyxError {
    crate::CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
