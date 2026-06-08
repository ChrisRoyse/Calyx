//! Top-level search engine wiring SlotIndexMap to fusion.

use std::collections::BTreeMap;

use calyx_core::{Anchor, CalyxError, Constellation, CxId, Result, SlotId, SlotState, SlotVector};
use zeroize::Zeroizing;

use crate::fusion::{self, FusionContext, FusionStrategy};
use crate::hit::{FreshnessTag, Hit};
use crate::planner::QueryPlanner;
use crate::planner_explain::PlannerExplain;
use crate::query::{
    AnchorPredicate, FreshnessRequirement, MetadataPredicate, Query, QueryFilters, ScalarOp,
    ScalarPredicate,
};
use crate::reranker::{RerankRequest, RerankerClient};
use crate::slot_index_map::SlotIndexMap;
use crate::util::{hex32, stub_ledger};

const DEFAULT_PIPELINE_RECALL_MULTIPLIER: usize = 10;

#[derive(Clone, Default)]
pub struct SearchEngine {
    pub indexes: SlotIndexMap,
    docs: BTreeMap<CxId, Constellation>,
}

impl SearchEngine {
    pub fn new(indexes: SlotIndexMap) -> Self {
        Self {
            indexes,
            docs: BTreeMap::new(),
        }
    }

    pub fn put_constellation(&mut self, constellation: Constellation) {
        self.docs.insert(constellation.cx_id, constellation);
    }

    pub fn constellation(&self, cx_id: CxId) -> Option<&Constellation> {
        self.docs.get(&cx_id)
    }

    pub fn search(&self, query: &Query) -> Result<Vec<Hit>> {
        self.search_inner(query, None)
    }

    pub fn search_with_reranker(
        &self,
        query: &Query,
        reranker: &RerankerClient,
    ) -> Result<Vec<Hit>> {
        self.search_inner(query, Some(reranker))
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

    fn search_inner(&self, query: &Query, reranker: Option<&RerankerClient>) -> Result<Vec<Hit>> {
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
                crate::error::CALYX_SEXTANT_RERANKER_TIMEOUT,
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
        hits.truncate(query.k);
        self.renumber_hits(&mut hits);
        self.attach_provenance_and_freshness(&mut hits, &slots, &query.freshness);
        Ok(hits)
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
        let response = reranker.rerank(&RerankRequest {
            query: query.text.clone(),
            candidates,
        })?;
        if !response.zeroizing_ok {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_RERANKER_TIMEOUT,
                "reranker did not report request-scoped candidate handling",
            ));
        }
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
    ) -> Result<Vec<Zeroizing<String>>> {
        if stage1_slots.is_empty() {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_RERANKER_TIMEOUT,
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
            texts.push(Zeroizing::new(text.ok_or_else(|| {
                crate::error::sextant_error(
                    crate::error::CALYX_SEXTANT_RERANKER_TIMEOUT,
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
    ) {
        let stats = self.indexes.stats();
        let base = slots
            .iter()
            .filter_map(|slot| stats.iter().find(|stats| stats.slot == *slot))
            .fold((u64::MAX, 0), |(built, base), stats| {
                (built.min(stats.built_at_seq), base.max(stats.base_seq))
            });
        for hit in hits {
            hit.provenance = self
                .docs
                .get(&hit.cx_id)
                .map(|cx| cx.provenance.clone())
                .unwrap_or_else(|| stub_ledger(hit.cx_id, hit.rank as u64));
            hit.freshness = match freshness {
                FreshnessRequirement::FreshDerived => FreshnessTag::fresh(base.1),
                FreshnessRequirement::StaleOk { .. } => FreshnessTag::stale_ok(base.0, base.1),
            };
            if let Some(explain) = &mut hit.explain {
                explain.provenance_hex = hex32(&hit.provenance.hash);
                explain.per_lens_count = hit.per_lens.len();
            }
        }
    }
}

fn scalar_matches(cx: &Constellation, filter: &ScalarPredicate) -> bool {
    cx.scalars
        .get(&filter.name)
        .is_some_and(|actual| compare_scalar(*actual, filter.op, filter.value))
}

fn compare_scalar(actual: f64, op: ScalarOp, expected: f64) -> bool {
    if !actual.is_finite() || !expected.is_finite() {
        return false;
    }
    match op {
        ScalarOp::Eq => actual == expected,
        ScalarOp::Gt => actual > expected,
        ScalarOp::Gte => actual >= expected,
        ScalarOp::Lt => actual < expected,
        ScalarOp::Lte => actual <= expected,
    }
}

fn anchor_filter_matches(cx: &Constellation, filter: &AnchorPredicate) -> bool {
    cx.anchors
        .iter()
        .any(|anchor| anchor_matches(anchor, filter))
}

fn anchor_matches(anchor: &Anchor, filter: &AnchorPredicate) -> bool {
    if anchor.kind != filter.kind {
        return false;
    }
    if let Some(value) = &filter.value
        && &anchor.value != value
    {
        return false;
    }
    if let Some(min_confidence) = filter.min_confidence
        && (!min_confidence.is_finite() || anchor.confidence < min_confidence)
    {
        return false;
    }
    if let Some(source) = &filter.source
        && &anchor.source != source
    {
        return false;
    }
    true
}

fn metadata_matches(cx: &Constellation, filter: &MetadataPredicate) -> bool {
    match filter {
        MetadataPredicate::Vault(vault) => &cx.vault_id == vault,
        MetadataPredicate::Modality(modality) => &cx.modality == modality,
        MetadataPredicate::PanelVersion(panel_version) => cx.panel_version == *panel_version,
        MetadataPredicate::CreatedAt { min, max } => {
            min.is_none_or(|value| cx.created_at >= value)
                && max.is_none_or(|value| cx.created_at <= value)
        }
        MetadataPredicate::InputRedacted(redacted) => cx.input_ref.redacted == *redacted,
        MetadataPredicate::InputPointerContains(fragment) => cx
            .input_ref
            .pointer
            .as_ref()
            .is_some_and(|pointer| pointer.contains(fragment)),
    }
}

fn default_strategy(slots: &[SlotId]) -> FusionStrategy {
    if slots.len() == 1 {
        FusionStrategy::SingleLens { slot: slots[0] }
    } else {
        FusionStrategy::Rrf
    }
}

fn strategy_weights(strategy: &FusionStrategy) -> BTreeMap<SlotId, f32> {
    match strategy {
        FusionStrategy::WeightedRrf { profile } => crate::fusion::profiles::lookup(*profile)
            .map(|profile| profile.weights)
            .unwrap_or_default(),
        _ => BTreeMap::new(),
    }
}

fn text_to_sparse(text: &str) -> SlotVector {
    SlotVector::Sparse {
        dim: 1_000_000,
        entries: crate::index::tokenizer::tokenize(text)
            .into_iter()
            .enumerate()
            .map(|(idx, _)| calyx_core::SparseEntry {
                idx: idx as u32,
                val: 1.0,
            })
            .collect(),
    }
}
