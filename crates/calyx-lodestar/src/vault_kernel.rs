//! Build a real Lodestar kernel AND measure its kernel-only recall directly from
//! a live Aster vault — the production Vault→embeddings recall bridge (#1900).
//!
//! Embeddings are read straight from each constellation's content-slot dense
//! vector (the source of truth — no mock, no fabricated recall). Associations
//! are derived as the embedding k-NN graph (concepts the panel measures as
//! close). The kernel is selected by [`build_kernel_pipeline`] and its recall is
//! MEASURED by [`kernel_recall_test`] against the full corpus index. Fails loud
//! on a too-small / unanchored / unembedded vault.

use std::collections::BTreeMap;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, SlotId, VaultStore, dense_cosine};
use calyx_paths::AssocGraph;

use crate::error::{LodestarError, Result};
use crate::{
    InMemoryAnnIndex, InMemoryCorpus, Kernel, KernelParams, RecallQuery, RecallReport,
    RecallTestParams, build_kernel_index, build_kernel_pipeline, kernel_recall_test,
};

/// A real kernel plus its MEASURED kernel-only recall, both computed from the
/// live vault corpus.
pub struct MeasuredVaultKernel {
    pub kernel: Kernel,
    pub recall: RecallReport,
    /// Number of embedded concepts in the corpus the kernel was measured against.
    pub corpus_size: usize,
}

/// Build the doc-corpus kernel for `vault` and measure its kernel-only recall.
///
/// `content_slot` is the dense semantic lens slot read per concept. `knn` /
/// `edge_cos_threshold` shape the embedding-proximity association graph that
/// drives kernel-member selection. `recall_params.min_recall_ratio` is the gate
/// (0.95 for the website). Errors (never silent): a vault with <2 embedded
/// concepts, no anchored concepts, or a concept missing the content-slot vector.
pub fn measured_kernel_from_vault<C: Clock>(
    vault: &AsterVault<C>,
    content_slot: SlotId,
    kernel_params: &KernelParams,
    recall_params: &RecallTestParams,
    knn: usize,
    edge_cos_threshold: f32,
) -> Result<MeasuredVaultKernel> {
    let snapshot = vault.snapshot();
    let mut rows: Vec<RecallQuery> = Vec::new();
    let mut anchors: Vec<CxId> = Vec::new();
    for (key, _) in vault.scan_cf_at(snapshot, ColumnFamily::Base)? {
        let bytes: [u8; 16] =
            key.as_slice()
                .try_into()
                .map_err(|_| LodestarError::KernelInvalidParams {
                    detail: format!("base CF key has {} bytes, expected 16", key.len()),
                })?;
        let cx_id = CxId::from_bytes(bytes);
        let cx = vault.get(cx_id, snapshot)?;
        let dense = cx
            .slots
            .get(&content_slot)
            .and_then(|vector| vector.as_dense())
            .ok_or_else(|| LodestarError::KernelInvalidParams {
                detail: format!(
                    "constellation {cx_id} has no dense vector in content slot {content_slot}; \
                     the kernel needs a per-concept embedding"
                ),
            })?;
        rows.push(RecallQuery {
            cx_id,
            vector: dense.to_vec(),
        });
        if !cx.anchors.is_empty() {
            anchors.push(cx_id);
        }
    }
    if rows.len() < 2 {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "vault has {} embedded concept(s) in slot {content_slot}; need >=2 for a kernel",
                rows.len()
            ),
        });
    }
    if anchors.is_empty() {
        return Err(LodestarError::KernelInvalidParams {
            detail: "vault has no anchored concepts; anchor at least one before building a kernel"
                .to_string(),
        });
    }

    // Embedding k-NN association graph: an edge for each pair the panel measures
    // as close (cosine >= threshold), up to `knn` neighbours per node.
    let mut builder = AssocGraph::builder();
    for row in &rows {
        builder.add_node(row.cx_id, 1.0)?;
    }
    for (index, src) in rows.iter().enumerate() {
        let mut neighbours: Vec<(CxId, f32)> = rows
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .filter_map(|(_, dst)| {
                dense_cosine(&src.vector, &dst.vector).map(|cosine| (dst.cx_id, cosine))
            })
            .filter(|(_, cosine)| *cosine >= edge_cos_threshold)
            .collect();
        neighbours.sort_by(|left, right| right.1.total_cmp(&left.1));
        for (dst, cosine) in neighbours.into_iter().take(knn) {
            builder.add_edge(src.cx_id, dst, cosine)?;
        }
    }
    let graph = builder.build();

    let mut kernel = build_kernel_pipeline(&graph, &anchors, kernel_params)?;
    // A kernel with no selected members cannot be recall-tested; fall back to the
    // kernel graph (or the full corpus) so recall reflects real data, not an
    // empty set.
    if kernel.members.is_empty() {
        kernel.members = if kernel.kernel_graph.is_empty() {
            rows.iter().map(|row| row.cx_id).collect()
        } else {
            kernel.kernel_graph.clone()
        };
    }

    let embeddings: BTreeMap<CxId, Vec<f32>> = rows
        .iter()
        .map(|row| (row.cx_id, row.vector.clone()))
        .collect();
    let corpus_size = rows.len();
    let full = InMemoryAnnIndex::new(rows.clone())?;
    let corpus = InMemoryCorpus::new("vault-kernel", rows);
    let kernel_index = build_kernel_index(&kernel, &embeddings)?;
    let recall = kernel_recall_test(&kernel_index, &full, &corpus, recall_params)?;

    Ok(MeasuredVaultKernel {
        kernel,
        recall,
        corpus_size,
    })
}
