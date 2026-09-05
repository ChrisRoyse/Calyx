use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, SlotId};
use calyx_lodestar::{AsterAssocMetadata, PANEL_RRF_K, write_assoc_metadata};
use serde::Serialize;

use super::WeaveLoomArgs;

#[derive(Clone, Debug, Serialize)]
pub(super) struct GraphContract {
    embedding_slots: Vec<SlotId>,
    fusion: &'static str,
    rrf_k: u32,
    panel_version: u64,
    graph_source_seq: u64,
    knn: usize,
    edge_score_threshold: f32,
    /// Candidate-generation device pinned in provenance (#1963). Candidate sets
    /// may differ CPU-vs-GPU (both valid under the recall contract); the
    /// per-pair similarity evidence is bit-identical regardless.
    candidate_device: &'static str,
}

impl GraphContract {
    pub(super) fn from_args(
        embedding_slots: Vec<SlotId>,
        panel_version: u64,
        graph_source_seq: u64,
        args: &WeaveLoomArgs,
    ) -> Self {
        Self {
            embedding_slots,
            fusion: "typed_block_rrf",
            rrf_k: PANEL_RRF_K,
            panel_version,
            graph_source_seq,
            knn: args.knn,
            edge_score_threshold: args.edge_score_threshold,
            candidate_device: args.candidate_device.as_str(),
        }
    }

    pub(super) fn persist<C: Clock>(
        &self,
        vault: &AsterVault<C>,
        storage_collection: &str,
    ) -> Result<(), CalyxError> {
        write_assoc_metadata(
            vault,
            storage_collection,
            &AsterAssocMetadata {
                retention_horizon: None,
                embedding_slot: None,
                embedding_slots: self.embedding_slots.clone(),
                fusion: Some(self.fusion.to_string()),
                rrf_k: Some(self.rrf_k),
                panel_version: Some(self.panel_version),
                graph_source_seq: Some(self.graph_source_seq),
                knn: Some(self.knn),
                edge_cos_threshold: None,
                edge_score_threshold: Some(self.edge_score_threshold),
            },
        )
        .map(|_| ())
    }
}
