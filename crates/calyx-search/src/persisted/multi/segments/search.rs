use super::*;
use crate::persisted::multi::pinned;
#[cfg(feature = "cuda")]
use crate::persisted::multi::pinned::PinnedSegmentSpec;
#[cfg(feature = "cuda")]
use std::fs;
#[cfg(feature = "cuda")]
use std::io::{BufReader, Read};
#[cfg(feature = "cuda")]
use std::time::Instant;

use calyx_sextant::index::MAXSIM_CUDA_MAX_K;
#[cfg(feature = "cuda")]
use calyx_sextant::index::{MaxSimCudaChunk, MaxSimCudaRequest, maxsim_cuda_topk};

#[path = "search/config.rs"]
mod config;
#[cfg(feature = "cuda")]
use config::{maxsim_cuda_chunk_rows, maxsim_cuda_chunk_tokens};
#[cfg(feature = "cuda")]
#[path = "search/cuda.rs"]
mod cuda;
use config::{maxsim_cuda_disabled, maxsim_cuda_min_tokens, maxsim_cuda_strict};
#[cfg(feature = "cuda")]
use cuda::{cuda_scores, maxsim_cuda_error};
#[cfg(feature = "cuda")]
#[path = "search/resident.rs"]
mod resident;
#[cfg(feature = "cuda")]
use resident::{ResidentCandidateChunkStream, flatten_query};
#[cfg(feature = "cuda")]
#[path = "search/telemetry.rs"]
mod telemetry;
#[cfg(feature = "cuda")]
pub(crate) use telemetry::take_maxsim_cuda_detail;

pub(in crate::persisted::multi) fn search_segments(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
    query_tokens: &[Vec<f32>],
    k: usize,
    candidates: Option<&BTreeSet<CxId>>,
) -> CliResult<Vec<IndexSearchHit>> {
    pinned::observe_generation(vault_dir, slot, entry.require_sha256(slot)?)?;
    let manifest = read_segments_manifest(vault_dir, entry, manifest_base_seq, slot)?;
    let token_dim = entry.require_token_dim(slot)?;
    #[cfg(feature = "cuda")]
    if let Some(candidates) = candidates
        && !maxsim_cuda_strict()
    {
        return search_resident_candidates_exact(
            vault_dir,
            entry,
            slot,
            token_dim,
            &manifest,
            query_tokens,
            k,
            candidates,
        );
    }
    if let Some(scored) = search_segments_cuda(
        vault_dir,
        entry,
        slot,
        token_dim,
        &manifest,
        query_tokens,
        k,
        candidates,
    )? {
        return Ok(ranked(scored));
    }
    // CPU serving path: stream each verified segment row-by-row, scoring with
    // the exact `MaxSimIndex::maxsim` arithmetic and holding only one decoded
    // row plus the running scored vector. No lens is ever fully materialized or
    // pinned resident, so the peak working set is independent of the lens token
    // volume (issue #1845).
    score_segments_streaming(
        vault_dir,
        slot,
        token_dim,
        &manifest,
        query_tokens,
        k,
        candidates,
    )
}

#[cfg(feature = "cuda")]
#[allow(clippy::too_many_arguments)]
fn search_resident_candidates_exact(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    token_dim: u32,
    manifest: &MultiSegmentsManifest,
    query_tokens: &[Vec<f32>],
    k: usize,
    candidates: &BTreeSet<CxId>,
) -> CliResult<Vec<IndexSearchHit>> {
    let started = Instant::now();
    validate_or_memoize_segments(vault_dir, entry, slot, token_dim, manifest)?;
    let specs = pinned_segment_specs(vault_dir, slot, token_dim, manifest)?;
    let access = pinned::pinned_index(vault_dir, entry, slot, &specs)?;
    let selection = access.index.select_candidates(candidates);
    let scored = access
        .index
        .score_candidates_exact(&selection, query_tokens)?;
    telemetry::record_cpu(telemetry::resident_cpu_report(
        slot,
        candidates.len(),
        &selection,
        &access,
        &specs,
        query_tokens.len(),
        started.elapsed(),
    ));
    Ok(ranked(top_k(scored, k)))
}

