use std::collections::BTreeMap;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId,
    SlotVector, VaultId,
};
use calyx_sextant::{
    CALYX_SEXTANT_VECTOR_SHAPE, HitGuardMode, HnswIndex, Query, QueryGuard, SearchEngine,
    SlotIndexMap,
};
use calyx_ward::{GuardId, GuardPolicy, GuardProfile, NoveltyAction};
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

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
#[ignore = "manual aiwonder FSV fixture; set CALYX_SEXTANT_PH38_T06_FSV_DIR"]
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

fn row(cx_id: CxId, vector: SlotVector, seq: u64) -> calyx_core::Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(slot(), vector);
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

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense { dim: 2, data }
}

fn write_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
    let path = std::path::Path::new(root).join(name);
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

fn cx(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}

fn vault() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

const fn slot() -> SlotId {
    SlotId::new(8)
}
