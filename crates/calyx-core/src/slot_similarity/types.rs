use serde::{Deserialize, Serialize};

use crate::{Result, SegmentationProvenance};

use super::CALYX_SLOT_SIMILARITY_SHAPE;
use super::{SLOT_SIMILARITY_SCHEMA_VERSION, similarity_error};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimilaritySlotShape {
    Dense,
    Sparse,
    Multi,
    SegmentedMulti,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotBlockKind {
    Dense,
    Sparse,
    MultiToken,
    Segment,
}

/// Stable source coordinate for one independently measured vector block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotBlockRef {
    pub kind: SlotBlockKind,
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_end: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_end: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirectionalBlockMatch {
    pub source: SlotBlockRef,
    pub target: SlotBlockRef,
    pub cosine: f32,
}

/// Complete typed evidence for one slot-to-slot comparison.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotSimilarityEvidence {
    pub schema_version: u16,
    pub left_shape: SimilaritySlotShape,
    pub right_shape: SimilaritySlotShape,
    pub ambient_dim: u32,
    pub left_block_count: u32,
    pub right_block_count: u32,
    pub evaluated_pair_count: u64,
    /// BLAKE3 over every row-major block-pair cosine's exact f32 bits.
    pub pairwise_digest: [u8; 32],
    pub left_blocks: Vec<SlotBlockRef>,
    pub right_blocks: Vec<SlotBlockRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_segmentation: Option<SegmentationProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_segmentation: Option<SegmentationProvenance>,
    pub left_to_right: Vec<DirectionalBlockMatch>,
    pub right_to_left: Vec<DirectionalBlockMatch>,
    /// Symmetric fail-closed score: minimum of the two directional mean-max
    /// similarities. The decompositions above remain the authoritative proof.
    pub score: f32,
}

impl SlotSimilarityEvidence {
    pub fn validate(&self) -> Result<()> {
        let left = self.left_block_count as usize;
        let right = self.right_block_count as usize;
        let expected_pairs = u64::try_from(left)
            .ok()
            .and_then(|left| {
                u64::try_from(right)
                    .ok()
                    .and_then(|right| left.checked_mul(right))
            })
            .ok_or_else(|| {
                similarity_error(
                    CALYX_SLOT_SIMILARITY_SHAPE,
                    "block-pair count overflow",
                    "reduce the bounded block count or repair the stored slot",
                )
            })?;
        if self.schema_version != SLOT_SIMILARITY_SCHEMA_VERSION
            || self.ambient_dim == 0
            || left == 0
            || right == 0
            || self.evaluated_pair_count != expected_pairs
            || self.left_blocks.len() != left
            || self.right_blocks.len() != right
            || self.left_to_right.len() != left
            || self.right_to_left.len() != right
            || !self.score.is_finite()
            || self
                .left_to_right
                .iter()
                .chain(&self.right_to_left)
                .any(|pair| !pair.cosine.is_finite())
        {
            return Err(similarity_error(
                CALYX_SLOT_SIMILARITY_SHAPE,
                "typed similarity evidence violates its schema/count contract",
                "recompute the evidence from valid stored slot vectors",
            ));
        }
        Ok(())
    }
}
