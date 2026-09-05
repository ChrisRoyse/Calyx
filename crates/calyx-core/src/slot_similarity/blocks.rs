use crate::{Result, SegmentationProvenance, SlotVector, SparseEntry};

use super::{
    CALYX_SLOT_SIMILARITY_ABSENT, SimilaritySlotShape, SlotBlockKind, SlotBlockRef,
    block_count_error, similarity_error,
};

#[derive(Clone, Copy)]
pub enum SlotBlockValues<'a> {
    Dense(&'a [f32]),
    Sparse(&'a [SparseEntry]),
}

pub struct SlotBlockView<'a> {
    pub reference: SlotBlockRef,
    pub values: SlotBlockValues<'a>,
}

pub fn slot_blocks(vector: &SlotVector) -> Result<Vec<SlotBlockView<'_>>> {
    let simple = |kind, values| {
        vec![SlotBlockView {
            reference: SlotBlockRef {
                kind,
                index: 0,
                byte_start: None,
                byte_end: None,
                token_start: None,
                token_end: None,
            },
            values,
        }]
    };
    match vector {
        SlotVector::Dense { data, .. } => {
            Ok(simple(SlotBlockKind::Dense, SlotBlockValues::Dense(data)))
        }
        SlotVector::Sparse { entries, .. } => Ok(simple(
            SlotBlockKind::Sparse,
            SlotBlockValues::Sparse(entries),
        )),
        SlotVector::Multi { tokens, .. } => tokens
            .iter()
            .enumerate()
            .map(|(index, token)| {
                Ok(SlotBlockView {
                    reference: SlotBlockRef {
                        kind: SlotBlockKind::MultiToken,
                        index: u32::try_from(index).map_err(|_| block_count_error())?,
                        byte_start: None,
                        byte_end: None,
                        token_start: None,
                        token_end: None,
                    },
                    values: SlotBlockValues::Dense(token),
                })
            })
            .collect(),
        SlotVector::SegmentedMulti { segments, .. } => segments
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                Ok(SlotBlockView {
                    reference: SlotBlockRef {
                        kind: SlotBlockKind::Segment,
                        index: u32::try_from(index).map_err(|_| block_count_error())?,
                        byte_start: Some(segment.byte_start),
                        byte_end: Some(segment.byte_end),
                        token_start: Some(segment.token_start),
                        token_end: Some(segment.token_end),
                    },
                    values: SlotBlockValues::Dense(&segment.data),
                })
            })
            .collect(),
        SlotVector::Absent { reason } => Err(similarity_error(
            CALYX_SLOT_SIMILARITY_ABSENT,
            format!("slot is explicitly absent: {reason:?}"),
            "supply the required measured slot; absence is never a zero vector",
        )),
    }
}

pub fn slot_ambient_dim(vector: &SlotVector) -> Result<u32> {
    match vector {
        SlotVector::Dense { dim, .. } | SlotVector::Sparse { dim, .. } => Ok(*dim),
        SlotVector::Multi { token_dim, .. } | SlotVector::SegmentedMulti { token_dim, .. } => {
            Ok(*token_dim)
        }
        SlotVector::Absent { .. } => Err(similarity_error(
            CALYX_SLOT_SIMILARITY_ABSENT,
            "absent slot has no ambient dimension",
            "supply the required measured slot",
        )),
    }
}

pub fn slot_similarity_shape(vector: &SlotVector) -> Result<SimilaritySlotShape> {
    match vector {
        SlotVector::Dense { .. } => Ok(SimilaritySlotShape::Dense),
        SlotVector::Sparse { .. } => Ok(SimilaritySlotShape::Sparse),
        SlotVector::Multi { .. } => Ok(SimilaritySlotShape::Multi),
        SlotVector::SegmentedMulti { .. } => Ok(SimilaritySlotShape::SegmentedMulti),
        SlotVector::Absent { .. } => Err(similarity_error(
            CALYX_SLOT_SIMILARITY_ABSENT,
            "absent slot has no comparable shape",
            "supply the required measured slot",
        )),
    }
}

pub fn slot_segmentation(vector: &SlotVector) -> Option<SegmentationProvenance> {
    match vector {
        SlotVector::SegmentedMulti { provenance, .. } => Some(provenance.clone()),
        _ => None,
    }
}
