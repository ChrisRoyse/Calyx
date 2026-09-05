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
//! Memory (issue #1991): neither pass holds the decoded corpus resident. The
//! coverage scan streamed every measured vector's canonical bytes into an
//! out-of-core [`WeaveVectorScratch`] store keyed by scan ordinal; both passes
//! read one constellation's typed panel back on demand — within-doc per batched
//! node, between-doc per query node plus its bounded candidate set — so anon RSS
//! is `O(batch + per-node candidate set)`, never `O(corpus x slots)`. The bytes
//! read back are byte-for-byte the vectors the old in-heap path held, so the
//! persisted `SlotSimilarityEvidence` is unchanged (#1960).
//!
//! Every failure propagates (fail-closed): a constellation missing the content
//! vector, a compressed slot row, an absent DiskANN slot index — all hard-error
//! with the offending `cx_id`/slot named, never a silent skip or fabricated value.

use std::collections::BTreeMap;

use calyx_aster::cf::{ColumnFamily, weave_xterm_generation_key};
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, SlotId, SlotVector, slot_ambient_dim};
use calyx_lodestar::{
    AsterAssocNodeProps, LodestarError, PANEL_ASTER_ASSOC_COLLECTION, TypedPanelVectors,
    encode_assoc_node_props,
};
use calyx_loom::LoomStore;
use serde::Serialize;

use super::coverage::WeaveNodeMeta;
use super::scratch::WeaveVectorScratch;
use crate::error::CliResult;

const LOOM_CACHE_CAP: usize = 16;
const XTERM_WRITE_CHUNK: usize = 8192;
const NODE_WRITE_CHUNK: usize = 4096;
/// Cadence (in processed nodes) at which each pass emits a sub-phase
/// CALYX_WEAVE_RUNTIME line and samples live RSS against the budget, so residual
/// mid-pass growth is localized instead of dying silent between phase boundaries
/// (#1991 round-4).
pub(super) const WEAVE_PROGRESS_EVERY_NODES: usize = 4_096;
/// Primary flush budget for one weave-loom commit, in accumulated encoded row
/// bytes (key + value). v8 node rows embed full SegmentedMulti typed embeddings,
/// so a fixed row count is not a memory bound; this byte budget caps single-commit
/// memory (prod weave peaked at 54 GB RSS) and keeps one WAL commit comfortably
/// under the 64 MiB record-fragment cap so the common case never fragments. Row
/// counts remain as a secondary cap.
pub(super) const WEAVE_COMMIT_BYTE_BUDGET: usize = 32 * 1024 * 1024;

/// Flushes CF rows in commits bounded by both `WEAVE_COMMIT_BYTE_BUDGET`
/// (primary, on accumulated key+value bytes) and `max_rows` (secondary). A single
/// row larger than the budget becomes its own commit (and is fragmented by the
/// WAL if it exceeds the record cap), so no row is ever dropped or split here.
fn write_cf_byte_budgeted<C: Clock>(
    vault: &AsterVault<C>,
    rows: impl IntoIterator<Item = (ColumnFamily, Vec<u8>, Vec<u8>)>,
    max_rows: usize,
) -> CliResult<()> {
    let mut batch: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)> = Vec::new();
    let mut batch_bytes = 0usize;
    for row in rows {
        batch_bytes += row.1.len() + row.2.len();
        batch.push(row);
        if batch.len() >= max_rows || batch_bytes >= WEAVE_COMMIT_BYTE_BUDGET {
            vault.write_reconstructible_derived_cf_batch(std::mem::take(&mut batch))?;
            batch_bytes = 0;
        }
    }
    if !batch.is_empty() {
        vault.write_reconstructible_derived_cf_batch(batch)?;
    }
    Ok(())
}

/// Per-corpus aggregate of one content-lens pair's agreement (mean cosine over
/// every constellation that has both lenses, plus the observation count).
#[derive(Clone, Debug, Serialize)]
pub(super) struct SlotPairAgreement {
    pub a: u16,
    pub b: u16,
    pub mean_agreement: f32,
    pub n: usize,
}

/// Result of the within-doc weave pass. The typed panel is **not** returned
/// resident (that was the O(corpus) `panel_vectors` heap); between-doc re-reads
/// each node's panel from the out-of-core scratch store instead.
pub(super) struct WithinDocResult {
    pub constellations_in_vault: usize,
    pub constellations_processed: usize,
    pub xterm_rows_persisted: usize,
    pub agreement_pairs: Vec<SlotPairAgreement>,
    pub anchors: Vec<CxId>,
}

/// Sub-phase progress for the within-doc pass (#1991 round-4): drives mid-pass
/// RSS sampling + live budget enforcement so a killed run localizes growth.
#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct WithinDocProgress {
    pub nodes_total: usize,
    pub nodes_processed: usize,
    pub xterm_rows_persisted: usize,
}

fn data_error<T>(detail: String) -> CliResult<T> {
    Err(LodestarError::KernelInvalidParams { detail }.into())
}

