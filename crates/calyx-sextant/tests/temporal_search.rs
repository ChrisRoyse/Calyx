use std::collections::{BTreeMap, BTreeSet};

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CALYX_TEMPORAL_AP60_VIOLATION, CxFlags, CxId, DecayFunction,
    InputRef, LedgerRef, Modality, SlotId, VaultId,
};
use calyx_sextant::{
    CALYX_SEXTANT_INDEX_EMPTY, FreshnessTag, FusionStrategy, Hit, HnswIndex, PeriodicOptions,
    ProvenanceSource, Query, SearchEngine, SlotIndexMap, TemporalFixedClock, TemporalPolicy,
    TemporalScores, TemporalSearchInput, TemporalSearchResult, TimeWindow, temporal_search,
    temporal_search_from_primary, temporal_search_pipeline, validate_primary_temporal_weight,
};
use proptest::prelude::*;
use sextant_support::{cx_u8_fill as cx, dense};

const CONTENT_SLOT: SlotId = SlotId::new(8);
const EMPTY_PRIMARY_SLOT: SlotId = SlotId::new(9);
const TEMPORAL_SLOT: SlotId = SlotId::new(20);
const QUERY_TIME: i64 = 1_000_000;
const SCORE_EPSILON: f32 = 1.0e-5;

