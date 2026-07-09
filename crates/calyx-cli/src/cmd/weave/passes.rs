//! Corpus `weave-loom` execution passes (#870).
//!
//! Pass A ([`weave_within_doc`]) reads the corpus with **sequential bulk scans**
//! (one Base-CF scan for anchors/metadata + one scan per content-slot CF for
//! vectors) rather than a random per-document `get` — at 199k constellations the
//! per-doc path is disk-bound and intractable, the per-slot sequential path is a
//! handful of streaming reads. It then weaves within-doc cross-lens **agreement**
//! cross-terms (grouped by vector dimension, since cosine agreement is only
//! defined between equal-dimension lenses) into the XTerm CF, and writes the
//! between-doc graph **node** (props = content-slot embedding + anchor kinds +
//! metadata) into the `graph` CF.
//!
//! Pass B ([`build_between_doc_graph`]) uses the persisted DiskANN index to find
//! each node's top-k nearest neighbours (panel-measured proximity) and writes the
//! directed k-NN **edges** into the `graph` CF, returning the in-memory
//! `AssocGraph` the acceptance report is measured over.
//!
//! Every failure propagates (fail-closed): a constellation missing the content
//! vector, a compressed slot row, an absent DiskANN slot index — all hard-error
//! with the offending `cx_id`/slot named, never a silent skip or fabricated value.

use std::collections::{BTreeMap, HashSet};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, SlotId, SlotVector};
use calyx_lodestar::{AsterAssocNodeProps, LodestarError, encode_assoc_node_props};
use calyx_loom::LoomStore;
use calyx_paths::AssocGraph;
use serde::Serialize;

use super::super::PersistedSearchIndexes;
use super::coverage::DenseSlotPreflight;
use crate::error::{CliError, CliResult};

pub(super) const EDGE_TYPE: &str = "knn";
const LOOM_CACHE_CAP: usize = 16;
const XTERM_WRITE_CHUNK: usize = 8192;
const NODE_WRITE_CHUNK: usize = 4096;
const EDGE_FLUSH_ROWS: usize = 8192;

/// Per-corpus aggregate of one content-lens pair's agreement (mean cosine over
/// every constellation that has both lenses, plus the observation count).
#[derive(Clone, Debug, Serialize)]
pub(super) struct SlotPairAgreement {
    pub a: u16,
    pub b: u16,
    pub mean_agreement: f32,
    pub n: usize,
}

/// Result of the within-doc weave pass.
pub(super) struct WithinDocResult {
    pub constellations_in_vault: usize,
    pub constellations_processed: usize,
    pub xterm_rows_persisted: usize,
    pub agreement_pairs: Vec<SlotPairAgreement>,
    pub anchors: Vec<CxId>,
    /// `(cx_id, content-slot embedding)` for every node, in scan order — the
    /// Pass-B k-NN query set. Held in memory (one dense vector per node).
    pub knn_vectors: Vec<(CxId, Vec<f32>)>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct BetweenDocProgress {
    pub nodes_total: usize,
    pub nodes_processed: usize,
    pub edges_persisted: usize,
}

pub(super) struct BetweenDocGraphRequest<'a> {
    pub indexes: &'a PersistedSearchIndexes,
    pub knn_slot: SlotId,
    pub knn: usize,
    pub edge_cos_threshold: f32,
    pub knn_vectors: &'a [(CxId, Vec<f32>)],
}

#[derive(Serialize)]
struct EdgeValue {
    cosine: f32,
    rank: usize,
}

fn data_error<T>(detail: String) -> CliResult<T> {
    Err(LodestarError::KernelInvalidParams { detail }.into())
}

