use calyx_core::SlotId;
use serde::Serialize;

use super::ir::IrMetrics;
use super::panel::RealPanelSlot;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PanelRelevanceReport {
    pub(crate) dataset: String,
    pub(crate) corpus_docs: usize,
    pub(crate) stored_docs: usize,
    pub(crate) qrels_rows: usize,
    pub(crate) qrels_queries: usize,
    pub(crate) queries_evaluated: usize,
    pub(crate) skipped_queries_without_relevance: usize,
    pub(crate) queries_truncated_by_limit: usize,
    pub(crate) k: usize,
    pub(crate) panel_slots: Vec<RealPanelSlot>,
    pub(crate) final_snapshot: u64,
    pub(crate) provenance_ok: bool,
    pub(crate) strategy: String,
    pub(crate) metric_kind: String,
    pub(crate) best_single: SlotMetricReport,
    pub(crate) candidate_fusion: SlotMetricReport,
    pub(crate) fused: SlotMetricReport,
    pub(crate) fusion_minus_best_ndcg: f64,
    pub(crate) min_fusion_gain: f64,
    pub(crate) meets_fusion_gate: bool,
    pub(crate) query_evidence: Vec<RealQueryEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SlotMetricReport {
    pub(crate) label: String,
    pub(crate) slot: Option<SlotId>,
    pub(crate) lens: Option<String>,
    pub(crate) metrics: IrMetrics,
    pub(crate) hits_examined: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RealQueryEvidence {
    pub(crate) qid: String,
    pub(crate) relevant_docs: usize,
    pub(crate) rerank_candidates: usize,
    pub(crate) candidate_fusion_metrics: IrMetrics,
    pub(crate) fused_metrics: IrMetrics,
    pub(crate) fused_top_k: Vec<String>,
    pub(crate) singles: Vec<QuerySlotEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct QuerySlotEvidence {
    pub(crate) slot: SlotId,
    pub(crate) lens: String,
    pub(crate) metrics: IrMetrics,
    pub(crate) top_k: Vec<String>,
}
