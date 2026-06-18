//! Top-level search engine wiring SlotIndexMap to fusion.

use std::collections::BTreeMap;

use calyx_core::{
    CalyxError, Constellation, CxId, METADATA_TEMPORAL_LANE_STATE, Result, SlotId, SlotState,
    SlotVector, TEMPORAL_LANE_INACTIVE,
};

use crate::fusion::{self, FusionContext, FusionStrategy};
use crate::guarded::{GuardedSearchReport, apply_query_guard};
use crate::hit::{FreshnessTag, Hit, ProvenanceSource};
use crate::planner::QueryPlanner;
use crate::planner_explain::PlannerExplain;
use crate::query::{FreshnessRequirement, Query, QueryFilters};
use crate::query_admission::{QueryAdmissionConfig, QueryAdmissionController, QueryAdmissionStats};
use crate::reranker::{RerankCandidateText, RerankRequest, RerankerClient};
use crate::search_support::{
    anchor_filter_matches, default_strategy, metadata_matches, scalar_matches, strategy_weights,
    text_to_sparse,
};
use crate::slot_index_map::SlotIndexMap;
use crate::util::{event_time_secs_from_ts, hex32, stub_ledger};

const DEFAULT_PIPELINE_RECALL_MULTIPLIER: usize = 10;

#[derive(Clone, Default)]
pub struct SearchEngine {
    pub indexes: SlotIndexMap,
    docs: BTreeMap<CxId, Constellation>,
    query_admission: QueryAdmissionController,
    assoc_graph: Option<calyx_paths::AssocGraph>,
}

impl SearchEngine {
    pub fn new(indexes: SlotIndexMap) -> Self {
        Self {
            indexes,
            docs: BTreeMap::new(),
            query_admission: QueryAdmissionController::default(),
            assoc_graph: None,
        }
    }

    pub fn set_query_admission_config(&mut self, config: QueryAdmissionConfig) {
        self.query_admission = QueryAdmissionController::new(config);
    }

    pub fn query_admission_stats(&self) -> QueryAdmissionStats {
        self.query_admission.stats()
    }

    pub fn query_admission_metrics_text(&self) -> String {
        self.query_admission.metrics_text()
    }

    pub fn put_constellation(&mut self, constellation: Constellation) {
        self.docs.insert(constellation.cx_id, constellation);
    }

    pub fn constellation(&self, cx_id: CxId) -> Option<&Constellation> {
        self.docs.get(&cx_id)
    }

    /// Stored constellation ids in deterministic (sorted) order.
    pub fn constellation_ids(&self) -> Vec<CxId> {
        self.docs.keys().copied().collect()
    }

    /// Sets the vault association graph used by `navigation::traverse`.
    pub fn set_assoc_graph(&mut self, graph: calyx_paths::AssocGraph) {
        self.assoc_graph = Some(graph);
    }

    pub fn assoc_graph(&self) -> Option<&calyx_paths::AssocGraph> {
        self.assoc_graph.as_ref()
    }

    pub fn search(&self, query: &Query) -> Result<Vec<Hit>> {
        Ok(self.search_inner(query, None)?.hits)
    }

    pub fn search_with_reranker(
        &self,
        query: &Query,
        reranker: &RerankerClient,
    ) -> Result<Vec<Hit>> {
        Ok(self.search_inner(query, Some(reranker))?.hits)
    }

    pub fn search_with_guard_report(&self, query: &Query) -> Result<GuardedSearchReport> {
        self.search_inner(query, None)
    }

    pub fn planned_search(&self, query: Query, planner: &QueryPlanner) -> Result<Vec<Hit>> {
        let index_size = self.planner_index_size(&query);
        let plan = planner.plan(query, index_size)?;
        self.search(&plan.query)
    }

    pub fn planned_explain_search(
        &self,
        mut query: Query,
        planner: &QueryPlanner,
    ) -> Result<PlannerExplain> {
        query.explain = true;
        let index_size = self.planner_index_size(&query);
        let plan = planner.plan(query, index_size)?;
        let hits = self.search(&plan.query)?;
        Ok(PlannerExplain::new(&plan, hits))
    }

