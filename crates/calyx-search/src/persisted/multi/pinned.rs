//! Verified, pinned in-memory multi-vector (MaxSim) index. Each manifest
//! generation is loaded once: whole-file reads, one-shot sha256 against the
//! manifest, full structural validation (header, duplicate rows, token
//! counts, finiteness, trailing bytes), then a flattened token matrix with
//! precomputed token norms is pinned. Candidate reranks resolve only the
//! requested rows and use canonical exact CPU MaxSim by default, avoiding the
//! multi-GiB CUDA context cost for a few dozen rows. Strict CUDA may explicitly
//! select bounded GPU chunks; CPU-only full scans stream verified segments.

#[cfg(feature = "cuda")]
use std::collections::BTreeSet;

#[cfg(feature = "cuda")]
use rayon::prelude::*;

use super::*;
#[cfg(feature = "cuda")]
use crate::persisted::pinned::{self, PinKey};

#[cfg(feature = "cuda")]
const PIN_KIND: &str = "multi_maxsim";

// The CPU MaxSim serving path streams each segment sidecar row-by-row and
// never pins the lens resident (issue #1845). The bounded per-row decode
// buffer is capped by the per-segment byte budget, so this is the largest
// single decoded row the streaming scorer can hold at once.
#[cfg(not(feature = "cuda"))]
const STREAMING_ROW_WORKING_SET_BYTES: u64 = 64 * 1024 * 1024;

#[path = "pinned/bounded.rs"]
mod bounded;
pub(super) use bounded::{
    BoundedSegmentFile, memoize_bounded_segment_files, memoized_bounded_segment_files,
    stat_check_segment_files,
};
#[path = "pinned/cache.rs"]
mod cache;
#[cfg(feature = "cuda")]
pub(super) use cache::PinnedIndexAccess;
pub(super) use cache::observe_generation;
#[cfg(feature = "cuda")]
pub(super) use cache::pinned_index;
#[path = "pinned/candidates.rs"]
#[cfg(feature = "cuda")]
mod candidates;
#[cfg(feature = "cuda")]
pub(super) use candidates::{PinnedCandidateRow, PinnedCandidateSelection};

/// One verified binary segment to load: path plus the manifest-side
/// expectations the file must match. Only the resident CUDA candidate-reuse
/// path materializes segments; the CPU path streams them (issue #1845).
#[cfg(feature = "cuda")]
pub(super) struct PinnedSegmentSpec {
    pub(super) path: PathBuf,
    pub(super) index_rel: String,
    pub(super) sha256: String,
    pub(super) base_seq: u64,
    pub(super) row_count: u64,
    pub(super) token_count: u64,
    pub(super) byte_len: u64,
    pub(super) expected_ids: Vec<CxId>,
}

#[cfg(feature = "cuda")]
#[derive(Debug)]
pub(super) struct PinnedMultiIndex {
    pub(in crate::persisted::multi) token_dim: u32,
    rows: Vec<PinnedMultiRow>,
    #[cfg(feature = "cuda")]
    row_lookup: Vec<(CxId, usize)>,
    tokens: Vec<f32>,
    norms: Vec<f32>,
}

#[cfg(feature = "cuda")]
#[derive(Debug)]
struct PinnedMultiRow {
    cx_id: CxId,
    token_start: usize,
    token_count: usize,
}