/// Pass A: bulk-scan Base + content-slot CFs, weave within-doc agreement into the
/// XTerm CF, and write graph nodes. Each node's typed panel is fetched from the
/// out-of-core `scratch` store by its scan ordinal (#1991) rather than held
/// resident, so the working set is bounded by `batch`, not the corpus.
#[allow(clippy::too_many_arguments)]
pub(super) fn weave_within_doc<C: Clock>(
    vault: &AsterVault<C>,
    graph: &PlainGraph<'_, C>,
    candidates: &[WeaveNodeMeta],
    candidate_ordinals: &[u64],
    scratch: &WeaveVectorScratch,
    constellations_in_vault: usize,
    embedding_slots: &[SlotId],
    xterm_generation: &str,
    batch: usize,
    mut on_progress: Option<&mut dyn FnMut(WithinDocProgress) -> CliResult>,
) -> CliResult<WithinDocResult> {
    if candidates.len() < 2 {
        return data_error(format!(
            "weave-loom needs >=2 candidate constellations; candidate set has {}",
            candidates.len()
        ));
    }
    if candidate_ordinals.len() != candidates.len() {
        return data_error(format!(
            "weave-loom candidate/ordinal length mismatch: {} vs {}",
            candidates.len(),
            candidate_ordinals.len()
        ));
    }
    if embedding_slots.len() < 2 {
        return data_error("panel-native weave needs at least two typed content slots".to_string());
    }

    let mut xterm_rows_persisted = 0usize;
    let mut agreement_acc: BTreeMap<(u16, u16), (f64, usize)> = BTreeMap::new();
    let mut anchors: Vec<CxId> = Vec::new();
    let mut processed = 0usize;
    let mut last_progress = 0usize;
    let nodes_total = candidates.len();
    let batch = batch.max(1);

    let mut chunk_start = 0usize;
    while chunk_start < candidates.len() {
        let chunk_end = (chunk_start + batch).min(candidates.len());
        let mut loom = LoomStore::new(LOOM_CACHE_CAP);
        let mut node_rows: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)> =
            Vec::with_capacity(chunk_end - chunk_start);

        for index in chunk_start..chunk_end {
            let cx = &candidates[index];
            let cx_id = cx.cx_id;
            // Read this node's complete typed panel from the out-of-core store.
            // `read_panel` fails closed if any contracted slot is absent, exactly
            // like the old drain that refused partial constellations (#1991).
            let vectors: TypedPanelVectors = scratch
                .read_panel(candidate_ordinals[index], embedding_slots)
                .map_err(|error| LodestarError::KernelInvalidParams {
                    detail: format!(
                        "constellation {cx_id} panel read from scratch failed: {}",
                        error.message()
                    ),
                })?;

            // Agreement is defined only in a common ambient space. Preserve each
            // typed slot/block and weave every compatible pair without pooling.
            let mut by_dim: BTreeMap<u32, BTreeMap<SlotId, &SlotVector>> = BTreeMap::new();
            for (&slot, vector) in &vectors {
                by_dim
                    .entry(slot_ambient_dim(vector)?)
                    .or_default()
                    .insert(slot, vector);
            }
            for group in by_dim.values() {
                if group.len() < 2 {
                    continue;
                }
                loom.weave_typed_refs(cx_id, group).map_err(|error| {
                    LodestarError::KernelInvalidParams {
                        detail: format!("weave agreement for {cx_id} failed: {error}"),
                    }
                })?;
            }

            let props = AsterAssocNodeProps {
                embedding: None,
                embeddings: BTreeMap::new(),
                typed_embeddings: vectors,
                ts: Some(cx.created_at),
                anchors: cx.anchor_kinds.clone(),
                tenant: None,
                named_filters: Vec::new(),
                metadata: cx.metadata.clone(),
            };
            let encoded = encode_assoc_node_props(&props)?;
            node_rows.push((ColumnFamily::Graph, graph.node_key(cx_id), encoded));
            if !cx.anchor_kinds.is_empty() {
                anchors.push(cx_id);
            }
            processed += 1;
        }

        for edge in loom.agreement_graph()? {
            let entry = agreement_acc
                .entry((edge.a.get(), edge.b.get()))
                .or_default();
            entry.0 += f64::from(edge.raw_mean_agreement) * edge.n as f64;
            entry.1 += edge.n;
        }

        let kv = loom.xterm_kv_rows()?;
        xterm_rows_persisted += kv.len();
        let xterm_rows = kv
            .into_iter()
            .map(|(key, value)| {
                Ok((
                    ColumnFamily::XTerm,
                    weave_xterm_generation_key(
                        PANEL_ASTER_ASSOC_COLLECTION,
                        xterm_generation,
                        &key,
                    )?,
                    value,
                ))
            })
            .collect::<calyx_core::Result<Vec<_>>>()?;
        write_cf_byte_budgeted(vault, xterm_rows, XTERM_WRITE_CHUNK)?;

        write_cf_byte_budgeted(vault, node_rows, NODE_WRITE_CHUNK)?;
        chunk_start = chunk_end;

        // Emit a sub-phase runtime line + sample live RSS against the budget so
        // any residual mid-pass growth is localized, not silent.
        if processed - last_progress >= WEAVE_PROGRESS_EVERY_NODES
            && let Some(callback) = on_progress.as_mut()
        {
            callback(WithinDocProgress {
                nodes_total,
                nodes_processed: processed,
                xterm_rows_persisted,
            })?;
            last_progress = processed;
        }
    }

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
        constellations_processed: processed,
        xterm_rows_persisted,
        agreement_pairs,
        anchors,
    })
}
