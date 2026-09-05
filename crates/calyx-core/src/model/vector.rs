//! Slot vector representations.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{AbsentReason, Result};

use super::validation::record_schema_error;

/// Sparse vector entry.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SparseEntry {
    /// Ambient vector index.
    pub idx: u32,
    /// Entry value.
    pub val: f32,
}

/// Frozen segmentation contract and source binding for a long-input slot.
///
/// The source hash uses the same BLAKE3 bytes as [`crate::InputRef::hash`].
/// `policy_hash` content-addresses the complete runtime segmentation policy;
/// the explicit fields below make the contract independently inspectable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentationProvenance {
    pub policy_version: u16,
    pub policy_hash: [u8; 32],
    pub source_hash: [u8; 32],
    pub source_byte_len: u64,
    pub source_token_count: u32,
    pub max_input_tokens: u32,
    pub special_tokens: u16,
    pub overlap_tokens: u16,
    pub max_source_bytes: u64,
    pub max_segments: u32,
}

/// One ordered long-input segment measured by the owning frozen lens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SegmentVector {
    /// Inclusive UTF-8 byte offset in the original source.
    pub byte_start: u64,
    /// Exclusive UTF-8 byte offset in the original source.
    pub byte_end: u64,
    /// Inclusive content-token ordinal in the full-source tokenization.
    pub token_start: u32,
    /// Exclusive content-token ordinal in the full-source tokenization.
    pub token_end: u32,
    /// Tokens actually accepted by the service, including special tokens.
    pub measured_tokens: u32,
    /// The segment's independent frozen-lens measurement.
    pub data: Vec<f32>,
}

/// Per-slot vector payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotVector {
    /// Dense f32 payload.
    Dense { dim: u32, data: Vec<f32> },
    /// Sparse payload.
    Sparse { dim: u32, entries: Vec<SparseEntry> },
    /// Multi-vector token payload.
    Multi {
        token_dim: u32,
        tokens: Vec<Vec<f32>>,
    },
    /// Ordered long-input segment measurements with exact source provenance.
    ///
    /// This is deliberately distinct from `Dense`: consumers must process the
    /// block natively and may never pool or concatenate it into a substitute
    /// vector.
    SegmentedMulti {
        token_dim: u32,
        provenance: SegmentationProvenance,
        segments: Vec<SegmentVector>,
    },
    /// Explicit absence; this must never be interpreted as a zero vector.
    Absent { reason: AbsentReason },
}

impl SlotVector {
    /// Returns true when the vector is explicitly absent.
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent { .. })
    }

    /// Returns dense data only for a real dense vector.
    pub fn as_dense(&self) -> Option<&[f32]> {
        match self {
            Self::Dense { data, .. } => Some(data.as_slice()),
            Self::Sparse { .. }
            | Self::Multi { .. }
            | Self::SegmentedMulti { .. }
            | Self::Absent { .. } => None,
        }
    }

    /// Returns the element dimension for either native multi-vector shape.
    pub const fn multi_dim(&self) -> Option<u32> {
        match self {
            Self::Multi { token_dim, .. } | Self::SegmentedMulti { token_dim, .. } => {
                Some(*token_dim)
            }
            Self::Dense { .. } | Self::Sparse { .. } | Self::Absent { .. } => None,
        }
    }

    /// Returns the ordered number of vectors in either native multi shape.
    pub fn multi_len(&self) -> Option<usize> {
        match self {
            Self::Multi { tokens, .. } => Some(tokens.len()),
            Self::SegmentedMulti { segments, .. } => Some(segments.len()),
            Self::Dense { .. } | Self::Sparse { .. } | Self::Absent { .. } => None,
        }
    }

    /// Returns one ordered vector without erasing its enclosing slot shape.
    pub fn multi_vector(&self, index: usize) -> Option<&[f32]> {
        match self {
            Self::Multi { tokens, .. } => tokens.get(index).map(Vec::as_slice),
            Self::SegmentedMulti { segments, .. } => {
                segments.get(index).map(|segment| segment.data.as_slice())
            }
            Self::Dense { .. } | Self::Sparse { .. } | Self::Absent { .. } => None,
        }
    }

    /// Returns true when the payload carries a concrete, index-consumable shape.
    ///
    /// This is the single authoritative definition of "indexable" for every
    /// consumer (search roster admission, probe-matrix resident checks, and the
    /// measurement engine). It is deliberately written as an exhaustive match
    /// with no wildcard arm so that adding a `SlotVector` variant is a compile
    /// error at this predicate rather than a silent behavioural drift: a new
    /// shape must be explicitly classified as indexable or not.
    ///
    /// `SegmentedMulti` is indexable. Its segments are consumed natively as
    /// ordered late-interaction rows on both the stored and query sides; they
    /// are never pooled or flattened into a substitute vector.
    pub const fn is_indexable(&self) -> bool {
        match self {
            Self::Dense { .. }
            | Self::Sparse { .. }
            | Self::Multi { .. }
            | Self::SegmentedMulti { .. } => true,
            Self::Absent { .. } => false,
        }
    }

    /// Validates a stored vector payload against the Calyx record schema.
    pub fn validate_schema(&self) -> Result<()> {
        match self {
            Self::Dense { dim, data } => validate_dense(*dim, data),
            Self::Sparse { dim, entries } => validate_sparse(*dim, entries),
            Self::Multi { token_dim, tokens } => validate_multi(*token_dim, tokens),
            Self::SegmentedMulti {
                token_dim,
                provenance,
                segments,
            } => validate_segmented_multi(*token_dim, provenance, segments),
            Self::Absent { .. } => Ok(()),
        }
    }
}

