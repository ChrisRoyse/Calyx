//! Verified, pinned in-memory multi-vector (MaxSim) index. Each manifest
//! generation is loaded once: whole-file reads, one-shot sha256 against the
//! manifest, full structural validation (header, duplicate rows, token
//! counts, finiteness, trailing bytes), then a flattened token matrix with
//! precomputed token norms is pinned. Queries score with the exact
//! arithmetic of `MaxSimIndex::maxsim` / `cosine`, so results are bit-identical.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use rayon::prelude::*;

use super::*;
use crate::persisted::pinned::{self, PinKey};

const PIN_KIND: &str = "multi_maxsim";

/// One verified binary segment to load: path plus the manifest-side
/// expectations the file must match.
pub(super) struct PinnedSegmentSpec {
    pub(super) path: PathBuf,
    pub(super) index_rel: String,
    pub(super) sha256: String,
    pub(super) base_seq: u64,
    pub(super) row_count: u64,
    pub(super) token_count: u64,
}

#[derive(Debug)]
pub(super) struct PinnedMultiIndex {
    token_dim: u32,
    rows: Vec<PinnedMultiRow>,
    tokens: Vec<f32>,
    norms: Vec<f32>,
}

#[derive(Debug)]
struct PinnedMultiRow {
    cx_id: CxId,
    token_start: usize,
    token_count: usize,
}

struct PinnedGeneration {
    entry_sha256: String,
    index: Arc<PinnedMultiIndex>,
}

type PinCache = Mutex<BTreeMap<(String, u16), PinnedGeneration>>;

fn cache() -> &'static PinCache {
    static CACHE: OnceLock<PinCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Return the pinned index for `entry`, loading and verifying it on first
/// use per manifest generation. The cache key is the manifest entry sha256:
/// any rebuild produces a new sha and forces a fresh verified load.
pub(super) fn pinned_index(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    specs: Vec<PinnedSegmentSpec>,
) -> CliResult<Arc<PinnedMultiIndex>> {
    let entry_sha256 = entry.require_sha256(slot)?.to_string();
    let cache_key = (pinned::canonical_vault_dir(vault_dir)?, slot.get());
    {
        let cache = cache().lock().expect("multi pin cache poisoned");
        if let Some(generation) = cache.get(&cache_key)
            && generation.entry_sha256 == entry_sha256
        {
            return Ok(Arc::clone(&generation.index));
        }
    }
    let token_dim = entry.require_token_dim(slot)?;
    let index = Arc::new(load_verified(slot, token_dim, entry, &specs)?);
    let pin_key = PinKey::new(vault_dir, slot.get(), PIN_KIND)?;
    pinned::reserve(&pin_key, index.approx_bytes())?;
    let mut cache = cache().lock().expect("multi pin cache poisoned");
    cache.insert(
        cache_key,
        PinnedGeneration {
            entry_sha256,
            index: Arc::clone(&index),
        },
    );
    Ok(index)
}

fn load_verified(
    slot: SlotId,
    token_dim: u32,
    entry: &SearchIndexEntry,
    specs: &[PinnedSegmentSpec],
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
    let mut rows = Vec::with_capacity(entry.len);
    let mut tokens = Vec::with_capacity(expected_values);
    let mut norms = Vec::with_capacity(expected_tokens);
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
    Ok(PinnedMultiIndex {
        token_dim,
        rows,
        tokens,
        norms,
    })
}

struct LoadedSegment {
    rows: Vec<(CxId, usize)>,
    tokens: Vec<f32>,
    norms: Vec<f32>,
}

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
    parse_segment(slot, token_dim, spec, &bytes)
}

/// Memo of segment files that already passed the full bounded validation for
/// a manifest generation. A memo hit never skips on-disk presence/size
/// verification — callers must still run `stat_check_segment_files` — it only
/// skips re-reading and re-hashing the segments manifest JSON.
pub(super) struct BoundedSegmentFile {
    pub(super) path: PathBuf,
    pub(super) index_rel: String,
    pub(super) expected_bytes: u64,
}

struct BoundedGeneration {
    entry_sha256: String,
    files: Arc<Vec<BoundedSegmentFile>>,
}

type BoundedCache = Mutex<BTreeMap<(String, u16), BoundedGeneration>>;

