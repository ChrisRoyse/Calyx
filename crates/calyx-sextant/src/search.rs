//! Top-level search engine wiring SlotIndexMap to fusion.

use std::collections::BTreeMap;

use calyx_core::{CalyxError, Constellation, CxId, Result, SlotId, SlotVector};

use crate::fusion::{self, FusionContext, FusionStrategy};
use crate::hit::{FreshnessTag, Hit};
use crate::query::{FreshnessRequirement, Query};
use crate::reranker::{RerankRequest, RerankerClient};
use crate::slot_index_map::SlotIndexMap;
use crate::util::{hex32, stub_ledger};

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
        let mut per_slot = BTreeMap::new();
        for slot in &slots {
            let stats = self
                .indexes
                .stats()
                .into_iter()
                .find(|stats| stats.slot == *slot)
                .ok_or_else(|| SlotIndexMap::missing_slot_error(*slot))?;
            let hits = if stats.kind == "inverted" {
                self.indexes.search_text(*slot, &query.text, query.k)?
            } else {
                let vector = self.query_vector_for_slot(query, *slot)?;
                self.indexes.search(*slot, &vector, query.k, query.ef)?
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
            k: query.k,
            explain: query.explain,
            strategy: strategy.clone(),
            weights,
            stage1_slots: stage1_slots.clone(),
        };
        let mut hits = fusion::fuse(&per_slot, &context);
        self.attach_provenance_and_freshness(&mut hits, &slots, &query.freshness);
        if let Some(reranker) = reranker {
            self.rerank_pipeline_hits(query, &mut hits, &stage1_slots, reranker)?;
        }
        Ok(hits)
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
    ) -> Result<Vec<String>> {
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
            texts.push(text.ok_or_else(|| {
                crate::error::sextant_error(
                    crate::error::CALYX_SEXTANT_RERANKER_TIMEOUT,
                    format!("candidate text missing for {}", hit.cx_id),
                )
            })?);
        }
        Ok(texts)
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
