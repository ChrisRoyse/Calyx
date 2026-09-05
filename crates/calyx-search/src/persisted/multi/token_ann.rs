//! Token-level ANN candidate generation for segmented late-interaction lanes.
//!
//! Exact MaxSim belongs in the final panel rerank. Candidate generation only
//! needs a high-recall document ID union, so each stored token is indexed once
//! with DiskANN/CAGRA and mapped back to its owning constellation.

#[cfg(feature = "cuda")]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(feature = "cuda")]
use std::sync::{Arc, Mutex};

use calyx_aster::vault::encode::EncodedMultiSlotVector;
#[cfg(feature = "cuda")]
use calyx_core::SlotVector;
use calyx_core::{CxId, SlotId};
#[cfg(feature = "cuda")]
use calyx_sextant::index::{
    CAGRA_SERVING_MAX_BATCH, CAGRA_SERVING_MAX_K, DiskAnnSearch, DiskAnnSearchParams,
    IndexSearchHit, SextantIndex, ranked,
};
use calyx_sextant::index::{
    DiskAnnBuildBackend, DiskAnnBuildMetric, DiskAnnBuildParams, build_diskann_graph_with_backend,
    cagra_dataset_sidecar_path, cagra_sidecar_path, open_diskann_graph, read_diskann_build_metric,
};

use super::super::{SearchIndexEntry, rel, sha256_hex, stale, write_atomic_hashed};
use crate::error::CliResult;

const TOKEN_MAP_MAGIC: &[u8; 16] = b"CYX_MULTI_ANN_V1";

pub(in crate::persisted) struct TokenAnnBuilder {
    slot: SlotId,
    token_dim: u32,
    base_seq: u64,
    backend: DiskAnnBuildBackend,
    vectors: Vec<(u32, Vec<f32>)>,
    token_docs: Vec<CxId>,
}

pub(in crate::persisted) struct TokenAnnArtifacts {
    pub(in crate::persisted) graph_rel: String,
    pub(in crate::persisted) id_map_rel: String,
    pub(in crate::persisted) id_map_sha256: String,
    pub(in crate::persisted) directory: PathBuf,
}

impl TokenAnnBuilder {
    pub(in crate::persisted) fn new(
        slot: SlotId,
        token_dim: u32,
        base_seq: u64,
        backend: DiskAnnBuildBackend,
    ) -> Self {
        Self {
            slot,
            token_dim,
            base_seq,
            backend,
            vectors: Vec::new(),
            token_docs: Vec::new(),
        }
    }

    pub(in crate::persisted) fn push_encoded(
        &mut self,
        cx_id: CxId,
        encoded: EncodedMultiSlotVector<'_>,
    ) -> CliResult {
        if encoded.token_dim() != self.token_dim {
            return Err(stale(format!(
                "slot {} token ANN row {cx_id} has dim {}, expected {}",
                self.slot,
                encoded.token_dim(),
                self.token_dim
            )));
        }
        let dim = self.token_dim as usize;
        let mut components = encoded.components();
        for _ in 0..encoded.token_count() {
            let local_id = u32::try_from(self.vectors.len())
                .map_err(|_| stale("multi token ANN exceeds u32::MAX tokens"))?;
            let token = components.by_ref().take(dim).collect::<Vec<_>>();
            if token.len() != dim || token.iter().any(|value| !value.is_finite()) {
                return Err(stale(format!(
                    "slot {} token ANN row {cx_id} contains a truncated or non-finite token",
                    self.slot
                )));
            }
            self.vectors.push((local_id, token));
            self.token_docs.push(cx_id);
        }
        if components.next().is_some() {
            return Err(stale(format!(
                "slot {} token ANN row {cx_id} contains trailing token components",
                self.slot
            )));
        }
        Ok(())
    }

    pub(in crate::persisted) fn finish(
        self,
        vault_dir: &Path,
        root: &Path,
        doc_count: usize,
    ) -> CliResult<TokenAnnArtifacts> {
        if self.vectors.is_empty() || self.token_docs.len() != self.vectors.len() {
            return Err(stale(format!(
                "slot {} token ANN has no tokens or an inconsistent token map",
                self.slot
            )));
        }
        let directory = root.join(format!(
            "slot_{:05}_seq_{:020}_n_{:010}_t_{:010}.multi-ann",
            self.slot.get(),
            self.base_seq,
            doc_count,
            self.vectors.len()
        ));
        if directory.exists() {
            fs::remove_dir_all(&directory)?;
        }
        calyx_aster::private_fs::create_private_dir_all(&directory)?;
        let graph_path = directory.join("graph.cda");
        if let Err(error) = build_diskann_graph_with_backend(
            &graph_path,
            &self.vectors,
            build_params(self.token_dim as usize),
            self.backend,
        ) {
            let _ = fs::remove_dir_all(&directory);
            return Err(error.into());
        }
        let map_path = directory.join("token-docs.bin");
        let map_sha256 = match write_token_map(
            &map_path,
            self.slot,
            self.token_dim,
            self.base_seq,
            doc_count,
            &self.token_docs,
        ) {
            Ok(sha256) => sha256,
            Err(error) => {
                let _ = fs::remove_dir_all(&directory);
                return Err(error);
            }
        };
        Ok(TokenAnnArtifacts {
            graph_rel: rel(vault_dir, &graph_path)?,
            id_map_rel: rel(vault_dir, &map_path)?,
            id_map_sha256: map_sha256,
            directory,
        })
    }
}

