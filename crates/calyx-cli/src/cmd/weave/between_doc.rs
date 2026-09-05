//! Slot-major, batched between-document graph construction (#1997).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, SlotId, SlotVector};
use calyx_lodestar::{
    LodestarError, PANEL_RRF_K, TypedPanelFusionHit, TypedPanelFusionLane, TypedPanelVectors,
    rank_typed_panel_candidate_refs,
};
use calyx_paths::AssocGraph;
use calyx_sextant::index::CAGRA_SERVING_MAX_BATCH;
use rayon::prelude::*;
use serde::Serialize;

use super::super::PersistedSearchIndexes;
use super::coverage::WeaveNodeMeta;
use super::panel_cache::{PanelCache, PanelCacheStats};
use super::passes::WEAVE_COMMIT_BYTE_BUDGET;
use super::scratch::{OrdinalMap, WeaveVectorScratch};
use crate::error::{CliError, CliResult};

pub(super) const EDGE_TYPE: &str = "knn";
const EDGE_FLUSH_ROWS: usize = 8192;
// Share the serving boundary instead of duplicating it: every GPU batch formed
// here must be admissible by the CAGRA FFI that consumes it.
const CANDIDATE_BATCH_MAX_ROWS: usize = CAGRA_SERVING_MAX_BATCH;
const CANDIDATE_BATCH_MAX_BYTES: u64 = 64 * 1024 * 1024;
const PANEL_CACHE_MAX_BYTES: u64 = 512 * 1024 * 1024;
const EXACT_RERANK_BATCH_MAX_NODES: usize = 8;
const EXACT_RERANK_BATCH_MAX_BYTES: u64 = 384 * 1024 * 1024;
const BOUNDED_EXACT_DOMAIN_MAX_NODES: usize = 4096;

#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct BetweenDocProgress {
    pub stage: &'static str,
    pub nodes_total: usize,
    pub nodes_processed: usize,
    pub edges_persisted: usize,
    pub candidate_slots_total: usize,
    pub candidate_slots_processed: usize,
    pub candidate_batches: usize,
    pub candidate_queries: usize,
}

pub(super) struct BetweenDocGraphRequest<'a> {
    pub indexes: &'a PersistedSearchIndexes,
    pub embedding_slots: &'a [SlotId],
    pub knn: usize,
    pub edge_score_threshold: f32,
    pub candidates: &'a [WeaveNodeMeta],
    pub candidate_ordinals: &'a [u64],
    pub ordinal_map: &'a OrdinalMap,
    pub scratch: &'a WeaveVectorScratch,
    pub candidate_device: super::CandidateDevice,
    /// Whether the selected graph domain covers the complete Base scan. A
    /// bounded domain is small by design and is reranked exactly over every
    /// selected pair; global ANN hits alone cannot correctly implement a
    /// filter to an arbitrary prefix.
    pub candidate_domain_complete: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(super) struct BetweenDocTelemetry {
    pub candidate_slots: usize,
    pub candidate_batches: usize,
    pub candidate_queries: usize,
    pub max_batch_rows: usize,
    pub max_batch_encoded_bytes: u64,
    pub ann_candidate_pairs: usize,
    pub bounded_exact_domain: bool,
    pub unique_candidate_pairs: usize,
    pub exact_rerank_batches: usize,
    pub max_exact_rerank_batch_nodes: usize,
    pub max_exact_rerank_batch_bytes: u64,
    pub panel_cache: PanelCacheStats,
}

pub(super) struct BetweenDocBuildResult {
    pub edges_persisted: usize,
    pub graph: AssocGraph,
    pub telemetry: BetweenDocTelemetry,
}

#[derive(Serialize)]
struct EdgeValue {
    weight: f32,
    fusion: &'static str,
    rrf_k: u32,
    fused_rank: usize,
    lanes: Vec<TypedPanelFusionLane>,
}

