use std::fmt::Write;

use calyx_core::{SlotVector, SparseEntry};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::support::hex_from_bytes;
use crate::error::{CliError, CliResult};

#[derive(Serialize)]
pub(super) struct SparseEntryReport {
    pub(super) idx: u32,
    pub(super) val: f32,
}

#[derive(Serialize)]
pub(super) struct SegmentReport {
    index: usize,
    byte_start: u64,
    byte_end: u64,
    token_start: u32,
    token_end: u32,
    measured_tokens: u32,
    vector_values: usize,
    vector_sha256: String,
}

#[derive(Serialize)]
pub(super) struct SegmentationReport {
    policy_version: u16,
    policy_hash: String,
    source_hash: String,
    source_byte_len: u64,
    source_token_count: u32,
    max_input_tokens: u32,
    special_tokens: u16,
    overlap_tokens: u16,
    max_source_bytes: u64,
    max_segments: u32,
    segments: Vec<SegmentReport>,
}

pub(super) fn vector_kind(vector: &SlotVector) -> &'static str {
    match vector {
        SlotVector::Dense { .. } => "dense",
        SlotVector::Sparse { .. } => "sparse",
        SlotVector::Multi { .. } => "multi",
        SlotVector::SegmentedMulti { .. } => "segmented_multi",
        SlotVector::Absent { .. } => "absent",
    }
}

pub(super) fn sparse_entry_count(vector: &SlotVector) -> Option<usize> {
    match vector {
        SlotVector::Sparse { entries, .. } => Some(entries.len()),
        _ => None,
    }
}

pub(super) fn sparse_top(vector: &SlotVector, limit: usize) -> Option<Vec<SparseEntryReport>> {
    let SlotVector::Sparse { entries, .. } = vector else {
        return None;
    };
    let mut entries = entries.clone();
    entries.sort_by(|left, right| {
        right
            .val
            .total_cmp(&left.val)
            .then_with(|| left.idx.cmp(&right.idx))
    });
    Some(
        entries
            .into_iter()
            .take(limit)
            .map(|entry| SparseEntryReport {
                idx: entry.idx,
                val: entry.val,
            })
            .collect(),
    )
}

pub(super) fn vector_sha256(vector: &SlotVector) -> String {
    let mut hasher = Sha256::new();
    match vector {
        SlotVector::Dense { dim, data } => {
            hasher.update(b"calyx-slot-vector-dense-v1");
            update_u32(&mut hasher, *dim);
            update_f32s(&mut hasher, data);
        }
        SlotVector::Sparse { dim, entries } => {
            hasher.update(b"calyx-slot-vector-sparse-v1");
            update_u32(&mut hasher, *dim);
            update_u64(&mut hasher, entries.len() as u64);
            for SparseEntry { idx, val } in entries {
                update_u32(&mut hasher, *idx);
                update_f32(&mut hasher, *val);
            }
        }
        SlotVector::Multi { token_dim, tokens } => {
            hasher.update(b"calyx-slot-vector-multi-v1");
            update_u32(&mut hasher, *token_dim);
            update_u64(&mut hasher, tokens.len() as u64);
            update_vectors(&mut hasher, tokens.iter().map(Vec::as_slice));
        }
        SlotVector::SegmentedMulti {
            token_dim,
            provenance,
            segments,
        } => {
            hasher.update(b"calyx-slot-vector-segmented-multi-v1");
            update_u32(&mut hasher, *token_dim);
            hasher.update(provenance.policy_version.to_le_bytes());
            hasher.update(provenance.policy_hash);
            hasher.update(provenance.source_hash);
            update_u64(&mut hasher, provenance.source_byte_len);
            update_u32(&mut hasher, provenance.source_token_count);
            update_u32(&mut hasher, provenance.max_input_tokens);
            hasher.update(provenance.special_tokens.to_le_bytes());
            hasher.update(provenance.overlap_tokens.to_le_bytes());
            update_u64(&mut hasher, provenance.max_source_bytes);
            update_u32(&mut hasher, provenance.max_segments);
            update_u64(&mut hasher, segments.len() as u64);
            for segment in segments {
                update_u64(&mut hasher, segment.byte_start);
                update_u64(&mut hasher, segment.byte_end);
                update_u32(&mut hasher, segment.token_start);
                update_u32(&mut hasher, segment.token_end);
                update_u32(&mut hasher, segment.measured_tokens);
                update_f32s(&mut hasher, &segment.data);
            }
        }
        SlotVector::Absent { reason } => {
            hasher.update(b"calyx-slot-vector-absent-v1");
            hasher.update(format!("{reason:?}").as_bytes());
        }
    }
    hex_lower(&hasher.finalize())
}