/// Fail-closed serving memory admission for a segmented multi lens.
///
/// A CUDA build can select the generation-resident candidate rerank path, which
/// retains every decoded token, norm, row, and lookup entry. Admit that exact
/// worst-case footprint before any file is read or allocation is attempted.
/// CPU-only builds stream rows and retain only their bounded working set.
pub(super) fn predicted_pin_bytes(entry: &SearchIndexEntry, token_dim: u32) -> CliResult<u64> {
    #[cfg(feature = "cuda")]
    {
        let token_count = entry.token_count.ok_or_else(|| {
            stale(format!(
                "persistent segmented multi slot {} is missing token_count for CUDA pin admission",
                entry.slot
            ))
        })? as u64;
        let token_components = token_count
            .checked_mul(u64::from(token_dim))
            .ok_or_else(|| stale("persistent segmented multi token component count overflow"))?;
        let token_bytes = token_components
            .checked_mul(std::mem::size_of::<f32>() as u64)
            .ok_or_else(|| stale("persistent segmented multi token byte count overflow"))?;
        let norm_bytes = token_count
            .checked_mul(std::mem::size_of::<f32>() as u64)
            .ok_or_else(|| stale("persistent segmented multi norm byte count overflow"))?;
        let row_bytes = (entry.len as u64)
            .checked_mul(std::mem::size_of::<PinnedMultiRow>() as u64)
            .ok_or_else(|| stale("persistent segmented multi row byte count overflow"))?;
        let lookup_bytes = (entry.len as u64)
            .checked_mul(std::mem::size_of::<(CxId, usize)>() as u64)
            .ok_or_else(|| stale("persistent segmented multi lookup byte count overflow"))?;
        token_bytes
            .checked_add(norm_bytes)
            .and_then(|bytes| bytes.checked_add(row_bytes))
            .and_then(|bytes| bytes.checked_add(lookup_bytes))
            .ok_or_else(|| stale("persistent segmented multi CUDA pin byte count overflow"))
    }

    #[cfg(not(feature = "cuda"))]
    let _ = token_dim;
    #[cfg(not(feature = "cuda"))]
    let scored = (entry.len as u64)
        .checked_mul(std::mem::size_of::<(CxId, f32)>() as u64)
        .ok_or_else(|| stale("persistent segmented multi streaming scored byte count overflow"))?;
    #[cfg(not(feature = "cuda"))]
    scored
        .checked_add(STREAMING_ROW_WORKING_SET_BYTES)
        .ok_or_else(|| {
            stale("persistent segmented multi streaming working-set byte count overflow")
        })
}

#[cfg(feature = "cuda")]
fn load_verified(
    slot: SlotId,
    token_dim: u32,
    entry: &SearchIndexEntry,
    specs: &[PinnedSegmentSpec],
    pin_key: &PinKey,
    predicted_bytes: u64,
) -> CliResult<PinnedMultiIndex> {
    let segments = specs
        .par_iter()
        .map(|spec| load_segment(slot, token_dim, spec))
        .collect::<Vec<_>>();
    let expected_tokens = entry.token_count.unwrap_or_default();
    let expected_values = expected_tokens
        .checked_mul(token_dim as usize)
        .ok_or_else(|| stale("persistent segmented multi sidecar token byte count overflow"))?;
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    rows.try_reserve_exact(entry.len)
        .map_err(|_| pinned::pin_allocation_error(pin_key, predicted_bytes))?;
    let mut tokens: Vec<f32> = Vec::new();
    tokens
        .try_reserve_exact(expected_values)
        .map_err(|_| pinned::pin_allocation_error(pin_key, predicted_bytes))?;
    let mut norms: Vec<f32> = Vec::new();
    norms
        .try_reserve_exact(expected_tokens)
        .map_err(|_| pinned::pin_allocation_error(pin_key, predicted_bytes))?;
    let mut token_offset = 0usize;
    for segment in segments {
        let segment = segment?;
        for (cx_id, token_count) in segment.rows {
            if !seen.insert(cx_id) {
                return Err(stale(format!(
                    "persistent segmented multi sidecars repeat {cx_id}; rebuild the vault search indexes"
                )));
            }
            rows.push(PinnedMultiRow {
                cx_id,
                token_start: token_offset,
                token_count,
            });
            token_offset = token_offset
                .checked_add(token_count)
                .ok_or_else(|| stale("persistent segmented multi sidecar token_count overflow"))?;
        }
        tokens.extend_from_slice(&segment.tokens);
        norms.extend_from_slice(&segment.norms);
        if token_offset != norms.len() || tokens.len() != norms.len() * token_dim as usize {
            return Err(stale(format!(
                "persistent segmented multi sidecar {} token layout is inconsistent; rebuild the vault search indexes",
                entry.require_index_rel(slot)?
            )));
        }
    }
    if rows.len() != entry.len {
        return Err(stale(format!(
            "persistent segmented multi manifest row_count {} != scanned row count {}; rebuild the vault search indexes",
            entry.len,
            rows.len()
        )));
    }
    if norms.len() != expected_tokens {
        return Err(stale(format!(
            "persistent segmented multi manifest token_count {expected_tokens} != scanned token count {}; rebuild the vault search indexes",
            norms.len()
        )));
    }
    #[cfg(feature = "cuda")]
    let mut row_lookup = rows
        .iter()
        .enumerate()
        .map(|(index, row)| (row.cx_id, index))
        .collect::<Vec<_>>();
    #[cfg(feature = "cuda")]
    row_lookup.sort_unstable_by_key(|(cx_id, _)| *cx_id);
    Ok(PinnedMultiIndex {
        token_dim,
        rows,
        #[cfg(feature = "cuda")]
        row_lookup,
        tokens,
        norms,
    })
}