fn bounded_cache() -> &'static BoundedCache {
    static CACHE: OnceLock<BoundedCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn memoized_bounded_segment_files(
    vault_dir: &Path,
    slot: SlotId,
    entry_sha256: &str,
) -> CliResult<Option<Arc<Vec<BoundedSegmentFile>>>> {
    let key = (pinned::canonical_vault_dir(vault_dir)?, slot.get());
    let cache = bounded_cache()
        .lock()
        .expect("bounded segment memo poisoned");
    Ok(cache.get(&key).and_then(|generation| {
        (generation.entry_sha256 == entry_sha256).then(|| Arc::clone(&generation.files))
    }))
}

pub(super) fn memoize_bounded_segment_files(
    vault_dir: &Path,
    slot: SlotId,
    entry_sha256: &str,
    files: Vec<BoundedSegmentFile>,
) -> CliResult {
    let key = (pinned::canonical_vault_dir(vault_dir)?, slot.get());
    let mut cache = bounded_cache()
        .lock()
        .expect("bounded segment memo poisoned");
    cache.insert(
        key,
        BoundedGeneration {
            entry_sha256: entry_sha256.to_string(),
            files: Arc::new(files),
        },
    );
    Ok(())
}

/// Per-call on-disk verification for memoized bounded checks: every segment
/// file must still exist with exactly its validated byte size. Deletion or
/// truncation fails closed with the same errors as the full validation.
pub(super) fn stat_check_segment_files(slot: SlotId, files: &[BoundedSegmentFile]) -> CliResult {
    for file in files {
        if !file.path.is_file() {
            return Err(stale(format!(
                "persistent segmented multi sidecar missing for slot {slot} at {}; rebuild the vault search indexes",
                file.path.display()
            )));
        }
        let actual = fs::metadata(&file.path)?.len();
        if actual != file.expected_bytes {
            return Err(stale(format!(
                "persistent segmented multi sidecar {} has {actual} bytes, expected {}; rebuild the vault search indexes",
                file.index_rel, file.expected_bytes
            )));
        }
    }
    Ok(())
}

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

impl PinnedMultiIndex {
    fn approx_bytes(&self) -> u64 {
        (self.tokens.len() as u64 + self.norms.len() as u64) * 4
            + self.rows.len() as u64 * std::mem::size_of::<PinnedMultiRow>() as u64
    }

    pub(super) fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// MaxSim over the pinned matrix; arithmetic mirrors
    /// `MaxSimIndex::maxsim` + `cosine` exactly (same accumulation order and
    /// zero-norm semantics), parallelized across rows.
    pub(super) fn score(
        &self,
        query: &[Vec<f32>],
        candidates: Option<&BTreeSet<CxId>>,
    ) -> Vec<(CxId, f32)> {
        let dim = self.token_dim as usize;
        let query_norms = query
            .iter()
            .map(|token| {
                let mut squared = 0.0f32;
                for value in token {
                    squared += value * value;
                }
                squared.sqrt()
            })
            .collect::<Vec<_>>();
        self.rows
            .par_iter()
            .filter(|row| candidates.is_none_or(|allowed| allowed.contains(&row.cx_id)))
            .map(|row| {
                let tokens =
                    &self.tokens[row.token_start * dim..(row.token_start + row.token_count) * dim];
                let norms = &self.norms[row.token_start..row.token_start + row.token_count];
                let score = query
                    .iter()
                    .zip(&query_norms)
                    .map(|(q, q_norm)| {
                        let mut best = f32::NEG_INFINITY;
                        for (token, t_norm) in tokens.chunks_exact(dim).zip(norms) {
                            let mut dot = 0.0f32;
                            for (left, right) in q.iter().zip(token) {
                                dot += left * right;
                            }
                            let cosine = if *q_norm == 0.0 || *t_norm == 0.0 {
                                0.0
                            } else {
                                dot / (q_norm * t_norm)
                            };
                            best = best.max(cosine);
                        }
                        best
                    })
                    .sum::<f32>();
                (row.cx_id, score)
            })
            .collect()
    }
}

struct SegmentCursor<'a> {
    index_rel: &'a str,
    bytes: &'a [u8],
    offset: usize,
}

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