/// Candidate generation is deliberately slot-major. One manifest generation is
/// opened and kept hot while the full corpus is searched in bounded batches;
/// only after every query for that slot completes may the next GPU generation
/// displace it from the CAGRA LRU.
fn generate_candidates(
    request: &BetweenDocGraphRequest<'_>,
    node_set: &HashSet<CxId>,
    candidate_k: usize,
    progress: &mut Option<&mut dyn FnMut(BetweenDocProgress) -> CliResult>,
) -> CliResult<(Vec<Vec<CxId>>, BetweenDocTelemetry)> {
    let nodes_total = request.candidates.len();
    let mut candidates = vec![Vec::new(); nodes_total];
    let mut telemetry = BetweenDocTelemetry::default();

    for (slot_offset, &slot) in request.embedding_slots.iter().enumerate() {
        let mut start = 0usize;
        while start < nodes_total {
            let mut end = start;
            let mut encoded_bytes = 0_u64;
            while end < nodes_total && end - start < CANDIDATE_BATCH_MAX_ROWS {
                let record_bytes = request
                    .scratch
                    .vector_record_bytes(slot, request.candidate_ordinals[end])?;
                if record_bytes == 0 {
                    return Err(CliError::runtime(format!(
                        "graph source {} lost contracted slot {slot}",
                        request.candidates[end].cx_id
                    )));
                }
                if end > start
                    && encoded_bytes.saturating_add(record_bytes) > CANDIDATE_BATCH_MAX_BYTES
                {
                    break;
                }
                encoded_bytes = encoded_bytes.saturating_add(record_bytes);
                end += 1;
            }
            let queries = request.candidate_ordinals[start..end]
                .iter()
                .map(|&ordinal| {
                    request.scratch.read_vector(slot, ordinal)?.ok_or_else(|| {
                        CliError::runtime(format!(
                            "weave scratch ordinal {ordinal} has no measured vector in contracted slot {slot}"
                        ))
                    })
                })
                .collect::<CliResult<Vec<SlotVector>>>()?;
            let hit_batches = match request.candidate_device {
                super::CandidateDevice::Cpu => {
                    request.indexes.search_batch(slot, &queries, candidate_k)?
                }
                super::CandidateDevice::Gpu => {
                    request
                        .indexes
                        .search_candidates_gpu_batch(slot, &queries, candidate_k)?
                }
            };
            if hit_batches.len() != queries.len() {
                return Err(CliError::runtime(format!(
                    "slot {slot} candidate batch returned {} rows for {} queries",
                    hit_batches.len(),
                    queries.len()
                )));
            }
            for (batch_offset, hits) in hit_batches.into_iter().enumerate() {
                let node_index = start + batch_offset;
                let cx_id = request.candidates[node_index].cx_id;
                candidates[node_index].extend(hits.into_iter().filter_map(|hit| {
                    (hit.cx_id != cx_id && node_set.contains(&hit.cx_id)).then_some(hit.cx_id)
                }));
            }

            telemetry.candidate_batches += 1;
            telemetry.candidate_queries += queries.len();
            telemetry.max_batch_rows = telemetry.max_batch_rows.max(queries.len());
            telemetry.max_batch_encoded_bytes =
                telemetry.max_batch_encoded_bytes.max(encoded_bytes);
            start = end;
            if let Some(callback) = progress.as_mut() {
                callback(BetweenDocProgress {
                    stage: "candidate_generation",
                    nodes_total,
                    nodes_processed: 0,
                    edges_persisted: 0,
                    candidate_slots_total: request.embedding_slots.len(),
                    candidate_slots_processed: slot_offset,
                    candidate_batches: telemetry.candidate_batches,
                    candidate_queries: telemetry.candidate_queries,
                })?;
            }
        }
        for ids in &mut candidates {
            ids.sort_unstable();
            ids.dedup();
        }
        telemetry.candidate_slots += 1;
    }
    telemetry.ann_candidate_pairs = candidates.iter().map(Vec::len).sum();
    if !request.candidate_domain_complete {
        // `--limit` defines a self-contained graph domain (#2003). A global ANN query
        // followed by post-filtering is not a filtered ANN search: it can omit
        // every member of a small domain even when valid members exist. Make
        // the bounded contract exact by reranking all selected pairs. The ANN
        // batches still execute and remain telemetry-visible, exercising the
        // production index lifecycle without letting an approximate global
        // result corrupt bounded graph semantics.
        let bounded_ids = request
            .candidates
            .iter()
            .map(|candidate| candidate.cx_id)
            .collect::<Vec<_>>();
        for (node_index, ids) in candidates.iter_mut().enumerate() {
            let cx_id = request.candidates[node_index].cx_id;
            ids.extend(bounded_ids.iter().copied().filter(|&id| id != cx_id));
            ids.sort_unstable();
            ids.dedup();
        }
        telemetry.bounded_exact_domain = true;
    }
    telemetry.unique_candidate_pairs = candidates.iter().map(Vec::len).sum();
    Ok((candidates, telemetry))
}