/// Pass A: bulk-scan Base + content-slot CFs, weave within-doc agreement into the
/// XTerm CF, write graph nodes, and collect the Pass-B k-NN query vectors.
pub(super) fn weave_within_doc<C: Clock>(
    vault: &AsterVault<C>,
    graph: &PlainGraph<'_, C>,
    preflight: &DenseSlotPreflight,
    knn_slot: SlotId,
    batch: usize,
) -> CliResult<WithinDocResult> {
    let bases = &preflight.candidates;
    let constellations_in_vault = preflight.constellations_in_vault;
    if bases.len() < 2 {
        return data_error(format!(
            "weave-loom needs >=2 candidate constellations; candidate set has {}",
            bases.len()
        ));
    }
    let slot_maps = &preflight.slot_maps;
    let knn_map = slot_maps
        .get(&knn_slot)
        .ok_or_else(|| LodestarError::KernelInvalidParams {
            detail: format!("content slot {knn_slot} was not scanned"),
        })?;

    // Weave per constellation, batched for XTerm/node persistence.
    let mut xterm_rows_persisted = 0usize;
    let mut agreement_acc: BTreeMap<(u16, u16), (f64, usize)> = BTreeMap::new();
    let mut anchors: Vec<CxId> = Vec::new();
    let mut knn_vectors: Vec<(CxId, Vec<f32>)> = Vec::with_capacity(bases.len());

    for chunk in bases.chunks(batch.max(1)) {
        let mut loom = LoomStore::new(LOOM_CACHE_CAP);
        let mut node_rows: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)> = Vec::with_capacity(chunk.len());

        for cx in chunk {
            let cx_id = cx.cx_id;
            let knn_vec =
                knn_map
                    .get(&cx_id)
                    .cloned()
                    .ok_or_else(|| LodestarError::KernelInvalidParams {
                        detail: format!(
                            "constellation {cx_id} has no dense vector in content slot {knn_slot}; \
                         the between-doc graph needs a per-node embedding"
                        ),
                    })?;

            // Agreement is defined only between equal-dimension lenses; weave each
            // dimension group independently.
            let mut by_dim: BTreeMap<usize, BTreeMap<SlotId, Vec<f32>>> = BTreeMap::new();
            for (&slot, map) in slot_maps {
                if let Some(vector) = map.get(&cx_id) {
                    by_dim
                        .entry(vector.len())
                        .or_default()
                        .insert(slot, vector.clone());
                }
            }
            for group in by_dim.values() {
                if group.len() < 2 {
                    continue;
                }
                loom.weave(cx_id, group)
                    .map_err(|error| LodestarError::KernelInvalidParams {
                        detail: format!("weave agreement for {cx_id} failed: {error}"),
                    })?;
            }

            let props = AsterAssocNodeProps {
                embedding: Some(knn_vec.clone()),
                ts: Some(cx.created_at),
                anchors: cx
                    .anchors
                    .iter()
                    .map(|anchor| anchor.kind.clone())
                    .collect(),
                tenant: None,
                named_filters: Vec::new(),
                metadata: cx.metadata.clone(),
            };
            node_rows.push((
                ColumnFamily::Graph,
                graph.node_key(cx_id),
                encode_assoc_node_props(&props)?,
            ));
            if !cx.anchors.is_empty() {
                anchors.push(cx_id);
            }
            knn_vectors.push((cx_id, knn_vec));
        }

        for edge in loom.agreement_graph()? {
            let entry = agreement_acc
                .entry((edge.a.get(), edge.b.get()))
                .or_default();
            entry.0 += f64::from(edge.raw_mean_agreement) * edge.n as f64;
            entry.1 += edge.n;
        }

        let kv = loom.xterm_kv_rows()?;
        for rows in kv.chunks(XTERM_WRITE_CHUNK) {
            vault.write_cf_batch(
                rows.iter()
                    .map(|(k, v)| (ColumnFamily::XTerm, k.clone(), v.clone())),
            )?;
        }
        xterm_rows_persisted += kv.len();

        for rows in node_rows.chunks(NODE_WRITE_CHUNK) {
            vault.write_cf_batch(rows.iter().cloned())?;
        }
    }

    vault.flush()?;

    let agreement_pairs = agreement_acc
        .into_iter()
        .map(|((a, b), (sum, n))| SlotPairAgreement {
            a,
            b,
            mean_agreement: (sum / n.max(1) as f64) as f32,
            n,
        })
        .collect();

    Ok(WithinDocResult {
        constellations_in_vault,
        constellations_processed: knn_vectors.len(),
        xterm_rows_persisted,
        agreement_pairs,
        anchors,
        knn_vectors,
    })
}