/// Bounded, streaming CPU MaxSim scorer over the segmented sidecars. Each
/// segment is verified (sha256 + header/row/token invariants) as it is read
/// and scored one row at a time; the only retained state is the `(cx_id,
/// score)` vector and a single decoded row, so resident memory does not scale
/// with the lens token volume. Each row is scored with the canonical
/// `MaxSimIndex::maxsim` / `cosine` arithmetic and the canonical `top_k`
/// tie-break, preserving the established ranking contract.
#[allow(clippy::too_many_arguments)]
fn score_segments_streaming(
    vault_dir: &Path,
    slot: SlotId,
    token_dim: u32,
    manifest: &MultiSegmentsManifest,
    query_tokens: &[Vec<f32>],
    k: usize,
    candidates: Option<&BTreeSet<CxId>>,
) -> CliResult<Vec<IndexSearchHit>> {
    let mut seen = BTreeSet::new();
    let mut scored = Vec::new();
    let mut scanned_rows = 0u64;
    let mut scanned_tokens = 0u64;
    for segment in &manifest.segments {
        bounds::ensure_segment_ref_bounded(slot, token_dim, segment)?;
        let path = checked_segment_path(vault_dir, &segment.index_rel, slot)?;
        let expect = binary::SegmentExpectation {
            index_rel: &segment.index_rel,
            sha256: &segment.sha256,
            base_seq: segment.base_seq,
            row_count: segment.row_count as u64,
            token_count: segment.token_count as u64,
        };
        binary::stream_score_segment(
            &path,
            slot,
            token_dim,
            &expect,
            query_tokens,
            candidates,
            &mut seen,
            &mut scored,
        )?;
        scanned_rows += segment.row_count as u64;
        scanned_tokens += segment.token_count as u64;
    }
    if scanned_rows != manifest.row_count as u64 {
        return Err(stale(format!(
            "persistent segmented multi manifest row_count {} != scanned row count {scanned_rows}; rebuild the vault search indexes",
            manifest.row_count
        )));
    }
    if scanned_tokens != manifest.token_count as u64 {
        return Err(stale(format!(
            "persistent segmented multi manifest token_count {} != scanned token count {scanned_tokens}; rebuild the vault search indexes",
            manifest.token_count
        )));
    }
    Ok(ranked(top_k(scored, k)))
}

