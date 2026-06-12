//! Bounded windowed recall for AP-60 temporal primary retrieval (issue #633).
//!
//! Window filtering is a post-retrieval predicate (event times attach after
//! fusion), so completeness requires retrieving enough fused candidates. This
//! module makes that budget explicit instead of silently expanding `k`:
//!
//! - [`WindowRecallPolicy::Exhaustive`] (default) retrieves up to the union
//!   bound (sum of primary slot lengths) — complete by construction, with the
//!   visited counts measured in [`WindowRecallReport`].
//! - [`WindowRecallPolicy::Bounded`] starts at the caller's recall budget and
//!   geometrically deepens (pgvector-style iterative scan) until `k` in-window
//!   rows are found, the corpus is exhausted, or `max_candidates` is hit —
//!   then it fails closed with `CALYX_TEMPORAL_WINDOW_BUDGET_EXHAUSTED`.

use calyx_core::{Result, SlotId};
use serde::{Deserialize, Serialize};

use crate::error::{
    CALYX_SEXTANT_QUERY_SHAPE, CALYX_TEMPORAL_WINDOW_BUDGET_EXHAUSTED, sextant_error,
};
use crate::hit::Hit;
use crate::query::Query;
use crate::search::SearchEngine;

use super::{TimeWindow, count_hits_in_window};

const DEEPEN_GROWTH_FACTOR: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowRecallPolicy {
    /// Retrieve up to the primary-slot union bound: complete by construction.
    #[default]
    Exhaustive,
    /// Iteratively deepen from the caller's recall budget up to
    /// `max_candidates`; fail closed if `k` in-window rows cannot be proven
    /// within the cap.
    Bounded { max_candidates: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotLen {
    pub slot: SlotId,
    pub len: usize,
}

/// Measured recall evidence for one windowed temporal retrieval.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowRecallReport {
    pub policy: WindowRecallPolicy,
    pub windowed: bool,
    pub requested_k: usize,
    pub requested_recall_k: Option<usize>,
    pub primary_slot_lens: Vec<SlotLen>,
    /// Sum of primary slot lengths: upper bound of the fused candidate union.
    pub union_bound: usize,
    pub effective_budget: usize,
    pub effective_ef: Option<usize>,
    pub candidates_fetched: usize,
    pub in_window_count: usize,
    pub corpus_exhausted: bool,
    pub rounds: u32,
}

/// Runs primary retrieval under an explicit window-recall policy and returns
/// the raw fused hits (pre window filter) plus the measured recall report.
pub(crate) fn windowed_primary_search(
    engine: &SearchEngine,
    primary_query: &Query,
    window: Option<&TimeWindow>,
    final_k: usize,
    policy: WindowRecallPolicy,
) -> Result<(Vec<Hit>, WindowRecallReport)> {
    let requested = primary_query.recall_k.unwrap_or(final_k).max(final_k);
    let primary_slot_lens = primary_slot_lens(engine, &primary_query.slots);
    let union_bound = primary_slot_lens
        .iter()
        .fold(0_usize, |sum, slot| sum.saturating_add(slot.len));
    let mut report = WindowRecallReport {
        policy,
        windowed: window.is_some(),
        requested_k: final_k,
        requested_recall_k: primary_query.recall_k,
        primary_slot_lens,
        union_bound,
        ..WindowRecallReport::default()
    };

    let Some(window) = window else {
        // No window means no post-retrieval predicate: the caller's own
        // budget is complete for its top-k contract.
        let (hits, ef) = search_round(engine, primary_query, requested)?;
        report.rounds = 1;
        report.effective_budget = requested;
        report.effective_ef = ef;
        report.candidates_fetched = hits.len();
        report.in_window_count = hits.len();
        report.corpus_exhausted = hits.len() < requested;
        return Ok((hits, report));
    };

    // `cap` is the ceiling we will never fetch past; `budget` is where this
    // round starts. Exhaustive pins both to the union bound (complete in one
    // round); Bounded starts at the caller's request and deepens up to the cap.
    let (cap, mut budget) = match policy {
        WindowRecallPolicy::Exhaustive => {
            let cap = union_bound.max(requested);
            (cap, cap)
        }
        WindowRecallPolicy::Bounded { max_candidates } => {
            if max_candidates < final_k {
                return Err(sextant_error(
                    CALYX_SEXTANT_QUERY_SHAPE,
                    format!(
                        "window recall max_candidates {max_candidates} is below requested k {final_k}"
                    ),
                ));
            }
            (max_candidates, requested.min(max_candidates))
        }
    };

    loop {
        report.rounds += 1;
        let (hits, ef) = search_round(engine, primary_query, budget)?;
        let in_window = count_hits_in_window(&hits, window);
        // Fewer rows than the budget means every fused candidate was fetched;
        // a budget at the union bound is complete by construction.
        let corpus_exhausted = hits.len() < budget || budget >= union_bound;
        report.effective_budget = budget;
        report.effective_ef = ef;
        report.candidates_fetched = hits.len();
        report.in_window_count = in_window;
        report.corpus_exhausted = corpus_exhausted;

        if in_window >= final_k || corpus_exhausted {
            return Ok((hits, report));
        }
        if budget >= cap {
            return Err(sextant_error(
                CALYX_TEMPORAL_WINDOW_BUDGET_EXHAUSTED,
                format!(
                    "windowed recall budget exhausted after {} rounds: needed {final_k} \
                     in-window rows, found {in_window} within budget {budget} \
                     (max_candidates {cap}, fetched {}, union bound {union_bound})",
                    report.rounds, report.candidates_fetched
                ),
            ));
        }
        let next = budget.saturating_mul(DEEPEN_GROWTH_FACTOR).min(cap);
        eprintln!(
            "CALYX_TEMPORAL_WINDOW_DEEPEN round={} budget={budget} next_budget={next} \
             fetched={} in_window={in_window} needed={final_k}",
            report.rounds, report.candidates_fetched
        );
        budget = next;
    }
}

/// One bounded retrieval round: the budget drives `k`, `recall_k`, and a
/// sufficient `ef` so per-slot candidate lists can actually reach the budget.
fn search_round(
    engine: &SearchEngine,
    primary_query: &Query,
    budget: usize,
) -> Result<(Vec<Hit>, Option<usize>)> {
    let mut query = primary_query.clone();
    query.k = budget;
    query.recall_k = Some(budget);
    query.ef = primary_query.ef.map(|ef| ef.max(budget));
    let hits = engine.search(&query)?;
    Ok((hits, query.ef))
}

fn primary_slot_lens(engine: &SearchEngine, slots: &[SlotId]) -> Vec<SlotLen> {
    engine
        .indexes
        .stats()
        .into_iter()
        .filter(|stats| slots.contains(&stats.slot))
        .map(|stats| SlotLen {
            slot: stats.slot,
            len: stats.len,
        })
        .collect()
}
