use std::collections::BTreeMap;
use std::fs;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CALYX_TEMPORAL_AP60_VIOLATION, CxFlags, CxId, DecayFunction,
    InputRef, LedgerRef, Modality, SlotId, VaultId,
};
use calyx_sextant::{
    FreshnessTag, Hit, HnswIndex, PeriodicOptions, ProvenanceSource, Query, SearchEngine,
    SlotIndexMap, TemporalFixedClock, TemporalPolicy, TemporalSearchInput, TimeWindow,
    temporal_search, temporal_search_from_primary, temporal_search_pipeline,
    validate_primary_temporal_weight,
};
use serde_json::json;
use sextant_support::{
    cx_u8_fill as cx, dense, fsv_root, reset_dir, write_json, write_root_file_blake3_sums,
};

const CONTENT_SLOT: SlotId = SlotId::new(8);
const TEMPORAL_SLOT: SlotId = SlotId::new(20);
const QUERY_TIME: i64 = 1_000_000;

#[test]
fn temporal_search_fsv_writes_result_readback() {
    let (root, keep_root) = fsv_root(
        "CALYX_TEMPORAL_SEARCH_FSV_ROOT",
        "calyx-temporal-search-fsv",
    );
    reset_dir(&root);
    let output_path = root.join("temporal-search-readback.json");
    let before_output_exists = output_path.exists();

    let engine = sample_engine();
    let policy = policy_step(None);
    let query = Query {
        k: 2,
        explain: true,
        ..Query::new("temporal search fsv")
            .with_vector(dense(vec![1.0, 0.0]))
            .with_slots(vec![CONTENT_SLOT, TEMPORAL_SLOT])
            .with_recall_k(3)
    };
    write_json(
        &root.join("temporal-search-input.json"),
        &json!({
            "clock_secs": QUERY_TIME,
            "tz_offset_secs": 0,
            "query_k": query.k,
            "recall_k": query.recall_k,
            "query_slots": query.slots,
            "policy": policy,
            "hand_expected": {
                "pre_boost_ranking": [id_hex(1), id_hex(2), id_hex(3)],
                "final_hits": [id_hex(2), id_hex(1)],
                "content_miss_id": id_hex(3),
                "content_miss_absent_from_final": true,
                "temporal_weight_used": 0.0,
                "primary_slots_used": [CONTENT_SLOT],
                "temporal_slots_excluded": [TEMPORAL_SLOT],
            },
        }),
    );

    let result = temporal_search(
        &engine,
        &query,
        None,
        &policy,
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("temporal search");
    let zero_surface = temporal_search(
        &engine,
        &Query {
            k: 3,
            ..query.clone()
        },
        None,
        &policy,
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("zero surface");
    let zero_boost_proof = temporal_search_pipeline(
        vec![raw_hit(3, 0.0, 1, Some(QUERY_TIME - 100))],
        &TimeWindow::all(),
        &policy,
        0,
        &TemporalFixedClock::new(QUERY_TIME),
    )
    .expect("zero boost proof");
    let windowed = temporal_search(
        &engine,
        &Query {
            k: 3,
            ..query.clone()
        },
        Some(TimeWindow::last_hours(1, &TemporalFixedClock::new(QUERY_TIME)).unwrap()),
        &policy,
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("windowed");
    let underfill_window = TimeWindow::last_hours(1, &TemporalFixedClock::new(QUERY_TIME)).unwrap();
    let underfill_query = Query {
        k: 1,
        explain: true,
        ..Query::new("temporal search underfill")
            .with_vector(dense(vec![1.0, 0.0]))
            .with_slots(vec![CONTENT_SLOT, TEMPORAL_SLOT])
    };
    let underfill = temporal_search(
        &engine,
        &underfill_query,
        Some(underfill_window),
        &policy,
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("underfill regression");
    let empty = temporal_search(
        &empty_engine(),
        &query,
        None,
        &policy,
        &TemporalFixedClock::new(QUERY_TIME),
        0,
    )
    .expect("empty edge");
    let invalid = validate_primary_temporal_weight(0.25).expect_err("invalid weight");
    let tz_local = timezone_result(-18_000);
    let tz_utc = timezone_result(0);

    let readback = json!({
        "before_output_exists": before_output_exists,
        "trigger": "calyx_sextant::temporal_search(primary retrieval -> window -> temporal boost -> causal gate)",
        "result": result,
        "actual_pre_boost": ids_from_cx(&result.pre_boost_ranking),
        "actual_final": ids(&result.hits),
        "content_miss_absent_from_final": !result.hits.iter().any(|hit| hit.cx_id == cx(3)),
        "policy_never_dominant_visible": result.policy_snapshot.never_dominant,
        "zero_content_edge": {
            "before_content_score": 0.0,
            "boost_after_score": score_for(&zero_boost_proof, 3),
            "boost_after_temporal_scores": zero_boost_proof
                .iter()
                .find(|hit| hit.cx_id == cx(3))
                .and_then(|hit| hit.temporal_scores),
            "final_surface_contains_zero_content": zero_surface
                .hits
                .iter()
                .any(|hit| hit.cx_id == cx(3)),
        },
        "window_edge": {
            "before_count": result.pre_boost_ranking.len(),
            "window": TimeWindow::last_hours(1, &TemporalFixedClock::new(QUERY_TIME)).unwrap(),
            "after_ids": ids(&windowed.hits),
            "old_id_excluded": !windowed.hits.iter().any(|hit| hit.cx_id == cx(1)),
        },
        "underfill_edge": {
            "query_k": underfill_query.k,
            "query_recall_k": underfill_query.recall_k,
            "raw_pre_window_ids": ids_from_cx(&underfill.pre_boost_ranking),
            "raw_rank_1_id": id_hex(1),
            "raw_rank_1_event_time_secs": 900_000,
            "window": underfill_window,
            "raw_rank_1_in_window": underfill_window.contains(900_000),
            "raw_rank_2_id": id_hex(2),
            "raw_rank_2_event_time_secs": 999_500,
            "raw_rank_2_in_window": underfill_window.contains(999_500),
            "final_ids": ids(&underfill.hits),
            "filled_final_k": underfill.hits.len() == underfill_query.k,
        },
        "empty_edge": {
            "before_index_count": 0,
            "after_hit_count": empty.hits.len(),
            "after_pre_boost_count": empty.pre_boost_ranking.len(),
        },
        "tz_edge": {
            "event_utc_secs": 1_704_222_000,
            "utc_minus_5_e3": tz_local.hits[0].temporal_scores.unwrap().e3_periodic,
            "utc_e3": tz_utc.hits[0].temporal_scores.unwrap().e3_periodic,
            "expected_utc_minus_5_e3": 0.5,
            "expected_utc_e3": 0.0,
        },
        "invalid_weight_edge": {
            "before_temporal_weight_used": 0.25,
            "after_error_code": invalid.code,
            "expected_error_code": CALYX_TEMPORAL_AP60_VIOLATION,
        },
    });
    write_json(&output_path, &readback);
    write_root_file_blake3_sums(&root);

    println!("temporal_search_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(ids_from_cx(&result.pre_boost_ranking), vec![1, 2, 3]);
    assert_eq!(ids(&result.hits), vec![2, 1]);
    assert!(!result.hits.iter().any(|hit| hit.cx_id == cx(3)));
    assert_eq!(score_for(&zero_boost_proof, 3), 0.0);
    assert!(!zero_surface.hits.iter().any(|hit| hit.cx_id == cx(3)));
    assert!(!windowed.hits.iter().any(|hit| hit.cx_id == cx(1)));
    assert_eq!(ids_from_cx(&underfill.pre_boost_ranking), vec![1, 2, 3]);
    assert_eq!(ids(&underfill.hits), vec![2]);
    assert!(empty.hits.is_empty());
    assert_eq!(invalid.code, CALYX_TEMPORAL_AP60_VIOLATION);

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn timezone_result(tz_offset_secs: i32) -> calyx_sextant::TemporalSearchResult {
    let policy = policy_step(Some(14));
    temporal_search_from_primary(TemporalSearchInput {
        primary_hits: vec![raw_hit(9, 0.90, 1, Some(1_704_222_000))],
        temporal_weight_used: 0.0,
        final_k: 1,
        window: None,
        policy: &policy,
        clock: &TemporalFixedClock::new(1_704_222_060),
        tz_offset_secs,
        primary_slots_used: vec![CONTENT_SLOT],
        temporal_slots_excluded: Vec::new(),
        window_recall: Default::default(),
    })
    .expect("timezone result")
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

fn empty_engine() -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(CONTENT_SLOT, 2, 42)).unwrap();
    SearchEngine::new(map)
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
            pointer: Some(format!("zfs://calyx/temporal-search-fsv/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("temporal-search-fsv".to_string()),
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

fn score_for(hits: &[Hit], seed: u8) -> f32 {
    hits.iter()
        .find(|hit| hit.cx_id == cx(seed))
        .expect("hit by seed")
        .score
}

fn ids(hits: &[Hit]) -> Vec<u8> {
    hits.iter().map(|hit| hit.cx_id.as_bytes()[0]).collect()
}

fn ids_from_cx(ids: &[CxId]) -> Vec<u8> {
    ids.iter().map(|id| id.as_bytes()[0]).collect()
}

fn id_hex(seed: u8) -> String {
    cx(seed).to_string()
}
