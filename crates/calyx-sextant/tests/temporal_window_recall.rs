//! Issue #633 — multi-primary windowed temporal_search must be complete and
//! bounded by an explicit recall policy.
//!
//! Fixture (hand-computed): two disjoint primary slots, five docs each.
//! Slot A holds seeds 1..=5, slot B holds seeds 11..=15. Per-slot HNSW
//! ranking is by descending cosine, so slot A ranks 1,2,3,4,5 and slot B
//! ranks 11,12,13,14,15. Every doc appears in exactly one slot list, so its
//! RRF score is 1/(rank+60); equal-rank pairs tie and break by ascending hex
//! id ("01.." < "0b.."). Hand-computed fused order:
//!   1, 11, 2, 12, 3, 13, 4, 14, 5, 15
//! Only seed 15 is inside the query window and it sits at fused position 10 —
//! past the old max-slot-len budget of 5, which silently dropped it.

use std::collections::BTreeMap;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, DecayFunction, InputRef, LedgerRef, Modality,
    PeriodicOptions, SlotId, VaultId,
};
use calyx_sextant::{
    CALYX_SEXTANT_QUERY_SHAPE, CALYX_TEMPORAL_WINDOW_BUDGET_EXHAUSTED, Hit, HnswIndex, Query,
    SearchEngine, SlotIndexMap, TemporalFixedClock, TemporalPolicy, TimeWindow, WindowRecallPolicy,
    temporal_search, temporal_search_with_recall,
};
use sextant_support::{cx_u8_fill as cx, dense};

const SLOT_A: SlotId = SlotId::new(8);
const SLOT_B: SlotId = SlotId::new(9);
const QUERY_TIME: i64 = 1_000_000;
const IN_WINDOW_SEED: u8 = 15;
const FUSED_ORDER: [u8; 10] = [1, 11, 2, 12, 3, 13, 4, 14, 5, 15];