fn validate_dense(dim: u32, data: &[f32]) -> Result<()> {
    if dim == 0 {
        return Err(record_schema_error(
            "dense slot dim must be greater than zero",
        ));
    }
    if data.len() != dim as usize {
        return Err(record_schema_error(format!(
            "dense slot dim {dim} does not match {} values",
            data.len()
        )));
    }
    ensure_finite("dense slot", data)
}

fn validate_sparse(dim: u32, entries: &[SparseEntry]) -> Result<()> {
    if dim == 0 {
        return Err(record_schema_error(
            "sparse slot dim must be greater than zero",
        ));
    }
    let mut previous = None;
    let mut strictly_increasing = true;
    for entry in entries {
        if entry.idx >= dim {
            return Err(record_schema_error(format!(
                "sparse slot index {} outside dim {dim}",
                entry.idx
            )));
        }
        if previous == Some(entry.idx) {
            return Err(record_schema_error(format!(
                "sparse slot index {} is duplicated",
                entry.idx
            )));
        }
        if previous.is_some_and(|index| index > entry.idx) {
            strictly_increasing = false;
        }
        previous = Some(entry.idx);
        if !entry.val.is_finite() {
            return Err(record_schema_error(format!(
                "sparse slot index {} is non-finite",
                entry.idx
            )));
        }
    }
    if strictly_increasing {
        return Ok(());
    }
    let mut seen = HashSet::with_capacity(entries.len());
    for entry in entries {
        if !seen.insert(entry.idx) {
            return Err(record_schema_error(format!(
                "sparse slot index {} is duplicated",
                entry.idx
            )));
        }
    }
    Ok(())
}

fn validate_multi(token_dim: u32, tokens: &[Vec<f32>]) -> Result<()> {
    if token_dim == 0 {
        return Err(record_schema_error(
            "multi-vector token_dim must be greater than zero",
        ));
    }
    if tokens.is_empty() {
        return Err(record_schema_error(
            "multi-vector payload must contain at least one token",
        ));
    }
    for (idx, token) in tokens.iter().enumerate() {
        if token.len() != token_dim as usize {
            return Err(record_schema_error(format!(
                "multi-vector token {idx} length {} does not match token_dim {token_dim}",
                token.len()
            )));
        }
        ensure_finite("multi-vector token", token)?;
    }
    Ok(())
}