    fn search_inner(
        &self,
        query: &Query,
        reranker: Option<&RerankerClient>,
    ) -> Result<GuardedSearchReport> {
        let _query_permit = self.query_admission.acquire()?;
        query.validate()?;
        let slots = if query.slots.is_empty() {
            self.indexes.slots()
        } else {
            query.slots.clone()
        };
        if slots.is_empty() {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_NO_LENSES,
                "no registered slot indexes are available for search",
            ));
        }
        self.enforce_freshness(&slots, &query.freshness)?;
        let strategy = query
            .fusion
            .clone()
            .unwrap_or_else(|| default_strategy(&slots));
        if reranker.is_some() && !matches!(strategy, FusionStrategy::Pipeline) {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_QUERY_SHAPE,
                "reranker search requires Pipeline fusion",
            ));
        }
        let search_k = self.candidate_window(&slots, query, &strategy);
        let mut per_slot = BTreeMap::new();
        for slot in &slots {
            let stats = self
                .indexes
                .stats()
                .into_iter()
                .find(|stats| stats.slot == *slot)
                .ok_or_else(|| SlotIndexMap::missing_slot_error(*slot))?;
            let hits = if stats.kind == "inverted" {
                self.indexes.search_text(*slot, &query.text, search_k)?
            } else {
                let vector = self.query_vector_for_slot(query, *slot)?;
                self.indexes.search(*slot, &vector, search_k, query.ef)?
            };
            per_slot.insert(*slot, hits);
        }
        let weights = strategy_weights(&strategy);
        let stats = self.indexes.stats();
        let stage1_slots: Vec<SlotId> = slots
            .iter()
            .filter(|slot| {
                stats
                    .iter()
                    .any(|stats| stats.slot == **slot && stats.kind == "inverted")
            })
            .copied()
            .collect();
        let context = FusionContext {
            k: search_k,
            explain: query.explain,
            strategy: strategy.clone(),
            weights,
            stage1_slots: stage1_slots.clone(),
        };
        let mut hits = fusion::fuse(&per_slot, &context);
        self.apply_filters(&mut hits, &query.filters);
        if let Some(reranker) = reranker {
            self.rerank_pipeline_hits(query, &mut hits, &stage1_slots, reranker)?;
        }
        let dropped_guard_hits = apply_query_guard(&self.docs, query, &mut hits)?;
        hits.truncate(query.k);
        self.renumber_hits(&mut hits);
        self.attach_provenance_and_freshness(
            &mut hits,
            &slots,
            &query.freshness,
            query.require_stored_provenance,
        )?;
        Ok(GuardedSearchReport {
            hits,
            dropped_guard_hits,
        })
    }

    fn apply_filters(&self, hits: &mut Vec<Hit>, filters: &QueryFilters) {
        if filters.is_empty() {
            return;
        }
        hits.retain(|hit| {
            self.docs.get(&hit.cx_id).is_some_and(|cx| {
                filters
                    .scalars
                    .iter()
                    .all(|filter| scalar_matches(cx, filter))
                    && filters
                        .anchors
                        .iter()
                        .all(|filter| anchor_filter_matches(cx, filter))
                    && filters
                        .metadata
                        .iter()
                        .all(|filter| metadata_matches(cx, filter))
            })
        });
    }

    fn planner_index_size(&self, query: &Query) -> usize {
        let stats = self.indexes.stats();
        stats
            .iter()
            .filter(|stats| {
                if query.slots.contains(&stats.slot) {
                    return true;
                }
                query.slots.is_empty()
                    && matches!(self.indexes.slot_state(stats.slot), Ok(SlotState::Active))
            })
            .map(|stats| stats.len)
            .max()
            .unwrap_or(0)
    }

    fn candidate_window(
        &self,
        slots: &[SlotId],
        query: &Query,
        strategy: &FusionStrategy,
    ) -> usize {
        if query.guard.is_some() && query.filters.is_empty() {
            return query
                .recall_k
                .unwrap_or_else(|| query.k.saturating_mul(DEFAULT_PIPELINE_RECALL_MULTIPLIER))
                .max(query.k);
        }
        if query.filters.is_empty() {
            if matches!(strategy, FusionStrategy::Pipeline) {
                return query
                    .recall_k
                    .unwrap_or_else(|| query.k.saturating_mul(DEFAULT_PIPELINE_RECALL_MULTIPLIER));
            }
            return query.k;
        }
        self.indexes
            .stats()
            .into_iter()
            .filter(|stats| slots.contains(&stats.slot))
            .map(|stats| stats.len)
            .max()
            .unwrap_or(query.k)
            .max(query.k)
    }

    fn rerank_pipeline_hits(
        &self,
        query: &Query,
        hits: &mut [Hit],
        stage1_slots: &[SlotId],
        reranker: &RerankerClient,
    ) -> Result<()> {
        if hits.is_empty() {
            return Ok(());
        }
        let candidates = self.candidate_texts_for_hits(hits, stage1_slots)?;
        let response = reranker.rerank(&RerankRequest::from_candidate_texts(
            query.text.clone(),
            candidates,
        ))?;
        let mut scored = hits
            .iter()
            .cloned()
            .zip(response.scores)
            .enumerate()
            .collect::<Vec<_>>();
        scored.sort_by(
            |(left_order, (_, left_score)), (right_order, (_, right_score))| {
                right_score
                    .total_cmp(left_score)
                    .then_with(|| left_order.cmp(right_order))
            },
        );
        for (rank, (_, (mut hit, score))) in scored.into_iter().enumerate() {
            hit.score = score;
            hit.rank = rank + 1;
            if let Some(explain) = &mut hit.explain {
                explain.strategy = "pipeline+rerank".to_string();
                explain.per_lens_count = hit.per_lens.len();
                explain.provenance_hex = hex32(&hit.provenance.hash);
            }
            hits[rank] = hit;
        }
        Ok(())
    }

    fn candidate_texts_for_hits(
        &self,
        hits: &[Hit],
        stage1_slots: &[SlotId],
    ) -> Result<Vec<RerankCandidateText>> {
        if stage1_slots.is_empty() {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_RERANKER_NO_CANDIDATES,
                "pipeline rerank requires sparse stage-1 candidate text",
            ));
        }
        let mut texts = Vec::with_capacity(hits.len());
        for hit in hits {
            let mut text = None;
            for slot in stage1_slots {
                if let Some(candidate) = self.indexes.candidate_text(*slot, hit.cx_id)? {
                    text = Some(candidate);
                    break;
                }
            }
            texts.push(RerankCandidateText::new(text.ok_or_else(|| {
                crate::error::sextant_error(
                    crate::error::CALYX_SEXTANT_RERANKER_NO_CANDIDATES,
                    format!("candidate text missing for {}", hit.cx_id),
                )
            })?));
        }
        Ok(texts)
    }

    fn renumber_hits(&self, hits: &mut [Hit]) {
        for (idx, hit) in hits.iter_mut().enumerate() {
            hit.rank = idx + 1;
        }
    }

    fn query_vector_for_slot(&self, query: &Query, slot: SlotId) -> Result<SlotVector> {
        let stats = self
            .indexes
            .stats()
            .into_iter()
            .find(|stats| stats.slot == slot)
            .ok_or_else(|| SlotIndexMap::missing_slot_error(slot))?;
        if stats.kind == "inverted" {
            return Ok(text_to_sparse(&query.text));
        }
        query.vector.clone().ok_or_else(|| {
            crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                "dense or multi query vector required",
            )
        })
    }

    fn enforce_freshness(
        &self,
        slots: &[SlotId],
        requirement: &FreshnessRequirement,
    ) -> Result<()> {
        for slot in slots {
            let stats = self
                .indexes
                .stats()
                .into_iter()
                .find(|stats| stats.slot == *slot)
                .ok_or_else(|| SlotIndexMap::missing_slot_error(*slot))?;
            let stale_by = stats.base_seq.saturating_sub(stats.built_at_seq);
            match requirement {
                FreshnessRequirement::FreshDerived if stale_by > 0 => {
                    return Err(CalyxError::stale_derived(format!(
                        "slot {slot} stale by {stale_by} seq"
                    )));
                }
                FreshnessRequirement::StaleOk { seq_lag } if stale_by > *seq_lag => {
                    return Err(CalyxError::stale_derived(format!(
                        "slot {slot} stale by {stale_by} > lag {seq_lag}"
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn attach_provenance_and_freshness(
        &self,
        hits: &mut [Hit],
        slots: &[SlotId],
        freshness: &FreshnessRequirement,
        require_stored: bool,
    ) -> Result<()> {
        let stats = self.indexes.stats();
        let base = slots
            .iter()
            .filter_map(|slot| stats.iter().find(|stats| stats.slot == *slot))
            .fold((u64::MAX, 0), |(built, base), stats| {
                (built.min(stats.built_at_seq), base.max(stats.base_seq))
            });
        for hit in hits {
            if let Some(cx) = self.docs.get(&hit.cx_id) {
                hit.event_time_secs = if cx.metadata_value(METADATA_TEMPORAL_LANE_STATE)
                    == Some(TEMPORAL_LANE_INACTIVE)
                {
                    None
                } else {
                    cx.source_event_time_secs()
                        .or_else(|| event_time_secs_from_ts(cx.created_at))
                };
                hit.provenance = cx.provenance.clone();
                hit.provenance_source = ProvenanceSource::Stored;
            } else if require_stored {
                return Err(crate::error::sextant_error(
                    crate::error::CALYX_SEXTANT_PROVENANCE_MISSING,
                    format!("stored constellation missing for hit {}", hit.cx_id),
                ));
            } else {
                hit.provenance = stub_ledger(hit.cx_id, hit.rank as u64);
                hit.provenance_source = ProvenanceSource::Stub;
            }
            hit.freshness = match freshness {
                FreshnessRequirement::FreshDerived => FreshnessTag::fresh(base.1),
                FreshnessRequirement::StaleOk { .. } => FreshnessTag::stale_ok(base.0, base.1),
            };
            if let Some(explain) = &mut hit.explain {
                explain.provenance_hex = hex32(&hit.provenance.hash);
                explain.per_lens_count = hit.per_lens.len();
            }
        }
        Ok(())
    }
}