#[test]
fn windowed_multi_primary_keeps_in_window_row_past_max_slot_len() {
    let engine = two_slot_engine();
    let result = temporal_search(
        &engine,
        &query(2, None, Some(64)),
        Some(last_hour()),
        &policy(),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("windowed multi-primary search");

    // The regression: seed 15 is in-window but fused rank 10 > max slot len 5.
    assert_eq!(ids(&result.hits), vec![IN_WINDOW_SEED]);
    assert_eq!(ids_from_cx(&result.pre_boost_ranking), FUSED_ORDER.to_vec());
    assert_eq!(ids_from_cx(&result.windowed_ranking), vec![IN_WINDOW_SEED]);

    let report = &result.window_recall;
    assert!(report.windowed);
    assert_eq!(report.requested_k, 2);
    assert_eq!(report.union_bound, 10);
    assert_eq!(report.effective_budget, 10);
    assert_eq!(report.candidates_fetched, 10);
    assert_eq!(report.in_window_count, 1);
    assert!(report.corpus_exhausted);
    assert_eq!(report.rounds, 1);
    assert_eq!(report.policy, WindowRecallPolicy::Exhaustive);
}

#[test]
fn bounded_policy_deepens_geometrically_until_in_window_found() {
    let engine = two_slot_engine();
    let result = temporal_search_with_recall(
        &engine,
        &query(1, Some(2), Some(64)),
        Some(last_hour()),
        &policy(),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
        WindowRecallPolicy::Bounded { max_candidates: 10 },
    )
    .expect("bounded windowed search");

    // Rounds: budget 2 (0 in-window) -> 8 (0 in-window) -> 10 (found).
    assert_eq!(ids(&result.hits), vec![IN_WINDOW_SEED]);
    let report = &result.window_recall;
    assert_eq!(report.rounds, 3);
    assert_eq!(report.effective_budget, 10);
    assert_eq!(report.candidates_fetched, 10);
    assert_eq!(report.in_window_count, 1);
    assert_eq!(report.requested_recall_k, Some(2));
}

#[test]
fn bounded_policy_fails_closed_when_budget_cannot_prove_completeness() {
    let engine = two_slot_engine();
    let error = temporal_search_with_recall(
        &engine,
        &query(1, Some(2), Some(64)),
        Some(last_hour()),
        &policy(),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
        WindowRecallPolicy::Bounded { max_candidates: 4 },
    )
    .expect_err("budget below in-window fused rank must fail closed");

    assert_eq!(error.code, CALYX_TEMPORAL_WINDOW_BUDGET_EXHAUSTED);
    assert!(error.message.contains("max_candidates 4"));
    assert!(error.message.contains("found 0"));
}

#[test]
fn bounded_policy_below_k_is_rejected() {
    let engine = two_slot_engine();
    let error = temporal_search_with_recall(
        &engine,
        &query(2, None, Some(64)),
        Some(last_hour()),
        &policy(),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
        WindowRecallPolicy::Bounded { max_candidates: 1 },
    )
    .expect_err("max_candidates below k must be rejected");

    assert_eq!(error.code, CALYX_SEXTANT_QUERY_SHAPE);
}

#[test]
fn windowless_query_honors_caller_recall_budget() {
    let engine = two_slot_engine();
    let result = temporal_search(
        &engine,
        &query(2, Some(3), Some(64)),
        None,
        &policy(),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("windowless search");

    let report = &result.window_recall;
    assert!(!report.windowed);
    assert_eq!(report.effective_budget, 3);
    assert_eq!(report.candidates_fetched, 3);
    assert_eq!(report.rounds, 1);
}

#[test]
fn exhaustive_budget_raises_caller_ef_instead_of_failing() {
    let engine = two_slot_engine();
    // Old behavior: silent k expansion with caller ef=2 hit
    // CALYX_SEXTANT_EF_TOO_SMALL because ef < expanded k.
    let result = temporal_search(
        &engine,
        &query(2, None, Some(2)),
        Some(last_hour()),
        &policy(),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("windowed search with small caller ef");

    assert_eq!(ids(&result.hits), vec![IN_WINDOW_SEED]);
    assert_eq!(result.window_recall.effective_ef, Some(10));
}

#[test]
fn empty_window_returns_complete_empty_result_not_error() {
    let engine = two_slot_engine();
    // Window in the far past: no document matches.
    let window = TimeWindow::new(1_000, 2_000).expect("window");
    let result = temporal_search(
        &engine,
        &query(2, None, Some(64)),
        Some(window),
        &policy(),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("complete empty result");

    assert!(result.hits.is_empty());
    assert!(result.windowed_ranking.is_empty());
    assert!(result.window_recall.corpus_exhausted);
    assert_eq!(result.window_recall.in_window_count, 0);
}

fn two_slot_engine() -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(SLOT_A, 2, 42)).unwrap();
    map.register(HnswIndex::new(SLOT_B, 2, 43)).unwrap();
    let mut engine = SearchEngine::new(map);
    for rank in 1..=5_u8 {
        insert_doc(&mut engine, SLOT_A, rank, rank, out_of_window_created_at());
        let b_seed = rank + 10;
        let created_at = if b_seed == IN_WINDOW_SEED {
            in_window_created_at()
        } else {
            out_of_window_created_at()
        };
        insert_doc(&mut engine, SLOT_B, b_seed, rank, created_at);
    }
    engine
}

fn insert_doc(engine: &mut SearchEngine, slot: SlotId, seed: u8, slot_rank: u8, created_at: u64) {
    // Cosine against query [1, 0] decreases as slot_rank grows.
    let vector = dense(vec![1.0, 0.2 * f32::from(slot_rank)]);
    engine
        .indexes
        .insert(slot, cx(seed), vector, u64::from(seed))
        .unwrap();
    engine.put_constellation(row(seed, created_at));
}

fn in_window_created_at() -> u64 {
    (QUERY_TIME - 600) as u64
}

fn out_of_window_created_at() -> u64 {
    (QUERY_TIME - 100_000) as u64
}

fn last_hour() -> TimeWindow {
    TimeWindow::last_hours(1, &TemporalFixedClock::new(QUERY_TIME)).expect("window")
}

fn query(k: usize, recall_k: Option<usize>, ef: Option<usize>) -> Query {
    Query {
        k,
        recall_k,
        ef,
        ..Query::new("window recall proof")
            .with_vector(dense(vec![1.0, 0.0]))
            .with_slots(vec![SLOT_A, SLOT_B])
    }
}

fn policy() -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        DecayFunction::Step,
        PeriodicOptions::new(None, None).expect("periodic"),
        Default::default(),
        Default::default(),
        Default::default(),
        true,
    )
    .expect("policy")
}

fn row(seed: u8, created_at: u64) -> calyx_core::Constellation {
    calyx_core::Constellation {
        cx_id: cx(seed),
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 1,
        created_at,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("zfs://calyx/window-recall/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("window-recall".to_string()),
            value: AnchorValue::Text("synthetic".to_string()),
            source: "issue-633".to_string(),
            observed_at: created_at,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn ids(hits: &[Hit]) -> Vec<u8> {
    hits.iter().map(|hit| hit.cx_id.as_bytes()[0]).collect()
}

fn ids_from_cx(ids: &[CxId]) -> Vec<u8> {
    ids.iter().map(|id| id.as_bytes()[0]).collect()
}