#[cfg(feature = "cuda")]
struct LoadedSegment {
    rows: Vec<(CxId, usize)>,
    tokens: Vec<f32>,
    norms: Vec<f32>,
}

#[cfg(feature = "cuda")]
fn load_segment(
    slot: SlotId,
    token_dim: u32,
    spec: &PinnedSegmentSpec,
) -> CliResult<LoadedSegment> {
    let bytes = fs::read(&spec.path)?;
    if bytes.len() < 16 || &bytes[..16] != super::binary::MULTI_BINARY_MAGIC {
        return Err(stale(format!(
            "persistent binary multi sidecar {} has invalid magic; rebuild the vault search indexes",
            spec.index_rel
        )));
    }
    let actual = sha256_hex(&bytes);
    if actual != spec.sha256 {
        return Err(stale(format!(
            "persistent binary multi sidecar sha256 {actual} != manifest {}; rebuild the vault search indexes",
            spec.sha256
        )));
    }
    let loaded = parse_segment(slot, token_dim, spec, &bytes)?;
    let mut actual_ids = loaded
        .rows
        .iter()
        .map(|(cx_id, _)| *cx_id)
        .collect::<Vec<_>>();
    actual_ids.sort_unstable();
    let mut expected_ids = spec.expected_ids.clone();
    expected_ids.sort_unstable();
    if actual_ids != expected_ids {
        return Err(stale(format!(
            "persistent segmented multi sidecar {} IDs do not match its manifest; rebuild the vault search indexes",
            spec.index_rel
        )));
    }
    Ok(loaded)
}

#[cfg(feature = "cuda")]
fn parse_segment(
    slot: SlotId,
    token_dim: u32,
    spec: &PinnedSegmentSpec,
    bytes: &[u8],
) -> CliResult<LoadedSegment> {
    let mut cursor = SegmentCursor::new(&spec.index_rel, bytes);
    cursor.expect_magic()?;
    let header_slot = cursor.read_u16()?;
    let header_token_dim = cursor.read_u32()?;
    let header_base_seq = cursor.read_u64()?;
    let header_row_count = cursor.read_u64()?;
    let header_token_count = cursor.read_u64()?;
    if header_slot != slot.get() {
        return Err(stale(format!(
            "persistent binary multi sidecar slot {header_slot} != expected slot {}; rebuild the vault search indexes",
            slot.get()
        )));
    }
    if header_token_dim != token_dim {
        return Err(stale(format!(
            "persistent binary multi sidecar token_dim {header_token_dim} != expected token_dim {token_dim}; rebuild the vault search indexes"
        )));
    }
    if header_base_seq != spec.base_seq {
        return Err(stale(format!(
            "persistent segmented multi sidecar {} seq {header_base_seq} != segment manifest seq {}; rebuild the vault search indexes",
            spec.index_rel, spec.base_seq
        )));
    }
    if header_row_count != spec.row_count {
        return Err(stale(format!(
            "persistent binary multi sidecar row len {header_row_count} != expected {}; rebuild the vault search indexes",
            spec.row_count
        )));
    }
    if header_token_count != spec.token_count {
        return Err(stale(format!(
            "persistent binary multi sidecar token_count {header_token_count} != expected {}; rebuild the vault search indexes",
            spec.token_count
        )));
    }
    let dim = token_dim as usize;
    let segment_values = (header_token_count as usize)
        .checked_mul(dim)
        .ok_or_else(|| stale("persistent binary multi sidecar token byte count overflow"))?;
    let mut rows = Vec::with_capacity(header_row_count as usize);
    let mut seen = BTreeSet::new();
    let mut tokens: Vec<f32> = Vec::with_capacity(segment_values);
    let mut norms: Vec<f32> = Vec::with_capacity(header_token_count as usize);
    let mut observed_tokens = 0u64;
    for _ in 0..header_row_count {
        let cx_id = cursor.read_cx_id()?;
        if !seen.insert(cx_id) {
            return Err(stale(format!(
                "persistent binary multi segmented sidecar repeats {cx_id}; rebuild the vault search indexes"
            )));
        }
        let row_token_count = cursor.read_u32()? as u64;
        observed_tokens = observed_tokens
            .checked_add(row_token_count)
            .ok_or_else(|| stale("persistent binary multi sidecar token_count overflow"))?;
        if observed_tokens > header_token_count {
            return Err(stale(format!(
                "persistent binary multi sidecar token_count exceeds header {header_token_count}; rebuild the vault search indexes"
            )));
        }
        let row_tokens = row_token_count as usize;
        let payload_bytes = cursor.take(
            row_tokens
                .checked_mul(dim)
                .and_then(|values| values.checked_mul(4))
                .ok_or_else(|| {
                    stale("persistent binary multi sidecar token byte count overflow")
                })?,
        )?;
        // Single pass: decode, finite-check, accumulate the norm, and append
        // to the flat matrix without an intermediate per-row allocation.
        for token_bytes in payload_bytes.chunks_exact(dim * 4) {
            let mut squared = 0.0f32;
            for value_bytes in token_bytes.chunks_exact(4) {
                let value = f32::from_le_bytes(value_bytes.try_into().expect("len 4"));
                if !value.is_finite() {
                    return Err(CalyxError::lens_numerical_invariant(format!(
                        "persistent binary multi row {cx_id} slot {slot} has non-finite component"
                    ))
                    .into());
                }
                squared += value * value;
                tokens.push(value);
            }
            norms.push(squared.sqrt());
        }
        rows.push((cx_id, row_tokens));
    }
    if observed_tokens != header_token_count {
        return Err(stale(format!(
            "persistent binary multi sidecar token_count {observed_tokens} != header {header_token_count}; rebuild the vault search indexes"
        )));
    }
    cursor.expect_end()?;
    Ok(LoadedSegment {
        rows,
        tokens,
        norms,
    })
}