fn write_token_map(
    path: &Path,
    slot: SlotId,
    token_dim: u32,
    base_seq: u64,
    doc_count: usize,
    token_docs: &[CxId],
) -> CliResult<String> {
    write_atomic_hashed(path, |writer| {
        writer.write_all(TOKEN_MAP_MAGIC)?;
        writer.write_all(&slot.get().to_le_bytes())?;
        writer.write_all(&token_dim.to_le_bytes())?;
        writer.write_all(&base_seq.to_le_bytes())?;
        writer.write_all(&(doc_count as u64).to_le_bytes())?;
        writer.write_all(&(token_docs.len() as u64).to_le_bytes())?;
        for cx_id in token_docs {
            writer.write_all(cx_id.as_bytes())?;
        }
        Ok(())
    })
}

#[derive(Debug)]
#[cfg(feature = "cuda")]
struct TokenAnnIndex {
    search: DiskAnnSearch,
    token_docs: Vec<CxId>,
    token_dim: usize,
}

#[derive(Debug, Default)]
#[cfg(feature = "cuda")]
pub(in crate::persisted) struct TokenAnnCache {
    indexes: Mutex<BTreeMap<u16, Arc<TokenAnnIndex>>>,
}

#[cfg(feature = "cuda")]
pub(in crate::persisted) fn search_gpu_batch(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    queries: &[SlotVector],
    k: usize,
    cache: &TokenAnnCache,
) -> CliResult<Vec<Vec<IndexSearchHit>>> {
    if queries.is_empty() || k == 0 {
        return Ok(vec![Vec::new(); queries.len()]);
    }
    let index = open_cached(vault_dir, entry, slot, cache)?;
    let (flat, offsets) = flatten_queries(queries, slot, index.token_dim)?;
    let query_token_count = offsets.last().copied().unwrap_or(0);
    if query_token_count == 0 {
        return Err(stale(format!(
            "slot {slot} token ANN candidate batch contains no query tokens"
        )));
    }
    let token_k = k
        .saturating_mul(4)
        .clamp(64, CAGRA_SERVING_MAX_K)
        .min(index.token_docs.len());
    let mut out = Vec::with_capacity(queries.len());
    let mut query_start = 0usize;
    while query_start < queries.len() {
        let token_start = offsets[query_start];
        let mut query_end = query_start;
        while query_end < queries.len() {
            let next_token_end = offsets[query_end + 1];
            let next_token_count = next_token_end - token_start;
            if next_token_count > CAGRA_SERVING_MAX_BATCH {
                if query_end == query_start {
                    return Err(stale(format!(
                        "slot {slot} token ANN panel query {query_start} has {next_token_count} tokens, exceeding the CAGRA maximum of {CAGRA_SERVING_MAX_BATCH}"
                    )));
                }
                break;
            }
            query_end += 1;
        }

        let token_end = offsets[query_end];
        let chunk_token_count = token_end - token_start;
        let component_start = token_start.checked_mul(index.token_dim).ok_or_else(|| {
            stale(format!(
                "slot {slot} token ANN component offset overflow at token {token_start}"
            ))
        })?;
        let component_end = token_end.checked_mul(index.token_dim).ok_or_else(|| {
            stale(format!(
                "slot {slot} token ANN component offset overflow at token {token_end}"
            ))
        })?;
        let chunk = flat.get(component_start..component_end).ok_or_else(|| {
            stale(format!(
                "slot {slot} token ANN flattened query range {component_start}..{component_end} exceeds {} components",
                flat.len()
            ))
        })?;
        let token_hits = index.search.search_ids_batch(
            chunk,
            chunk_token_count,
            token_k,
            &search_params(token_k.max(64)),
        )?;
        if token_hits.len() != chunk_token_count {
            return Err(stale(format!(
                "slot {slot} token ANN returned {} token rows for {chunk_token_count} queries in panel range {query_start}..{query_end}",
                token_hits.len()
            )));
        }

        for bounds in offsets[query_start..=query_end].windows(2) {
            let mut aggregate = BTreeMap::<CxId, f32>::new();
            for hits in &token_hits[bounds[0] - token_start..bounds[1] - token_start] {
                let mut per_token = BTreeMap::<CxId, f32>::new();
                for &(token_id, distance) in hits {
                    let cx_id = *index.token_docs.get(token_id as usize).ok_or_else(|| {
                        stale(format!(
                            "slot {slot} token ANN returned out-of-range token id {token_id}"
                        ))
                    })?;
                    let similarity = 1.0 - distance;
                    per_token
                        .entry(cx_id)
                        .and_modify(|score| *score = score.max(similarity))
                        .or_insert(similarity);
                }
                for (cx_id, score) in per_token {
                    *aggregate.entry(cx_id).or_default() += score;
                }
            }
            let mut scored = aggregate.into_iter().collect::<Vec<_>>();
            scored.sort_by(|left, right| {
                right
                    .1
                    .total_cmp(&left.1)
                    .then_with(|| left.0.cmp(&right.0))
            });
            scored.truncate(k);
            out.push(ranked(scored));
        }
        query_start = query_end;
    }
    Ok(out)
}