fn validate_segmented_multi(
    token_dim: u32,
    provenance: &SegmentationProvenance,
    segments: &[SegmentVector],
) -> Result<()> {
    if token_dim == 0 {
        return Err(record_schema_error(
            "segmented multi-vector token_dim must be greater than zero",
        ));
    }
    if provenance.policy_version == 0
        || provenance.source_byte_len == 0
        || provenance.source_token_count == 0
        || provenance.max_input_tokens == 0
        || provenance.max_source_bytes == 0
        || provenance.max_segments == 0
        || u32::from(provenance.special_tokens) >= provenance.max_input_tokens
    {
        return Err(record_schema_error(
            "segmented multi-vector provenance has an invalid zero/boundary value",
        ));
    }
    let content_capacity = provenance.max_input_tokens - u32::from(provenance.special_tokens);
    if u32::from(provenance.overlap_tokens) >= content_capacity {
        return Err(record_schema_error(format!(
            "segmented multi-vector overlap {} must be below content capacity {content_capacity}",
            provenance.overlap_tokens
        )));
    }
    if segments.is_empty() {
        return Err(record_schema_error(
            "segmented multi-vector must contain at least one segment",
        ));
    }
    if provenance.source_byte_len > provenance.max_source_bytes
        || segments.len() > provenance.max_segments as usize
    {
        return Err(record_schema_error(
            "segmented multi-vector exceeds its frozen source/segment bound",
        ));
    }
    let mut previous: Option<&SegmentVector> = None;
    for (index, segment) in segments.iter().enumerate() {
        if segment.byte_start >= segment.byte_end
            || segment.byte_end > provenance.source_byte_len
            || segment.token_start >= segment.token_end
            || segment.token_end > provenance.source_token_count
        {
            return Err(record_schema_error(format!(
                "segmented multi-vector segment {index} has invalid source bounds"
            )));
        }
        if segment.token_end - segment.token_start > content_capacity {
            return Err(record_schema_error(format!(
                "segmented multi-vector segment {index} exceeds content capacity {content_capacity}"
            )));
        }
        let minimum_measured = segment
            .token_end
            .saturating_sub(segment.token_start)
            .min(1)
            .saturating_add(u32::from(provenance.special_tokens));
        if segment.measured_tokens < minimum_measured
            || segment.measured_tokens > provenance.max_input_tokens
        {
            return Err(record_schema_error(format!(
                "segmented multi-vector segment {index} measured token count {} is outside [{minimum_measured}, {}]",
                segment.measured_tokens, provenance.max_input_tokens
            )));
        }
        if segment.data.len() != token_dim as usize {
            return Err(record_schema_error(format!(
                "segmented multi-vector segment {index} length {} does not match token_dim {token_dim}",
                segment.data.len()
            )));
        }
        ensure_finite("segmented multi-vector segment", &segment.data)?;
        if let Some(previous) = previous {
            let expected_start = previous
                .token_end
                .saturating_sub(u32::from(provenance.overlap_tokens));
            if segment.token_start != expected_start
                || segment.token_end <= previous.token_end
                || segment.byte_start > previous.byte_end
                || segment.byte_end <= previous.byte_end
            {
                return Err(record_schema_error(format!(
                    "segmented multi-vector segment {index} breaks ordered overlap/coverage"
                )));
            }
        } else if segment.byte_start != 0 || segment.token_start != 0 {
            return Err(record_schema_error(
                "segmented multi-vector first segment must start at source byte/token zero",
            ));
        }
        previous = Some(segment);
    }
    let last = previous.expect("segments is nonempty");
    if last.byte_end != provenance.source_byte_len
        || last.token_end != provenance.source_token_count
    {
        return Err(record_schema_error(
            "segmented multi-vector final segment does not cover the complete source",
        ));
    }
    Ok(())
}

fn ensure_finite(field: &str, values: &[f32]) -> Result<()> {
    if values.iter().all(|value| value.is_finite()) {
        return Ok(());
    }
    Err(record_schema_error(format!("{field} contains NaN or Inf")))
}

#[cfg(test)]
mod is_indexable_tests {
    use super::*;

    fn segment(data: Vec<f32>) -> SegmentVector {
        SegmentVector {
            byte_start: 0,
            byte_end: 4,
            token_start: 0,
            token_end: 1,
            measured_tokens: 1,
            data,
        }
    }

    fn provenance() -> SegmentationProvenance {
        SegmentationProvenance {
            policy_version: 1,
            policy_hash: [0u8; 32],
            source_hash: [0u8; 32],
            source_byte_len: 4,
            source_token_count: 1,
            max_input_tokens: 8192,
            special_tokens: 2,
            overlap_tokens: 32,
            max_source_bytes: 1 << 20,
            max_segments: 64,
        }
    }

    /// Every concrete shape is indexable; only explicit absence is not.
    ///
    /// The match in this test is exhaustive by construction: `is_indexable`
    /// itself has no wildcard arm, so a new `SlotVector` variant fails to
    /// compile at the predicate and must be classified deliberately.
    #[test]
    fn concrete_shapes_are_indexable_and_absent_is_not() {
        let dense = SlotVector::Dense {
            dim: 1,
            data: vec![1.0],
        };
        let sparse = SlotVector::Sparse {
            dim: 4,
            entries: vec![SparseEntry { idx: 0, val: 1.0 }],
        };
        let multi = SlotVector::Multi {
            token_dim: 1,
            tokens: vec![vec![1.0]],
        };
        let segmented = SlotVector::SegmentedMulti {
            token_dim: 1,
            provenance: provenance(),
            segments: vec![segment(vec![1.0])],
        };
        let absent = SlotVector::Absent {
            reason: AbsentReason::LensInactive,
        };

        assert!(dense.is_indexable());
        assert!(sparse.is_indexable());
        assert!(multi.is_indexable());
        assert!(
            segmented.is_indexable(),
            "SegmentedMulti is consumed natively as ordered late-interaction rows (#1949)"
        );
        assert!(!absent.is_indexable());
    }

    /// A multi-segment block stays indexable and is never pooled to one vector.
    #[test]
    fn multi_segment_block_is_indexable_without_pooling() {
        let vector = SlotVector::SegmentedMulti {
            token_dim: 2,
            provenance: provenance(),
            segments: vec![segment(vec![1.0, 0.0]), segment(vec![0.0, 1.0])],
        };
        assert!(vector.is_indexable());
        assert_eq!(vector.multi_len(), Some(2));
        assert_eq!(vector.as_dense(), None);
    }
}