#[allow(clippy::too_many_arguments)]
fn search_segments_cuda(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    token_dim: u32,
    manifest: &MultiSegmentsManifest,
    query_tokens: &[Vec<f32>],
    k: usize,
    candidates: Option<&BTreeSet<CxId>>,
) -> CliResult<Option<Vec<(CxId, f32)>>> {
    #[cfg(feature = "cuda")]
    let slot_started = Instant::now();
    let strict = maxsim_cuda_strict();
    if maxsim_cuda_disabled() {
        if strict {
            return Err(stale(
                "persistent MaxSim CUDA strict mode requested but CALYX_SEARCH_MAXSIM_CUDA=0 disabled it",
            ));
        }
        return Ok(None);
    }
    if manifest.token_count < maxsim_cuda_min_tokens() && !strict {
        return Ok(None);
    }
    if k > MAXSIM_CUDA_MAX_K {
        return Err(stale(format!(
            "persistent MaxSim CUDA workload has {} tokens but k {k} exceeds max {MAXSIM_CUDA_MAX_K}; lower k or set CALYX_SEARCH_MAXSIM_CUDA=0 to force explicit CPU fallback",
            manifest.token_count
        )));
    }
    validate_or_memoize_segments(vault_dir, entry, slot, token_dim, manifest)?;

    #[cfg(not(feature = "cuda"))]
    {
        let _ = (query_tokens, candidates);
        Err(stale(format!(
            "persistent MaxSim CUDA workload has {} tokens but calyx-search was built without --features cuda; rebuild with CUDA support or set CALYX_SEARCH_MAXSIM_CUDA=0 to force explicit CPU fallback",
            manifest.token_count
        )))
    }

    #[cfg(feature = "cuda")]
    {
        let owned_specs = pinned_segment_specs(vault_dir, slot, token_dim, manifest)?;
        let specs = owned_specs.as_slice();
        let flat_query = flatten_query(query_tokens, token_dim as usize)?;
        let chunk_rows = maxsim_cuda_chunk_rows();
        let chunk_tokens = maxsim_cuda_chunk_tokens();
        if let Some(candidates) = candidates {
            let access = pinned::pinned_index(vault_dir, entry, slot, specs)?;
            let requested_candidates = candidates.len();
            let mut stream = ResidentCandidateChunkStream::new(access.index.clone(), candidates);
            let result = maxsim_cuda_topk(
                MaxSimCudaRequest {
                    token_dim: token_dim as usize,
                    total_rows: stream.row_count(),
                    total_tokens: stream.token_count(),
                    query_tokens: &flat_query,
                    query_token_count: query_tokens.len(),
                    k,
                    chunk_rows,
                    chunk_tokens,
                },
                |row_start, max_rows, max_tokens| {
                    Ok(stream.next_chunk(row_start, max_rows, max_tokens)?)
                },
            )
            .map_err(maxsim_cuda_error(slot))?;
            let report = telemetry::resident_report(
                slot,
                strict,
                requested_candidates,
                &stream,
                &access,
                specs,
                slot_started.elapsed(),
                &result.report,
            );
            telemetry::record(report, &result.report)?;
            return Ok(Some(cuda_scores(result, k)));
        }
        let mut stream = SegmentChunkStream::new(vault_dir, slot, token_dim, manifest, candidates)?;
        let result = maxsim_cuda_topk(
            MaxSimCudaRequest {
                token_dim: token_dim as usize,
                total_rows: manifest.row_count,
                total_tokens: manifest.token_count,
                query_tokens: &flat_query,
                query_token_count: query_tokens.len(),
                k,
                chunk_rows,
                chunk_tokens,
            },
            |row_start, max_rows, max_tokens| {
                Ok(stream.next_chunk(row_start, max_rows, max_tokens)?)
            },
        )
        .map_err(maxsim_cuda_error(slot))?;
        let report = telemetry::stream_report(
            slot,
            strict,
            manifest,
            specs,
            slot_started.elapsed(),
            &result.report,
        );
        telemetry::record(report, &result.report)?;
        Ok(Some(cuda_scores(result, k)))
    }
}

#[cfg(feature = "cuda")]
pub(in crate::persisted::multi) fn pinned_segment_specs(
    vault_dir: &Path,
    slot: SlotId,
    token_dim: u32,
    manifest: &MultiSegmentsManifest,
) -> CliResult<Vec<PinnedSegmentSpec>> {
    let mut specs = Vec::with_capacity(manifest.segments.len());
    for segment in &manifest.segments {
        bounds::ensure_segment_ref_bounded(slot, token_dim, segment)?;
        specs.push(PinnedSegmentSpec {
            path: checked_segment_path(vault_dir, &segment.index_rel, slot)?,
            index_rel: segment.index_rel.clone(),
            sha256: segment.sha256.clone(),
            base_seq: segment.base_seq,
            row_count: segment.row_count as u64,
            token_count: segment.token_count as u64,
            byte_len: bounds::segment_estimated_bytes(
                token_dim,
                segment.row_count,
                segment.token_count,
            )?,
            expected_ids: segment.ids.clone(),
        });
    }
    Ok(specs)
}

fn validate_or_memoize_segments(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    token_dim: u32,
    manifest: &MultiSegmentsManifest,
) -> CliResult {
    let entry_sha256 = entry.require_sha256(slot)?;
    if let Some(files) = pinned::memoized_bounded_segment_files(vault_dir, slot, entry_sha256)? {
        return pinned::stat_check_segment_files(slot, &files);
    }
    let files = validate_segment_files(vault_dir, slot, token_dim, manifest)?;
    pinned::memoize_bounded_segment_files(vault_dir, slot, entry_sha256, files)
}