#[cfg(feature = "cuda")]
fn open_cached(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    cache: &TokenAnnCache,
) -> CliResult<Arc<TokenAnnIndex>> {
    if let Some(index) = cache
        .indexes
        .lock()
        .map_err(|_| stale("persistent multi token ANN cache lock poisoned"))?
        .get(&slot.get())
        .cloned()
    {
        return Ok(index);
    }
    let token_docs = read_token_map(vault_dir, entry, slot)?;
    let graph = vault_dir.join(entry.require_graph_rel(slot)?);
    let ids = (0..token_docs.len()).map(token_cx).collect::<Vec<_>>();
    let mut search = DiskAnnSearch::open_gpu_serving(slot, graph, ids, None, search_params(64))?;
    search.set_base_seq(entry.built_at_seq);
    let index = Arc::new(TokenAnnIndex {
        search,
        token_docs,
        token_dim: entry.require_token_dim(slot)? as usize,
    });
    let mut indexes = cache
        .indexes
        .lock()
        .map_err(|_| stale("persistent multi token ANN cache lock poisoned"))?;
    Ok(Arc::clone(indexes.entry(slot.get()).or_insert(index)))
}

#[cfg(feature = "cuda")]
fn flatten_queries(
    queries: &[SlotVector],
    slot: SlotId,
    token_dim: usize,
) -> CliResult<(Vec<f32>, Vec<usize>)> {
    let mut flat = Vec::new();
    let mut offsets = Vec::with_capacity(queries.len() + 1);
    offsets.push(0);
    for query in queries {
        match query {
            SlotVector::Multi {
                token_dim: dim,
                tokens,
            } => {
                require_query_dim(slot, *dim as usize, token_dim, tokens.len())?;
                for token in tokens {
                    require_token(slot, token, token_dim)?;
                    flat.extend_from_slice(token);
                }
                offsets.push(offsets.last().copied().unwrap_or(0) + tokens.len());
            }
            SlotVector::SegmentedMulti {
                token_dim: dim,
                segments,
                ..
            } => {
                require_query_dim(slot, *dim as usize, token_dim, segments.len())?;
                for segment in segments {
                    require_token(slot, &segment.data, token_dim)?;
                    flat.extend_from_slice(&segment.data);
                }
                offsets.push(offsets.last().copied().unwrap_or(0) + segments.len());
            }
            _ => {
                return Err(stale(format!(
                    "slot {slot} token ANN candidate batch received a non-multi query"
                )));
            }
        }
    }
    Ok((flat, offsets))
}

#[cfg(feature = "cuda")]
fn require_query_dim(
    slot: SlotId,
    actual: usize,
    expected: usize,
    token_count: usize,
) -> CliResult {
    if actual != expected || token_count == 0 {
        return Err(stale(format!(
            "slot {slot} token ANN query has dim {actual} and {token_count} tokens; expected dim {expected} and at least one token"
        )));
    }
    Ok(())
}

#[cfg(feature = "cuda")]
fn require_token(slot: SlotId, token: &[f32], token_dim: usize) -> CliResult {
    if token.len() != token_dim || token.iter().any(|value| !value.is_finite()) {
        return Err(stale(format!(
            "slot {slot} token ANN query contains a malformed or non-finite token"
        )));
    }
    Ok(())
}