/// Resolve one node's exact-rerank candidate panels. Cache lookup and LRU
/// admission stay in sorted candidate order, while independent scratch decodes for
/// the misses run in parallel. The previous path decoded every miss serially on the
/// coordinator thread before entering the already-parallel similarity calculation;
/// on the real 109,133-row graph that left 34/38 threads asleep during the dominant
/// stage even with the scratch mmap fully page-cached.
fn cached_candidate_panels(
    cache: &mut PanelCache,
    request: &BetweenDocGraphRequest<'_>,
    ids: &[CxId],
) -> CliResult<BTreeMap<CxId, Arc<TypedPanelVectors>>> {
    let mut panels = BTreeMap::new();
    let mut misses = Vec::new();
    for &id in ids {
        if let Some(panel) = cache.lookup(id) {
            panels.insert(id, panel);
            continue;
        }
        let ordinal = request.ordinal_map.ordinal(&id).ok_or_else(|| {
            CliError::runtime(format!("graph candidate {id} has no scratch ordinal"))
        })?;
        let encoded_bytes = request
            .scratch
            .panel_record_bytes(ordinal, request.embedding_slots)?;
        misses.push((id, ordinal, encoded_bytes.saturating_mul(2)));
    }

    let decoded = misses
        .par_iter()
        .map(|&(id, ordinal, admitted_bytes)| {
            request
                .scratch
                .read_panel(ordinal, request.embedding_slots)
                .map(|panel| (id, admitted_bytes, panel))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .collect::<CliResult<Vec<_>>>()?;
    for (id, admitted_bytes, panel) in decoded {
        panels.insert(id, cache.admit_loaded(id, admitted_bytes, panel));
    }
    Ok(panels)
}

/// Load one deterministic, byte-bounded exact-rerank batch. The serial path
/// previously scheduled Rayon independently for each node, leaving the pool
/// with at most one small candidate set at a time. Loading the sorted union
/// preserves deterministic LRU behavior, while the caller can rank independent
/// nodes concurrently. The admitted-byte estimate is deliberately conservative
/// and a single oversized node is still allowed so valid inputs cannot deadlock
/// batch formation.
fn load_exact_panel_batch(
    cache: &mut PanelCache,
    request: &BetweenDocGraphRequest<'_>,
    candidate_ids: &[Vec<CxId>],
    start: usize,
) -> CliResult<(usize, u64, BTreeMap<CxId, Arc<TypedPanelVectors>>)> {
    let mut end = start;
    let mut panel_ids = BTreeSet::new();
    let mut admitted_bytes = 0_u64;
    while end < request.candidates.len() && end - start < EXACT_RERANK_BATCH_MAX_NODES {
        let source_id = request.candidates[end].cx_id;
        let mut additions = Vec::new();
        let mut additional_bytes = 0_u64;
        for id in std::iter::once(&source_id).chain(candidate_ids[end].iter()) {
            if panel_ids.contains(id) || additions.contains(id) {
                continue;
            }
            let ordinal = request.ordinal_map.ordinal(id).ok_or_else(|| {
                CliError::runtime(format!("graph candidate {id} has no scratch ordinal"))
            })?;
            let encoded_bytes = request
                .scratch
                .panel_record_bytes(ordinal, request.embedding_slots)?;
            additional_bytes = additional_bytes.saturating_add(encoded_bytes.saturating_mul(2));
            additions.push(*id);
        }
        if end > start
            && admitted_bytes.saturating_add(additional_bytes) > EXACT_RERANK_BATCH_MAX_BYTES
        {
            break;
        }
        admitted_bytes = admitted_bytes.saturating_add(additional_bytes);
        panel_ids.extend(additions);
        end += 1;
    }
    if end == start {
        return Err(CliError::runtime(format!(
            "exact-rerank batch formation made no progress at node {start}"
        )));
    }
    let panel_ids = panel_ids.into_iter().collect::<Vec<_>>();
    let panels = cached_candidate_panels(cache, request, &panel_ids)?;
    Ok((end, admitted_bytes, panels))
}

fn rank_exact_panel_batch(
    request: &BetweenDocGraphRequest<'_>,
    candidate_ids: &[Vec<CxId>],
    panels: &BTreeMap<CxId, Arc<TypedPanelVectors>>,
    start: usize,
    end: usize,
) -> CliResult<Vec<Vec<TypedPanelFusionHit>>> {
    (start..end)
        .into_par_iter()
        .map(|node_index| {
            let cx_id = request.candidates[node_index].cx_id;
            let vectors = panels.get(&cx_id).ok_or_else(|| {
                CliError::runtime(format!("exact-rerank batch lost source panel for {cx_id}"))
            })?;
            let candidate_refs = candidate_ids[node_index]
                .iter()
                .map(|id| {
                    panels
                        .get(id)
                        .map(|panel| (*id, panel.as_ref()))
                        .ok_or_else(|| {
                            CliError::runtime(format!(
                                "exact-rerank batch lost candidate panel {id} for {cx_id}"
                            ))
                        })
                })
                .collect::<CliResult<BTreeMap<_, _>>>()?;
            rank_typed_panel_candidate_refs(
                vectors.as_ref(),
                &candidate_refs,
                request.embedding_slots,
            )
            .map_err(CliError::from)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .collect()
}

/// Persist the directed k-NN graph after slot-major candidate generation and
/// byte-bounded exact panel reranking.
pub(super) fn build_between_doc_graph<C: Clock>(
    vault: &AsterVault<C>,
    graph: &PlainGraph<'_, C>,
    request: BetweenDocGraphRequest<'_>,
    mut progress: Option<&mut dyn FnMut(BetweenDocProgress) -> CliResult>,
) -> CliResult<BetweenDocBuildResult> {
    if request.candidate_ordinals.len() != request.candidates.len() {
        return Err(CliError::runtime(format!(
            "between-doc candidate/ordinal length mismatch: {} vs {}",
            request.candidates.len(),
            request.candidate_ordinals.len()
        )));
    }
    if !request.candidate_domain_complete
        && request.candidates.len() > BOUNDED_EXACT_DOMAIN_MAX_NODES
    {
        return Err(CliError::runtime(format!(
            "bounded weave domain has {} nodes, exceeding exact-domain maximum {}; use --limit <= {} for bounded verification or --limit 0 for the complete production graph",
            request.candidates.len(),
            BOUNDED_EXACT_DOMAIN_MAX_NODES,
            BOUNDED_EXACT_DOMAIN_MAX_NODES
        )));
    }
    let mut builder = AssocGraph::builder();
    let node_set = request
        .candidates
        .iter()
        .map(|cx| cx.cx_id)
        .collect::<HashSet<_>>();
    for cx in request.candidates {
        builder
            .add_node(cx.cx_id, 1.0)
            .map_err(LodestarError::from)?;
    }
    let candidate_k = request
        .knn
        .checked_mul(request.embedding_slots.len())
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| CliError::runtime("panel graph candidate k overflow"))?;
    let (candidate_ids, mut telemetry) =
        generate_candidates(&request, &node_set, candidate_k, &mut progress)?;

    let mut panel_cache = PanelCache::new(PANEL_CACHE_MAX_BYTES);
    let mut edges_persisted = 0usize;
    let mut edge_rows = Vec::new();
    let mut edge_bytes = 0usize;
    let nodes_total = request.candidates.len();

    let mut batch_start = 0usize;
    while batch_start < nodes_total {
        let (batch_end, batch_bytes, panels) =
            load_exact_panel_batch(&mut panel_cache, &request, &candidate_ids, batch_start)?;
        let ranked =
            rank_exact_panel_batch(&request, &candidate_ids, &panels, batch_start, batch_end)?;
        telemetry.exact_rerank_batches += 1;
        telemetry.max_exact_rerank_batch_nodes = telemetry
            .max_exact_rerank_batch_nodes
            .max(batch_end - batch_start);
        telemetry.max_exact_rerank_batch_bytes =
            telemetry.max_exact_rerank_batch_bytes.max(batch_bytes);

        for (batch_offset, hits) in ranked.into_iter().enumerate() {
            let node_index = batch_start + batch_offset;
            let cx_id = request.candidates[node_index].cx_id;
            let mut kept = 0usize;
            for (fused_offset, hit) in hits.into_iter().enumerate() {
                if hit.score < request.edge_score_threshold {
                    continue;
                }
                if kept >= request.knn {
                    break;
                }
                let weight = hit.score.clamp(0.0, 1.0);
                let out_key = graph.edge_out_key(cx_id, EDGE_TYPE, hit.cx_id)?;
                let in_key = graph.edge_in_key(hit.cx_id, EDGE_TYPE, cx_id)?;
                let value = serde_json::to_vec(&EdgeValue {
                    weight,
                    fusion: "typed_block_rrf",
                    rrf_k: PANEL_RRF_K,
                    fused_rank: fused_offset + 1,
                    lanes: hit.lanes,
                })
                .map_err(|error| CliError::runtime(format!("serialize edge value: {error}")))?;
                edge_bytes += out_key.len() + value.len() + in_key.len() + out_key.len();
                edge_rows.push((ColumnFamily::Graph, out_key.clone(), value));
                edge_rows.push((ColumnFamily::Graph, in_key, out_key));
                builder
                    .add_edge(cx_id, hit.cx_id, weight)
                    .map_err(LodestarError::from)?;
                kept += 1;
                edges_persisted += 1;
            }
            if edge_rows.len() >= EDGE_FLUSH_ROWS || edge_bytes >= WEAVE_COMMIT_BYTE_BUDGET {
                vault.write_reconstructible_derived_cf_batch(std::mem::take(&mut edge_rows))?;
                edge_bytes = 0;
            }
            let nodes_processed = node_index + 1;
            if (nodes_processed == nodes_total || nodes_processed.is_multiple_of(128))
                && let Some(callback) = progress.as_mut()
            {
                callback(BetweenDocProgress {
                    stage: "exact_rerank",
                    nodes_total,
                    nodes_processed,
                    edges_persisted,
                    candidate_slots_total: request.embedding_slots.len(),
                    candidate_slots_processed: telemetry.candidate_slots,
                    candidate_batches: telemetry.candidate_batches,
                    candidate_queries: telemetry.candidate_queries,
                })?;
            }
        }
        batch_start = batch_end;
    }
    if !edge_rows.is_empty() {
        vault.write_reconstructible_derived_cf_batch(edge_rows)?;
    }
    telemetry.panel_cache = panel_cache.stats();

    Ok(BetweenDocBuildResult {
        edges_persisted,
        graph: builder.build(),
        telemetry,
    })
}