#[cfg(feature = "cuda")]
struct SegmentChunkStream<'a> {
    vault_dir: &'a Path,
    slot: SlotId,
    token_dim: u32,
    segments: &'a [MultiSegmentRef],
    candidates: Option<&'a BTreeSet<CxId>>,
    segment_idx: usize,
    reader: Option<OpenSegment>,
    pending: Option<DecodedRow>,
    rows_read: usize,
}

#[cfg(feature = "cuda")]
struct OpenSegment {
    index_rel: String,
    expected_rows: usize,
    expected_tokens: usize,
    rows_read: usize,
    tokens_read: usize,
    reader: BufReader<fs::File>,
}

#[cfg(feature = "cuda")]
struct DecodedRow {
    cx_id: CxId,
    tokens: Vec<f32>,
    norms: Vec<f32>,
}

#[cfg(feature = "cuda")]
impl<'a> SegmentChunkStream<'a> {
    fn new(
        vault_dir: &'a Path,
        slot: SlotId,
        token_dim: u32,
        manifest: &'a MultiSegmentsManifest,
        candidates: Option<&'a BTreeSet<CxId>>,
    ) -> CliResult<Self> {
        Ok(Self {
            vault_dir,
            slot,
            token_dim,
            segments: &manifest.segments,
            candidates,
            segment_idx: 0,
            reader: None,
            pending: None,
            rows_read: 0,
        })
    }

    fn next_chunk(
        &mut self,
        expected_row_start: usize,
        max_rows: usize,
        max_tokens: usize,
    ) -> CliResult<Option<MaxSimCudaChunk>> {
        if expected_row_start != self.rows_read {
            return Err(stale(format!(
                "persistent MaxSim CUDA stream requested row {expected_row_start}, but cursor is at {}",
                self.rows_read
            )));
        }
        let mut row_offsets = Vec::with_capacity(max_rows + 1);
        let mut tokens = Vec::new();
        let mut token_norms = Vec::new();
        let mut id_hi = Vec::new();
        let mut id_lo = Vec::new();
        let mut candidate_mask = Vec::new();
        row_offsets.push(0);
        while id_hi.len() < max_rows {
            let row = if let Some(row) = self.pending.take() {
                row
            } else {
                let Some(row) = self.read_next_row()? else {
                    break;
                };
                row
            };
            let row_tokens = row.norms.len();
            if row_tokens > max_tokens {
                return Err(stale(format!(
                    "persistent MaxSim row {} has {row_tokens} tokens, exceeding CUDA chunk token budget {max_tokens}; raise CALYX_SEARCH_MAXSIM_CUDA_CHUNK_TOKENS",
                    row.cx_id
                )));
            }
            if !id_hi.is_empty() && token_norms.len() + row_tokens > max_tokens {
                self.pending = Some(row);
                break;
            }
            let (hi, lo) = cx_id_halves(row.cx_id);
            id_hi.push(hi);
            id_lo.push(lo);
            candidate_mask.push(u8::from(
                self.candidates
                    .is_none_or(|allowed| allowed.contains(&row.cx_id)),
            ));
            tokens.extend_from_slice(&row.tokens);
            token_norms.extend_from_slice(&row.norms);
            row_offsets.push(
                u32::try_from(token_norms.len())
                    .map_err(|_| stale("persistent MaxSim CUDA chunk token offsets exceed u32"))?,
            );
            self.rows_read += 1;
        }
        if id_hi.is_empty() {
            return Ok(None);
        }
        Ok(Some(MaxSimCudaChunk {
            row_count: id_hi.len(),
            token_count: token_norms.len(),
            row_offsets,
            tokens,
            token_norms,
            id_hi,
            id_lo,
            candidate_mask,
        }))
    }