pub(super) fn token_count(vector: &SlotVector) -> Option<usize> {
    match vector {
        SlotVector::Multi { tokens, .. } => Some(tokens.len()),
        SlotVector::SegmentedMulti { provenance, .. } => {
            Some(provenance.source_token_count as usize)
        }
        _ => None,
    }
}

pub(super) fn segment_count(vector: &SlotVector) -> Option<usize> {
    match vector {
        SlotVector::SegmentedMulti { segments, .. } => Some(segments.len()),
        _ => None,
    }
}

pub(super) fn segmentation_report(vector: &SlotVector) -> Option<SegmentationReport> {
    let SlotVector::SegmentedMulti {
        provenance,
        segments,
        ..
    } = vector
    else {
        return None;
    };
    Some(SegmentationReport {
        policy_version: provenance.policy_version,
        policy_hash: hex_from_bytes(&provenance.policy_hash),
        source_hash: hex_from_bytes(&provenance.source_hash),
        source_byte_len: provenance.source_byte_len,
        source_token_count: provenance.source_token_count,
        max_input_tokens: provenance.max_input_tokens,
        special_tokens: provenance.special_tokens,
        overlap_tokens: provenance.overlap_tokens,
        max_source_bytes: provenance.max_source_bytes,
        max_segments: provenance.max_segments,
        segments: segments
            .iter()
            .enumerate()
            .map(|(index, segment)| SegmentReport {
                index,
                byte_start: segment.byte_start,
                byte_end: segment.byte_end,
                token_start: segment.token_start,
                token_end: segment.token_end,
                measured_tokens: segment.measured_tokens,
                vector_values: segment.data.len(),
                vector_sha256: f32_sha256(&segment.data),
            })
            .collect(),
    })
}

pub(super) fn norm(vector: &SlotVector) -> Option<f32> {
    match vector {
        SlotVector::SegmentedMulti { .. } => None,
        _ => Some(super::support::slot_norm(vector)),
    }
}

pub(super) fn segment_norms(vector: &SlotVector) -> Option<Vec<f32>> {
    match vector {
        SlotVector::SegmentedMulti { segments, .. } => Some(
            segments
                .iter()
                .map(|segment| l2_norm(&segment.data))
                .collect(),
        ),
        _ => None,
    }
}

pub(super) fn full_vector(vector: &SlotVector, enabled: bool) -> CliResult<Option<Vec<f32>>> {
    if !enabled {
        return Ok(None);
    }
    match vector {
        SlotVector::Dense { data, .. } => Ok(Some(data.clone())),
        SlotVector::Sparse { .. } => Ok(None),
        SlotVector::Multi { .. }
        | SlotVector::SegmentedMulti { .. }
        | SlotVector::Absent { .. } => Err(CliError::usage(
            "--full-vector is supported only for dense or sparse lens explain output; ordered multi vectors are reported by typed hashes and source bounds",
        )),
    }
}

pub(super) fn full_sparse(
    vector: &SlotVector,
    enabled: bool,
) -> CliResult<Option<Vec<SparseEntryReport>>> {
    if !enabled {
        return Ok(None);
    }
    match vector {
        SlotVector::Sparse { entries, .. } => Ok(Some(sparse_entries_report(entries))),
        SlotVector::Dense { .. } => Ok(None),
        SlotVector::Multi { .. }
        | SlotVector::SegmentedMulti { .. }
        | SlotVector::Absent { .. } => Err(CliError::usage(
            "--full-vector is supported only for dense or sparse lens explain output; ordered multi vectors are reported by typed hashes and source bounds",
        )),
    }
}

fn sparse_entries_report(entries: &[SparseEntry]) -> Vec<SparseEntryReport> {
    entries
        .iter()
        .map(|entry| SparseEntryReport {
            idx: entry.idx,
            val: entry.val,
        })
        .collect()
}

fn update_vectors<'a>(hasher: &mut Sha256, vectors: impl Iterator<Item = &'a [f32]>) {
    for vector in vectors {
        update_f32s(hasher, vector);
    }
}

fn update_u32(hasher: &mut Sha256, value: u32) {
    hasher.update(value.to_le_bytes());
}

fn update_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_le_bytes());
}

fn update_f32s(hasher: &mut Sha256, values: &[f32]) {
    update_u64(hasher, values.len() as u64);
    for value in values {
        update_f32(hasher, *value);
    }
}

fn update_f32(hasher: &mut Sha256, value: f32) {
    hasher.update(value.to_bits().to_le_bytes());
}

fn f32_sha256(values: &[f32]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"calyx-segment-vector-f32-v1");
    update_f32s(&mut hasher, values);
    hex_lower(&hasher.finalize())
}

fn l2_norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut out, "{byte:02x}").expect("hex write");
    }
    out
}