#[test]
fn temporal_search_records_preboost_and_reranks_postboost() {
    let engine = sample_engine();
    let result = temporal_search(
        &engine,
        &sample_query(3),
        None,
        &policy_step(None),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("temporal search");

    assert_eq!(ids_from_cx(&result.pre_boost_ranking), vec![1, 2, 3]);
    assert_eq!(ids(&result.hits), vec![2, 1]);
    assert_eq!(result.temporal_weight_used, 0.0);
    assert_eq!(result.primary_slots_used, vec![CONTENT_SLOT]);
    assert_eq!(result.temporal_slots_excluded, vec![TEMPORAL_SLOT]);
}

#[test]
fn temporal_search_window_excludes_old_hit() {
    let engine = sample_engine();
    let result = temporal_search(
        &engine,
        &sample_query(3),
        Some(TimeWindow::last_hours(1, &TemporalFixedClock::new(QUERY_TIME)).unwrap()),
        &policy_step(None),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("windowed temporal search");

    assert_eq!(ids(&result.hits), vec![2]);
}

#[test]
fn temporal_search_overfetches_before_window_filter_for_k_one() {
    let engine = sample_engine();
    let result = temporal_search(
        &engine,
        &sample_query(1),
        Some(TimeWindow::last_hours(1, &TemporalFixedClock::new(QUERY_TIME)).unwrap()),
        &policy_step(None),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("windowed temporal search");

    assert_eq!(ids_from_cx(&result.pre_boost_ranking), vec![1, 2, 3]);
    assert_eq!(ids(&result.hits), vec![2]);
}

#[test]
fn zero_content_score_is_not_elevated_or_final_surfaced_by_recency() {
    let engine = sample_engine();
    let result = temporal_search(
        &engine,
        &sample_query(3),
        None,
        &policy_step(None),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("temporal search");
    assert!(!result.hits.iter().any(|hit| hit.cx_id == cx(3)));

    let boosted = temporal_search_pipeline(
        vec![raw_hit(3, 0.0, 1, Some(QUERY_TIME - 100))],
        &TimeWindow::all(),
        &policy_step(None),
        0,
        &TemporalFixedClock::new(QUERY_TIME),
    )
    .expect("zero proof");
    let zero = boosted.first().expect("zero proof hit");

    assert_eq!(zero.score, 0.0);
    assert_eq!(zero.temporal_scores, Some(TemporalScores::zero()));
}

#[test]
fn empty_registered_primary_slot_returns_empty_result() {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(CONTENT_SLOT, 2, 42)).unwrap();
    let engine = SearchEngine::new(map);
    let result = temporal_search(
        &engine,
        &sample_query(3),
        None,
        &policy_step(None),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("empty vault");

    assert!(result.hits.is_empty());
    assert!(result.pre_boost_ranking.is_empty());
}

#[test]
fn mixed_empty_primary_slot_fails_loud() {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(EMPTY_PRIMARY_SLOT, 2, 42))
        .unwrap();
    map.register(HnswIndex::new(CONTENT_SLOT, 2, 43)).unwrap();
    let mut engine = SearchEngine::new(map);
    engine
        .indexes
        .insert(CONTENT_SLOT, cx(1), dense(vec![1.0, 0.0]), 1)
        .unwrap();
    engine.put_constellation(row(1, 999_500));
    let query = Query::new("mixed empty")
        .with_vector(dense(vec![1.0, 0.0]))
        .with_slots(vec![EMPTY_PRIMARY_SLOT, CONTENT_SLOT]);

    let error = temporal_search(
        &engine,
        &query,
        None,
        &policy_step(None),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect_err("mixed empty primary slot fails loud");

    assert_eq!(error.code, CALYX_SEXTANT_INDEX_EMPTY);
}

#[test]
fn timezone_offset_controls_periodic_scoring() {
    let policy = policy_step(Some(14));
    let hit = raw_hit(9, 0.90, 1, Some(1_704_222_000));

    let local = temporal_search_from_primary(TemporalSearchInput {
        primary_hits: vec![hit.clone()],
        temporal_weight_used: 0.0,
        final_k: 1,
        window: None,
        policy: &policy,
        clock: &TemporalFixedClock::new(1_704_222_060),
        tz_offset_secs: -18_000,
        primary_slots_used: vec![CONTENT_SLOT],
        temporal_slots_excluded: Vec::new(),
        window_recall: Default::default(),
    })
    .expect("local tz");
    let utc = temporal_search_from_primary(TemporalSearchInput {
        primary_hits: vec![hit],
        temporal_weight_used: 0.0,
        final_k: 1,
        window: None,
        policy: &policy,
        clock: &TemporalFixedClock::new(1_704_222_060),
        tz_offset_secs: 0,
        primary_slots_used: vec![CONTENT_SLOT],
        temporal_slots_excluded: Vec::new(),
        window_recall: Default::default(),
    })
    .expect("utc");

    assert_close(periodic_score(&local), 0.5);
    assert_close(periodic_score(&utc), 0.0);
}

#[test]
fn primary_temporal_weight_violation_fails_closed() {
    let error = validate_primary_temporal_weight(0.1).expect_err("nonzero temporal weight");
    assert_eq!(error.code, CALYX_TEMPORAL_AP60_VIOLATION);
}

#[test]
fn temporal_single_lens_primary_fails_closed() {
    let engine = sample_engine();
    let mut query = sample_query(3);
    query.fusion = Some(FusionStrategy::SingleLens {
        slot: TEMPORAL_SLOT,
    });

    let error = temporal_search(
        &engine,
        &query,
        None,
        &policy_step(None),
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect_err("temporal single-lens rejected");
    assert_eq!(error.code, CALYX_TEMPORAL_AP60_VIOLATION);
}

proptest! {
    #[test]
    fn temporal_search_from_primary_returns_subset_of_primary_ids(
        seeds in proptest::collection::btree_set(1_u8..64, 0..24),
    ) {
        let primary = seeds
            .iter()
            .enumerate()
            .map(|(index, seed)| {
                raw_hit(*seed, 1.0 / (*seed as f32 + 1.0), index + 1, Some(999_000))
            })
            .collect::<Vec<_>>();
        let primary_ids = primary.iter().map(|hit| hit.cx_id).collect::<BTreeSet<_>>();
        let result = temporal_search_from_primary(TemporalSearchInput {
            primary_hits: primary,
            temporal_weight_used: 0.0,
            final_k: 10,
            window: None,
            policy: &policy_step(None),
            clock: &TemporalFixedClock::new(QUERY_TIME),
            tz_offset_secs: 0,
            primary_slots_used: vec![CONTENT_SLOT],
            temporal_slots_excluded: vec![TEMPORAL_SLOT],
            window_recall: Default::default(),
        }).expect("temporal from primary");

        prop_assert!(result.hits.iter().all(|hit| primary_ids.contains(&hit.cx_id)));
    }
}

fn sample_engine() -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(CONTENT_SLOT, 2, 42)).unwrap();
    map.register(HnswIndex::new(TEMPORAL_SLOT, 2, 43)).unwrap();
    let mut engine = SearchEngine::new(map);
    let rows = [
        (1, vec![1.0, 0.0], vec![0.0, 1.0], 900_000),
        (2, vec![0.98, 0.2], vec![0.0, 1.0], 999_500),
        (3, vec![0.0, 1.0], vec![1.0, 0.0], 999_900),
    ];
    for (seed, content, temporal, created_at) in rows {
        let id = cx(seed);
        engine
            .indexes
            .insert(CONTENT_SLOT, id, dense(content), seed as u64)
            .unwrap();
        engine
            .indexes
            .insert(TEMPORAL_SLOT, id, dense(temporal), seed as u64)
            .unwrap();
        engine.put_constellation(row(seed, created_at));
    }
    engine
}

fn sample_query(k: usize) -> Query {
    Query {
        k,
        explain: true,
        ..Query::new("temporal proof")
            .with_vector(dense(vec![1.0, 0.0]))
            .with_slots(vec![CONTENT_SLOT, TEMPORAL_SLOT])
    }
}

fn policy_step(target_hour: Option<u8>) -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        DecayFunction::Step,
        PeriodicOptions::new(target_hour, None).expect("periodic"),
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
            pointer: Some(format!("zfs://calyx/temporal-search/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("temporal-search".to_string()),
            value: AnchorValue::Text("synthetic".to_string()),
            source: "issue-377".to_string(),
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

fn raw_hit(seed: u8, score: f32, rank: usize, event_time_secs: Option<i64>) -> Hit {
    Hit {
        cx_id: cx(seed),
        score,
        rank,
        event_time_secs,
        temporal_scores: None,
        causal_confidence: calyx_sextant::CausalConfidence::Absent,
        causal_gate: None,
        per_lens: Vec::new(),
        cross_terms_used: false,
        guard: None,
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        provenance_source: ProvenanceSource::Stub,
        freshness: FreshnessTag::fresh(0),
        explain: None,
    }
}

fn ids(hits: &[Hit]) -> Vec<u8> {
    hits.iter().map(|hit| hit.cx_id.as_bytes()[0]).collect()
}

fn ids_from_cx(ids: &[CxId]) -> Vec<u8> {
    ids.iter().map(|id| id.as_bytes()[0]).collect()
}

fn periodic_score(result: &TemporalSearchResult) -> f32 {
    result.hits[0].temporal_scores.expect("scores").e3_periodic
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= SCORE_EPSILON,
        "actual {actual} expected {expected}"
    );
}
