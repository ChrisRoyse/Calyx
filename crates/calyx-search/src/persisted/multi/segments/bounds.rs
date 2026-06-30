use std::ops::Range;

use calyx_core::{CxId, SlotId};

use super::super::{stale, unbounded_multi_sidecar};
use super::MultiSegmentRef;
use crate::error::CliResult;

pub(super) const DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;

const BINARY_HEADER_BYTES: u64 = 16 + 2 + 4 + 8 + 8 + 8;
const ROW_HEADER_BYTES: u64 = 16 + 4;
const F32_BYTES: u64 = 4;

pub(in crate::persisted::multi) fn ensure_entry_bounded(
    slot: SlotId,
    rel: &str,
    token_dim: u32,
    row_count: usize,
    token_count: usize,
) -> CliResult {
    ensure_bounded(
        slot,
        rel,
        token_dim,
        row_count,
        token_count,
        DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES,
    )
}

pub(super) fn ensure_segment_ref_bounded(
    slot: SlotId,
    token_dim: u32,
    segment: &MultiSegmentRef,
) -> CliResult {
    ensure_bounded(
        slot,
        &segment.index_rel,
        token_dim,
        segment.row_count,
        segment.token_count,
        DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES,
    )
}

pub(super) fn segment_ref_is_bounded(
    slot: SlotId,
    token_dim: u32,
    segment: &MultiSegmentRef,
) -> bool {
    ensure_segment_ref_bounded(slot, token_dim, segment).is_ok()
}

pub(super) fn split_row_ranges_by_segment_budget(
    slot: SlotId,
    token_dim: u32,
    rows: &[(CxId, Vec<Vec<f32>>)],
) -> CliResult<Vec<Range<usize>>> {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut bytes = BINARY_HEADER_BYTES;
    for (idx, (cx_id, tokens)) in rows.iter().enumerate() {
        let row_bytes = row_estimated_bytes(token_dim, tokens.len())?;
        if BINARY_HEADER_BYTES + row_bytes > DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES {
            return Err(unbounded_multi_sidecar(format!(
                "persistent multi row {cx_id} for slot {slot} is estimated {} bytes; exceeds search binary segment limit {} bytes (tokens={}, token_dim={token_dim})",
                BINARY_HEADER_BYTES + row_bytes,
                DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES,
                tokens.len()
            )));
        }
        if idx > start && bytes + row_bytes > DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES {
            ranges.push(start..idx);
            start = idx;
            bytes = BINARY_HEADER_BYTES;
        }
        bytes += row_bytes;
    }
    if start < rows.len() {
        ranges.push(start..rows.len());
    }
    Ok(ranges)
}

fn ensure_bounded(
    slot: SlotId,
    rel: &str,
    token_dim: u32,
    row_count: usize,
    token_count: usize,
    max_bytes: u64,
) -> CliResult {
    let estimated = segment_estimated_bytes(token_dim, row_count, token_count)?;
    if estimated > max_bytes {
        return Err(unbounded_multi_sidecar(format!(
            "persistent binary multi segment for slot {slot} is estimated {estimated} bytes at {rel}; exceeds search binary segment limit {max_bytes} bytes (rows={row_count}, tokens={token_count}, token_dim={token_dim})"
        )));
    }
    Ok(())
}

pub(super) fn segment_estimated_bytes(
    token_dim: u32,
    row_count: usize,
    token_count: usize,
) -> CliResult<u64> {
    let row_headers = (row_count as u64)
        .checked_mul(ROW_HEADER_BYTES)
        .ok_or_else(|| stale("persistent binary multi segment row byte count overflow"))?;
    let payload = (token_count as u64)
        .checked_mul(token_dim as u64)
        .and_then(|components| components.checked_mul(F32_BYTES))
        .ok_or_else(|| stale("persistent binary multi segment token byte count overflow"))?;
    BINARY_HEADER_BYTES
        .checked_add(row_headers)
        .and_then(|sum| sum.checked_add(payload))
        .ok_or_else(|| stale("persistent binary multi segment byte count overflow"))
}

fn row_estimated_bytes(token_dim: u32, token_count: usize) -> CliResult<u64> {
    let payload = (token_count as u64)
        .checked_mul(token_dim as u64)
        .and_then(|components| components.checked_mul(F32_BYTES))
        .ok_or_else(|| stale("persistent binary multi row token byte count overflow"))?;
    ROW_HEADER_BYTES
        .checked_add(payload)
        .ok_or_else(|| stale("persistent binary multi row byte count overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_rows_keeps_each_range_within_budget() {
        let rows = (0..3)
            .map(|idx| (CxId::from_bytes([idx; 16]), vec![vec![1.0; 2]; 2]))
            .collect::<Vec<_>>();
        let ranges = split_row_ranges_by_limit(SlotId::new(2), 2, &rows, 90).unwrap();
        assert_eq!(ranges, vec![0..1, 1..2, 2..3]);
    }

    fn split_row_ranges_by_limit(
        slot: SlotId,
        token_dim: u32,
        rows: &[(CxId, Vec<Vec<f32>>)],
        limit: u64,
    ) -> CliResult<Vec<Range<usize>>> {
        let mut ranges = Vec::new();
        let mut start = 0usize;
        let mut bytes = BINARY_HEADER_BYTES;
        for (idx, (cx_id, tokens)) in rows.iter().enumerate() {
            let row_bytes = row_estimated_bytes(token_dim, tokens.len())?;
            if BINARY_HEADER_BYTES + row_bytes > limit {
                return Err(unbounded_multi_sidecar(format!(
                    "persistent multi row {cx_id} for slot {slot} exceeds test limit"
                )));
            }
            if idx > start && bytes + row_bytes > limit {
                ranges.push(start..idx);
                start = idx;
                bytes = BINARY_HEADER_BYTES;
            }
            bytes += row_bytes;
        }
        if start < rows.len() {
            ranges.push(start..rows.len());
        }
        Ok(ranges)
    }
}
