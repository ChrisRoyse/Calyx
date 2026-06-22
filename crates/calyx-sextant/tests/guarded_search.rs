use std::collections::BTreeMap;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId,
    SlotVector,
};
use calyx_sextant::{
    CALYX_SEXTANT_VECTOR_SHAPE, HitGuardMode, HnswIndex, Query, QueryGuard, SearchEngine,
    SlotIndexMap,
};
use calyx_ward::{GuardPolicy, GuardProfile, NoveltyAction};
use serde_json::json;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::{
    cx_u8_fill as cx, default_vault_id as vault, dense, guarded_test_guard_id as guard_id,
    write_named_json as write_json,
};

#[test]
fn in_region_only_drops_ood_and_attaches_verdict() {
    let engine = guarded_engine(false);
    let before = engine.search(&base_query(2)).expect("unguarded search");
    let report = engine
        .search_with_guard_report(&guarded_query(2, 2, true))
        .expect("guarded search");

    assert_eq!(ids(&before), vec![cx(2), cx(1)]);
    assert_eq!(ids(&report.hits), vec![cx(1)]);
    assert_eq!(report.dropped_guard_hits.len(), 1);
    assert_eq!(report.dropped_guard_hits[0].cx_id, cx(2));
    assert_eq!(report.dropped_guard_hits[0].reason, "ood");
    assert!(
        !report.dropped_guard_hits[0]
            .verdict
            .as_ref()
            .unwrap()
            .overall_pass
    );

    let guard = report.hits[0].guard.as_ref().expect("surviving verdict");
    assert_eq!(guard.mode, HitGuardMode::InRegionOnly);
    assert!(guard.verdict.overall_pass);
    let explain = report.hits[0].explain.as_ref().expect("explain");
    assert_eq!(explain.guard_dropped.len(), 1);
    assert_eq!(explain.guard_dropped[0].cx_id, cx(2));
}

#[test]
fn guarded_search_expands_candidate_window_before_final_k() {
    let engine = guarded_engine(false);
    let before = engine.search(&base_query(1)).expect("unguarded top hit");
    let after = engine
        .search(&guarded_query(1, 0, false))
        .expect("guarded top hit");

    assert_eq!(ids(&before), vec![cx(2)]);
    assert_eq!(ids(&after), vec![cx(1)]);
    assert!(after[0].guard.as_ref().unwrap().verdict.overall_pass);
}

#[test]
fn missing_candidate_constellation_is_dropped_with_reason() {
    let engine = guarded_engine(true);
    let report = engine
        .search_with_guard_report(&guarded_query(3, 3, false))
        .expect("guarded search");

    assert!(report.hits.iter().all(|hit| hit.cx_id != cx(3)));
    let missing = report
        .dropped_guard_hits
        .iter()
        .find(|dropped| dropped.cx_id == cx(3))
        .expect("missing doc dropped");
    assert_eq!(missing.reason, "missing_constellation");
    assert!(missing.verdict.is_none());
}

#[test]
fn non_dense_guarded_query_fails_closed() {
    let engine = guarded_engine(false);
    let err = engine
        .search(
            &Query::new("guarded")
                .with_slots(vec![slot()])
                .with_vector(SlotVector::Sparse {
                    dim: 2,
                    entries: Vec::new(),
                })
                .with_guard(QueryGuard::InRegionOnly(profile())),
        )
        .expect_err("sparse guard query fails");

    assert_eq!(err.code, CALYX_SEXTANT_VECTOR_SHAPE);
}

#[test]
fn multi_slot_guard_requires_slot_aware_query_vectors() {
    let engine = multi_slot_guarded_engine();
    let err = engine
        .search_with_guard_report(&multi_slot_query_without_guard_vectors())
        .expect_err("multi-slot guard without slot-aware vectors fails");

    assert_eq!(err.code, CALYX_SEXTANT_VECTOR_SHAPE);
    assert!(err.message.contains("slot-aware query guard vectors"));
}