pub(in crate::persisted::multi) fn validate_entry(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
) -> CliResult {
    let token_docs = read_token_map(vault_dir, entry, slot)?;
    let graph_path = vault_dir.join(entry.require_graph_rel(slot)?);
    let graph = open_diskann_graph(&graph_path)?;
    let header = graph.header();
    if header.dim != entry.require_token_dim(slot)? || header.node_count != token_docs.len() as u64
    {
        return Err(stale(format!(
            "slot {slot} token ANN graph shape {}x{} != map/manifest {}x{}",
            header.node_count,
            header.dim,
            token_docs.len(),
            entry.require_token_dim(slot)?
        )));
    }
    Ok(())
}

pub(in crate::persisted::multi) fn validate_gpu_assets(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
) -> CliResult {
    let graph_path = vault_dir.join(entry.require_graph_rel(slot)?);
    for path in [
        cagra_sidecar_path(&graph_path),
        cagra_dataset_sidecar_path(&graph_path),
    ] {
        let valid = path
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0);
        if !valid {
            return Err(stale(format!(
                "slot {slot} token ANN required CAGRA asset {} is missing, empty, or not a regular file; rebuild with cuVS CAGRA",
                path.display()
            )));
        }
    }
    let metric = read_diskann_build_metric(&graph_path)?;
    if metric != DiskAnnBuildMetric::UnitL2 {
        return Err(stale(format!(
            "slot {slot} token ANN metric is {metric:?}, expected UnitL2 for CAGRA serving; rebuild the index"
        )));
    }
    Ok(())
}

fn read_token_map(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
) -> CliResult<Vec<CxId>> {
    let path = vault_dir.join(entry.require_id_map_rel(slot)?);
    let bytes = fs::read(&path)?;
    let actual_sha256 = sha256_hex(&bytes);
    let expected_sha256 = entry.require_id_map_sha256(slot)?;
    if actual_sha256 != expected_sha256 {
        return Err(stale(format!(
            "slot {slot} token ANN map sha256 {actual_sha256} != manifest {expected_sha256}"
        )));
    }
    let mut reader = Cursor::new(bytes.as_slice());
    let mut magic = [0u8; 16];
    reader.read_exact(&mut magic)?;
    if &magic != TOKEN_MAP_MAGIC {
        return Err(stale(format!(
            "slot {slot} token ANN map {} has invalid magic",
            path.display()
        )));
    }
    let map_slot = read_u16(&mut reader)?;
    let token_dim = read_u32(&mut reader)?;
    let base_seq = read_u64(&mut reader)?;
    let doc_count = read_u64(&mut reader)? as usize;
    let token_count = read_u64(&mut reader)? as usize;
    if map_slot != slot.get()
        || token_dim != entry.require_token_dim(slot)?
        || base_seq != entry.built_at_seq
        || doc_count != entry.len
        || token_count != entry.require_token_count(slot)?
    {
        return Err(stale(format!(
            "slot {slot} token ANN map header disagrees with the search manifest"
        )));
    }
    let expected_bytes = 46usize
        .checked_add(token_count.checked_mul(16).ok_or_else(|| {
            stale(format!(
                "slot {slot} token ANN map byte length overflows usize"
            ))
        })?)
        .ok_or_else(|| {
            stale(format!(
                "slot {slot} token ANN map byte length overflows usize"
            ))
        })?;
    if bytes.len() != expected_bytes {
        return Err(stale(format!(
            "slot {slot} token ANN map has {} bytes, expected {expected_bytes}",
            bytes.len()
        )));
    }
    let mut token_docs = Vec::with_capacity(token_count);
    for _ in 0..token_count {
        let mut raw = [0u8; 16];
        reader.read_exact(&mut raw)?;
        token_docs.push(CxId::from_bytes(raw));
    }
    let unique_docs = token_docs.iter().copied().collect::<BTreeSet<_>>();
    if unique_docs.len() != doc_count {
        return Err(stale(format!(
            "slot {slot} token ANN map covers {} documents, expected {doc_count}",
            unique_docs.len()
        )));
    }
    Ok(token_docs)
}

fn read_u16(reader: &mut Cursor<&[u8]>) -> CliResult<u16> {
    let mut bytes = [0u8; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(reader: &mut Cursor<&[u8]>) -> CliResult<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut Cursor<&[u8]>) -> CliResult<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(feature = "cuda")]
fn token_cx(local_id: usize) -> CxId {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&(local_id as u64).to_le_bytes());
    bytes[8..].copy_from_slice(b"CYXTOKV1");
    CxId::from_bytes(bytes)
}

fn build_params(dim: usize) -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim,
        m_max: 32,
        ef_construction: 64,
        alpha: 1.2,
    }
}

#[cfg(feature = "cuda")]
fn search_params(ef: usize) -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: 32,
        ef_search: ef,
        rescore_k: ef,
        rescore_from_raw: false,
    }
}