/// Pass B: build the directed k-NN association graph over the persisted DiskANN
/// index, persist its edges into the `graph` CF, and return the in-memory
/// `AssocGraph` (cosine-weighted, clamped to `[0,1]`) for the acceptance report.
pub(super) fn build_between_doc_graph<C: Clock>(
    vault: &AsterVault<C>,
    graph: &PlainGraph<'_, C>,
    request: BetweenDocGraphRequest<'_>,
    mut progress: Option<&mut dyn FnMut(BetweenDocProgress) -> CliResult>,
) -> CliResult<(usize, AssocGraph)> {
    let mut builder = AssocGraph::builder();
    let node_set: HashSet<CxId> = request
        .knn_vectors
        .iter()
        .map(|(cx_id, _)| *cx_id)
        .collect();
    for (cx_id, _) in request.knn_vectors {
        builder.add_node(*cx_id, 1.0).map_err(LodestarError::from)?;
    }

    let mut edges_persisted = 0usize;
    let mut edge_rows: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)> = Vec::new();

    let nodes_total = request.knn_vectors.len();
    for (node_index, (cx_id, vector)) in request.knn_vectors.iter().enumerate() {
        let query = SlotVector::Dense {
            dim: vector.len() as u32,
            data: vector.clone(),
        };
        let hits = request
            .indexes
            .search(request.knn_slot, &query, request.knn + 1)?;
        let mut kept = 0usize;
        for hit in hits {
            // Skip self, sub-threshold, and any neighbour outside the processed
            // node set (only possible under `--limit`; the full corpus run keeps
            // every neighbour). Guarantees every edge endpoint has a graph node.
            if hit.cx_id == *cx_id
                || hit.score < request.edge_cos_threshold
                || !node_set.contains(&hit.cx_id)
            {
                continue;
            }
            if kept >= request.knn {
                break;
            }
            let cosine = hit.score.clamp(0.0, 1.0);
            let out_key = graph.edge_out_key(*cx_id, EDGE_TYPE, hit.cx_id)?;
            let in_key = graph.edge_in_key(hit.cx_id, EDGE_TYPE, *cx_id)?;
            let value = serde_json::to_vec(&EdgeValue {
                cosine: hit.score,
                rank: hit.rank,
            })
            .map_err(|error| CliError::runtime(format!("serialize edge value: {error}")))?;
            edge_rows.push((ColumnFamily::Graph, out_key.clone(), value));
            edge_rows.push((ColumnFamily::Graph, in_key, out_key));
            builder
                .add_edge(*cx_id, hit.cx_id, cosine)
                .map_err(LodestarError::from)?;
            kept += 1;
            edges_persisted += 1;
        }
        if edge_rows.len() >= EDGE_FLUSH_ROWS {
            vault.write_cf_batch(std::mem::take(&mut edge_rows))?;
        }
        let nodes_processed = node_index + 1;
        if (nodes_processed == nodes_total || nodes_processed % 128 == 0)
            && let Some(callback) = progress.as_mut()
        {
            callback(BetweenDocProgress {
                nodes_total,
                nodes_processed,
                edges_persisted,
            })?;
        }
    }
    if !edge_rows.is_empty() {
        vault.write_cf_batch(edge_rows)?;
    }
    vault.flush()?;

    Ok((edges_persisted, builder.build()))
}
