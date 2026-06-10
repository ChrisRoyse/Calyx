use std::collections::BTreeMap;

use calyx_core::SlotId;

use super::FusionContext;
use crate::hit::{FreshnessTag, Hit, PerLensContribution, ProvenanceSource};
use crate::index::IndexSearchHit;
use crate::util::stub_ledger;

pub fn single_lens_fuse(
    slot: SlotId,
    results: &BTreeMap<SlotId, Vec<IndexSearchHit>>,
    context: &FusionContext,
) -> Vec<Hit> {
    let Some(items) = results.get(&slot) else {
        return Vec::new();
    };
    let mut hits: Vec<_> = items
        .iter()
        .take(context.k)
        .map(|item| Hit {
            cx_id: item.cx_id,
            score: item.score,
            rank: item.rank,
            event_time_secs: None,
            temporal_scores: None,
            causal_confidence: crate::temporal::CausalConfidence::Absent,
            causal_gate: None,
            per_lens: vec![PerLensContribution {
                slot,
                rank: item.rank,
                raw_score: item.score,
                weight: 1.0,
                contribution: item.score,
            }],
            cross_terms_used: false,
            guard: None,
            provenance: stub_ledger(item.cx_id, item.rank as u64),
            provenance_source: ProvenanceSource::Stub,
            freshness: FreshnessTag::fresh(0),
            explain: None,
        })
        .collect();
    if context.explain {
        for hit in &mut hits {
            *hit = hit.clone().with_explain("single_lens");
        }
    }
    hits
}