#[test]
fn multi_slot_guard_uses_distinct_query_vectors() {
    let engine = multi_slot_guarded_engine();
    let before = engine.search(&multi_slot_base_query(2)).expect("before");
    let report = engine
        .search_with_guard_report(&multi_slot_guarded_query(2, true))
        .expect("guarded multi-slot search");

    assert_eq!(ids(&before), vec![cx(4), cx(5)]);
    assert_eq!(ids(&report.hits), vec![cx(4)]);
    assert_eq!(report.dropped_guard_hits.len(), 1);
    assert_eq!(report.dropped_guard_hits[0].cx_id, cx(5));
    assert_eq!(report.dropped_guard_hits[0].reason, "ood");
    let dropped_verdict = report.dropped_guard_hits[0]
        .verdict
        .as_ref()
        .expect("dropped verdict");
    assert!(
        dropped_verdict
            .per_slot
            .iter()
            .any(|slot| slot.slot == slot_style() && !slot.pass)
    );
    let survivor = report.hits[0].guard.as_ref().expect("survivor guard");
    assert!(survivor.verdict.overall_pass);
    assert_eq!(survivor.verdict.per_slot.len(), 2);
    assert!(
        survivor
            .verdict
            .per_slot
            .iter()
            .any(|slot| slot.slot == slot_style() && slot.pass)
    );
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_SEXTANT_PH38_T06_FSV_DIR"]
fn ph38_t06_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_SEXTANT_PH38_T06_FSV_DIR")
        .expect("CALYX_SEXTANT_PH38_T06_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let engine = guarded_engine(true);
    let before = engine.search(&base_query(3)).expect("before search");
    let guarded = engine
        .search_with_guard_report(&guarded_query(2, 3, true))
        .expect("guarded search");
    let missing_doc = engine
        .search_with_guard_report(&guarded_query(3, 3, false))
        .expect("missing doc edge");
    let non_dense = engine
        .search(
            &Query::new("guarded")
                .with_slots(vec![slot()])
                .with_vector(SlotVector::Sparse {
                    dim: 2,
                    entries: Vec::new(),
                })
                .with_guard(QueryGuard::InRegionOnly(profile())),
        )
        .expect_err("non-dense query edge");

    write_json(&root, "before-unguarded-hits.json", &before);
    write_json(&root, "after-guarded-hits.json", &guarded.hits);
    write_json(
        &root,
        "dropped-guard-hits.json",
        &guarded.dropped_guard_hits,
    );
    write_json(&root, "missing-doc-report.json", &missing_doc);
    write_json(
        &root,
        "non-dense-query-error.json",
        &json!({"code": non_dense.code, "message": non_dense.message}),
    );

    println!(
        "FSV_SEXTANT_INREGION before={} after={} dropped={} survivor={}",
        before.len(),
        guarded.hits.len(),
        guarded.dropped_guard_hits.len(),
        guarded.hits.first().map(|hit| hit.cx_id).unwrap_or(cx(0))
    );
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_SEXTANT_PH38_T06_MULTISLOT_FSV_DIR"]
fn ph38_t06_multi_slot_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_SEXTANT_PH38_T06_MULTISLOT_FSV_DIR")
        .expect("CALYX_SEXTANT_PH38_T06_MULTISLOT_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let engine = multi_slot_guarded_engine();
    let before = engine.search(&multi_slot_base_query(2)).expect("before");
    let missing_guard_vectors = engine
        .search_with_guard_report(&multi_slot_query_without_guard_vectors())
        .expect_err("missing slot-aware query guard vectors");
    let guarded = engine
        .search_with_guard_report(&multi_slot_guarded_query(2, true))
        .expect("guarded multi-slot search");

    write_json(&root, "before-unguarded-hits.json", &before);
    write_json(
        &root,
        "missing-guard-vectors-error.json",
        &json!({
            "code": missing_guard_vectors.code,
            "message": missing_guard_vectors.message,
        }),
    );
    write_json(&root, "after-guarded-hits.json", &guarded.hits);
    write_json(
        &root,
        "dropped-guard-hits.json",
        &guarded.dropped_guard_hits,
    );
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "before_ids": ids(&before),
            "after_ids": ids(&guarded.hits),
            "dropped_ids": ids_from_dropped(&guarded.dropped_guard_hits),
            "missing_guard_vectors_error_code": missing_guard_vectors.code,
            "missing_guard_vectors_message": missing_guard_vectors.message,
            "survivor_per_slot_count": guarded
                .hits
                .first()
                .and_then(|hit| hit.guard.as_ref())
                .map(|guard| guard.verdict.per_slot.len()),
            "dropped_style_slot_failed": guarded
                .dropped_guard_hits
                .first()
                .and_then(|dropped| dropped.verdict.as_ref())
                .map(|verdict| verdict
                    .per_slot
                    .iter()
                    .any(|slot| slot.slot == slot_style() && !slot.pass)),
        }),
    );

    println!(
        "FSV_SEXTANT_INREGION_MULTISLOT before={} after={} dropped={} missing_code={}",
        before.len(),
        guarded.hits.len(),
        guarded.dropped_guard_hits.len(),
        missing_guard_vectors.code,
    );
}