    fn read_next_row(&mut self) -> CliResult<Option<DecodedRow>> {
        loop {
            if self.reader.is_none() {
                if self.segment_idx >= self.segments.len() {
                    return Ok(None);
                }
                self.reader = Some(open_segment(
                    self.vault_dir,
                    self.slot,
                    self.token_dim,
                    &self.segments[self.segment_idx],
                )?);
                self.segment_idx += 1;
            }
            let segment = self.reader.as_mut().expect("segment open");
            if segment.rows_read >= segment.expected_rows {
                if segment.tokens_read != segment.expected_tokens {
                    return Err(stale(format!(
                        "persistent segmented multi sidecar {} token_count {} != expected {}; rebuild the vault search indexes",
                        segment.index_rel, segment.tokens_read, segment.expected_tokens
                    )));
                }
                self.reader = None;
                continue;
            }
            let row = read_segment_row(self.slot, self.token_dim, segment)?;
            segment.rows_read += 1;
            segment.tokens_read += row.norms.len();
            return Ok(Some(row));
        }
    }
}

#[cfg(feature = "cuda")]
fn open_segment(
    vault_dir: &Path,
    slot: SlotId,
    token_dim: u32,
    segment: &MultiSegmentRef,
) -> CliResult<OpenSegment> {
    let path = checked_segment_path(vault_dir, &segment.index_rel, slot)?;
    let mut reader = BufReader::new(fs::File::open(&path)?);
    let mut magic = [0_u8; 16];
    reader.read_exact(&mut magic)?;
    if &magic != binary::MULTI_BINARY_MAGIC {
        return Err(stale(format!(
            "persistent binary multi sidecar {} has invalid magic; rebuild the vault search indexes",
            segment.index_rel
        )));
    }
    let header_slot = read_u16(&mut reader)?;
    let header_token_dim = read_u32(&mut reader)?;
    let header_base_seq = read_u64(&mut reader)?;
    let header_row_count = read_u64(&mut reader)? as usize;
    let header_token_count = read_u64(&mut reader)? as usize;
    if header_slot != slot.get()
        || header_token_dim != token_dim
        || header_base_seq != segment.base_seq
        || header_row_count != segment.row_count
        || header_token_count != segment.token_count
    {
        return Err(stale(format!(
            "persistent binary multi sidecar {} header does not match segment manifest; rebuild the vault search indexes",
            segment.index_rel
        )));
    }
    Ok(OpenSegment {
        index_rel: segment.index_rel.clone(),
        expected_rows: segment.row_count,
        expected_tokens: segment.token_count,
        rows_read: 0,
        tokens_read: 0,
        reader,
    })
}

#[cfg(feature = "cuda")]
fn read_segment_row(
    slot: SlotId,
    token_dim: u32,
    segment: &mut OpenSegment,
) -> CliResult<DecodedRow> {
    let mut id = [0_u8; 16];
    segment.reader.read_exact(&mut id)?;
    let cx_id = CxId::from_bytes(id);
    let row_token_count = read_u32(&mut segment.reader)? as usize;
    let dim = token_dim as usize;
    let mut tokens = Vec::with_capacity(row_token_count * dim);
    let mut norms = Vec::with_capacity(row_token_count);
    let mut bytes = [0_u8; 4];
    for _ in 0..row_token_count {
        let mut squared = 0.0_f32;
        for _ in 0..dim {
            segment.reader.read_exact(&mut bytes)?;
            let value = f32::from_le_bytes(bytes);
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
    Ok(DecodedRow {
        cx_id,
        tokens,
        norms,
    })
}

#[cfg(feature = "cuda")]
fn read_u16<R: Read>(reader: &mut R) -> CliResult<u16> {
    let mut bytes = [0_u8; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

#[cfg(feature = "cuda")]
fn read_u32<R: Read>(reader: &mut R) -> CliResult<u32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(feature = "cuda")]
fn read_u64<R: Read>(reader: &mut R) -> CliResult<u64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(feature = "cuda")]
fn cx_id_halves(cx_id: CxId) -> (u64, u64) {
    let bytes = cx_id.to_bytes();
    let hi = u64::from_be_bytes(bytes[..8].try_into().expect("8 bytes"));
    let lo = u64::from_be_bytes(bytes[8..].try_into().expect("8 bytes"));
    (hi, lo)
}

#[cfg(feature = "cuda")]
fn cx_id_from_halves(hi: u64, lo: u64) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&hi.to_be_bytes());
    bytes[8..].copy_from_slice(&lo.to_be_bytes());
    CxId::from_bytes(bytes)
}
