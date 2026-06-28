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

use std::collections::{BTreeMap, HashMap, HashSet};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{Clock, Constellation, CxId, SlotId, SlotVector};
use calyx_lodestar::{AsterAssocNodeProps, LodestarError, encode_assoc_node_props};
use calyx_loom::LoomStore;
use calyx_paths::AssocGraph;
use serde::Serialize;

use super::super::PersistedSearchIndexes;
use crate::error::CliResult;

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
    snapshot: u64,
    content_slots: &[SlotId],
    knn_slot: SlotId,
    batch: usize,
    limit: usize,
) -> CliResult<WithinDocResult> {
    // 1. Sequential Base scan: cx order + anchors + metadata (slot vectors are
    //    left Absent in this decode — vectors come from the per-slot scans).
    let mut bases: Vec<Constellation> = Vec::new();
    for (_, value) in vault.scan_cf_at(snapshot, ColumnFamily::Base)? {
        bases.push(encode::decode_constellation_base(&value)?);
    }
    let constellations_in_vault = bases.len();
    if constellations_in_vault < 2 {
        return data_error(format!(
            "weave-loom needs >=2 constellations; vault has {constellations_in_vault}"
        ));
    }
    if limit > 0 && limit < bases.len() {
        bases.truncate(limit);
    }
    let wanted: HashSet<CxId> = bases.iter().map(|cx| cx.cx_id).collect();

    // 2. Sequential per-slot scans: dense vector per wanted cx for each content
    //    lens. Fails closed on a compressed slot row (needs a compression-aware
    //    path); skips genuinely-Absent rows.
    let mut slot_maps: BTreeMap<SlotId, HashMap<CxId, Vec<f32>>> = BTreeMap::new();
    for &slot in content_slots {
        let mut map: HashMap<CxId, Vec<f32>> = HashMap::new();
        for (key, value) in vault.scan_cf_at(snapshot, ColumnFamily::slot(slot))? {
            let cx_id = cx_from_key(&key)?;
            if !wanted.contains(&cx_id) {
                continue;
            }
            if let Some(dense) = decode_dense(slot, cx_id, &value)? {
                map.insert(cx_id, dense);
            }
        }
        slot_maps.insert(slot, map);
    }
    let knn_map = slot_maps
        .get(&knn_slot)
        .ok_or_else(|| LodestarError::KernelInvalidParams {
            detail: format!("content slot {knn_slot} was not scanned"),
        })?;

    // 3. Weave per constellation, batched for XTerm/node persistence.
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
            for (&slot, map) in &slot_maps {
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

        for edge in loom.agreement_graph() {
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

fn cx_from_key(key: &[u8]) -> CliResult<CxId> {
    let bytes: [u8; 16] = key
        .try_into()
        .map_err(|_| LodestarError::KernelInvalidParams {
            detail: format!("slot CF key has {} bytes, expected 16", key.len()),
        })?;
    Ok(CxId::from_bytes(bytes))
}

/// Decode a slot CF row to its dense vector. `None` for a genuinely-Absent slot
/// (the lens did not measure this constellation); hard error on a compressed or
/// otherwise non-dense/corrupt row (fail closed, never a silent skip).
fn decode_dense(slot: SlotId, cx_id: CxId, value: &[u8]) -> CliResult<Option<Vec<f32>>> {
    match encode::decode_slot_vector(value) {
        Ok(SlotVector::Dense { data, .. }) => Ok(Some(data)),
        Ok(SlotVector::Absent { .. }) => Ok(None),
        Ok(_) => data_error(format!(
            "slot {slot} row for {cx_id} is not dense; weave-loom requires dense content lenses"
        )),
        Err(error) => Err(error.into()),
    }
}

/// Pass B: build the directed k-NN association graph over the persisted DiskANN
/// index, persist its edges into the `graph` CF, and return the in-memory
/// `AssocGraph` (cosine-weighted, clamped to `[0,1]`) for the acceptance report.
pub(super) fn build_between_doc_graph<C: Clock>(
    vault: &AsterVault<C>,
    graph: &PlainGraph<'_, C>,
    indexes: &PersistedSearchIndexes,
    knn_slot: SlotId,
    knn: usize,
    edge_cos_threshold: f32,
    knn_vectors: &[(CxId, Vec<f32>)],
) -> CliResult<(usize, AssocGraph)> {
    let mut builder = AssocGraph::builder();
    let node_set: HashSet<CxId> = knn_vectors.iter().map(|(cx_id, _)| *cx_id).collect();
    for (cx_id, _) in knn_vectors {
        builder.add_node(*cx_id, 1.0).map_err(LodestarError::from)?;
    }

    let mut edges_persisted = 0usize;
    let mut edge_rows: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)> = Vec::new();

    for (cx_id, vector) in knn_vectors {
        let query = SlotVector::Dense {
            dim: vector.len() as u32,
            data: vector.clone(),
        };
        let hits = indexes.search(knn_slot, &query, knn + 1)?;
        let mut kept = 0usize;
        for hit in hits {
            // Skip self, sub-threshold, and any neighbour outside the processed
            // node set (only possible under `--limit`; the full corpus run keeps
            // every neighbour). Guarantees every edge endpoint has a graph node.
            if hit.cx_id == *cx_id
                || hit.score < edge_cos_threshold
                || !node_set.contains(&hit.cx_id)
            {
                continue;
            }
            if kept >= knn {
                break;
            }
            let cosine = hit.score.clamp(0.0, 1.0);
            let out_key = graph.edge_out_key(*cx_id, EDGE_TYPE, hit.cx_id)?;
            let in_key = graph.edge_in_key(hit.cx_id, EDGE_TYPE, *cx_id)?;
            let value = serde_json::to_vec(&EdgeValue {
                cosine: hit.score,
                rank: hit.rank,
            })?;
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
    }
    if !edge_rows.is_empty() {
        vault.write_cf_batch(edge_rows)?;
    }
    vault.flush()?;

    Ok((edges_persisted, builder.build()))
}