fn guarded_engine(include_missing_doc_candidate: bool) -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(slot(), 2, 42)).unwrap();
    let mut engine = SearchEngine::new(map);
    insert(&engine, cx(2), dense(vec![1.0, 0.0]), 2);
    insert(&engine, cx(1), dense(vec![0.80, 0.60]), 1);
    if include_missing_doc_candidate {
        insert(&engine, cx(3), dense(vec![0.70, 0.714]), 3);
    }
    engine.put_constellation(row(cx(2), dense(vec![0.0, 1.0]), 2));
    engine.put_constellation(row(cx(1), dense(vec![1.0, 0.0]), 1));
    engine
}

fn multi_slot_guarded_engine() -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(slot(), 2, 42)).unwrap();
    let mut engine = SearchEngine::new(map);
    insert(&engine, cx(4), dense(vec![1.0, 0.0]), 4);
    insert(&engine, cx(5), dense(vec![0.95, 0.05]), 5);
    engine.put_constellation(row_with_slots(
        cx(4),
        vec![
            (slot(), dense(vec![1.0, 0.0])),
            (slot_style(), dense(vec![0.0, 1.0, 0.0])),
        ],
        4,
    ));
    engine.put_constellation(row_with_slots(
        cx(5),
        vec![
            (slot(), dense(vec![1.0, 0.0])),
            (slot_style(), dense(vec![1.0, 0.0, 0.0])),
        ],
        5,
    ));
    engine
}

fn insert(engine: &SearchEngine, cx_id: CxId, vector: SlotVector, seq: u64) {
    engine.indexes.insert(slot(), cx_id, vector, seq).unwrap();
}

fn base_query(k: usize) -> Query {
    let mut query = Query::new("guarded")
        .with_slots(vec![slot()])
        .with_vector(dense(vec![1.0, 0.0]));
    query.k = k;
    query
}

fn guarded_query(k: usize, recall_k: usize, explain: bool) -> Query {
    let mut query = base_query(k)
        .with_guard(QueryGuard::InRegionOnly(profile()))
        .explain(explain);
    if recall_k > 0 {
        query = query.with_recall_k(recall_k);
    }
    query
}

fn multi_slot_base_query(k: usize) -> Query {
    let mut query = Query::new("guarded multi-slot")
        .with_slots(vec![slot()])
        .with_vector(dense(vec![1.0, 0.0]));
    query.k = k;
    query
}

fn multi_slot_query_without_guard_vectors() -> Query {
    multi_slot_base_query(2).with_guard(QueryGuard::InRegionOnly(multi_slot_profile()))
}

fn multi_slot_guarded_query(k: usize, explain: bool) -> Query {
    multi_slot_base_query(k)
        .with_guard(QueryGuard::InRegionOnly(multi_slot_profile()))
        .with_guard_vectors(BTreeMap::from([
            (slot(), dense(vec![1.0, 0.0])),
            (slot_style(), dense(vec![0.0, 1.0, 0.0])),
        ]))
        .explain(explain)
}

fn profile() -> GuardProfile {
    let mut tau = BTreeMap::new();
    tau.insert(slot(), 0.70);
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic-sextant".to_string(),
        tau,
        required_slots: vec![slot()],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn multi_slot_profile() -> GuardProfile {
    let mut tau = BTreeMap::new();
    tau.insert(slot(), 0.70);
    tau.insert(slot_style(), 0.70);
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic-sextant-multislot".to_string(),
        tau,
        required_slots: vec![slot(), slot_style()],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn row(cx_id: CxId, vector: SlotVector, seq: u64) -> calyx_core::Constellation {
    row_with_slots(cx_id, vec![(slot(), vector)], seq)
}

fn row_with_slots(
    cx_id: CxId,
    slot_vectors: Vec<(SlotId, SlotVector)>,
    seq: u64,
) -> calyx_core::Constellation {
    let slots = slot_vectors.into_iter().collect();
    calyx_core::Constellation {
        cx_id,
        vault_id: vault(),
        panel_version: 42,
        created_at: seq,
        input_ref: InputRef {
            hash: [seq as u8; 32],
            pointer: Some(format!("zfs://calyx/guarded-search/{seq}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("guard-region".to_string()),
            value: AnchorValue::Enum("trusted".to_string()),
            source: "ph38-t06-fsv".to_string(),
            observed_at: seq,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq,
            hash: [seq as u8; 32],
        },
        flags: CxFlags::default(),
    }
}

fn ids(hits: &[calyx_sextant::Hit]) -> Vec<CxId> {
    hits.iter().map(|hit| hit.cx_id).collect()
}

fn ids_from_dropped(hits: &[calyx_sextant::DroppedGuardHit]) -> Vec<CxId> {
    hits.iter().map(|hit| hit.cx_id).collect()
}

const fn slot() -> SlotId {
    SlotId::new(8)
}

const fn slot_style() -> SlotId {
    SlotId::new(9)
}