#[cfg(feature = "cuda")]
impl PinnedMultiIndex {
    fn approx_bytes(&self) -> u64 {
        let bytes = (self.tokens.len() as u64 + self.norms.len() as u64) * 4
            + self.rows.len() as u64 * std::mem::size_of::<PinnedMultiRow>() as u64;
        #[cfg(feature = "cuda")]
        let bytes =
            bytes + self.row_lookup.len() as u64 * std::mem::size_of::<(CxId, usize)>() as u64;
        bytes
    }
}

#[cfg(feature = "cuda")]
struct SegmentCursor<'a> {
    index_rel: &'a str,
    bytes: &'a [u8],
    offset: usize,
}

#[cfg(feature = "cuda")]
impl<'a> SegmentCursor<'a> {
    fn new(index_rel: &'a str, bytes: &'a [u8]) -> Self {
        Self {
            index_rel,
            bytes,
            offset: 0,
        }
    }

    fn take(&mut self, len: usize) -> CliResult<&'a [u8]> {
        let end = self.offset.checked_add(len).ok_or_else(|| {
            stale(format!(
                "persistent binary multi sidecar {} offset overflow; rebuild the vault search indexes",
                self.index_rel
            ))
        })?;
        if end > self.bytes.len() {
            return Err(stale(format!(
                "persistent binary multi sidecar {} is truncated; rebuild the vault search indexes",
                self.index_rel
            )));
        }
        let slice = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    fn expect_magic(&mut self) -> CliResult {
        if self.take(16)? != super::binary::MULTI_BINARY_MAGIC {
            return Err(stale(format!(
                "persistent binary multi sidecar {} has invalid magic; rebuild the vault search indexes",
                self.index_rel
            )));
        }
        Ok(())
    }

    fn read_u16(&mut self) -> CliResult<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("len 2")))
    }

    fn read_u32(&mut self) -> CliResult<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("len 4")))
    }

    fn read_u64(&mut self) -> CliResult<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("len 8")))
    }

    fn read_cx_id(&mut self) -> CliResult<CxId> {
        Ok(CxId::from_bytes(self.take(16)?.try_into().expect("len 16")))
    }

    fn expect_end(&mut self) -> CliResult {
        if self.offset != self.bytes.len() {
            return Err(stale(format!(
                "persistent binary multi sidecar {} has trailing bytes; rebuild the vault search indexes",
                self.index_rel
            )));
        }
        Ok(())
    }
}
