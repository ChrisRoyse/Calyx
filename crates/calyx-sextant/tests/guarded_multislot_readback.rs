use std::collections::BTreeMap;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId,
    SlotVector,
};
use calyx_sextant::{
    CALYX_SEXTANT_VECTOR_SHAPE, HnswIndex, Query, QueryGuard, SearchEngine, SlotIndexMap,
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
fn multi_slot_guard_missing_required_guard_vector_fails_closed() {
    let (engine, _) = multi_slot_fixture();
    let err = engine
        .search_with_guard_report(&query_with_guard_vectors(
            2,
            BTreeMap::from([(slot_content(), dense(vec![1.0, 0.0]))]),
        ))
        .expect_err("partial guard_vectors map fails closed");

    assert_eq!(err.code, CALYX_SEXTANT_VECTOR_SHAPE);
    assert!(err.message.contains("missing slot-aware query vector"));
}

#[test]
fn multi_slot_guard_sparse_slot_aware_vector_fails_closed() {
    let (engine, _) = multi_slot_fixture();
    let err = engine
        .search_with_guard_report(&query_with_guard_vectors(
            2,
            BTreeMap::from([
                (slot_content(), dense(vec![1.0, 0.0])),
                (
                    slot_style(),
                    SlotVector::Sparse {
                        dim: 3,
                        entries: Vec::new(),
                    },
                ),
            ]),
        ))
        .expect_err("sparse guard vector fails closed");

    assert_eq!(err.code, CALYX_SEXTANT_VECTOR_SHAPE);
    assert!(err.message.contains("requires dense query vector"));
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_SEXTANT_ISSUE359_MULTISLOT_FSV_DIR"]
fn issue359_multislot_guard_vector_readback_fsv_fixture_writes_artifacts() {
    let root = std::env::var("CALYX_SEXTANT_ISSUE359_MULTISLOT_FSV_DIR")
        .expect("CALYX_SEXTANT_ISSUE359_MULTISLOT_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let (engine, candidate_rows) = multi_slot_fixture();
    let guarded_query = full_guard_query(2);
    let before = engine.search(&base_query(2)).expect("before search");
    let guarded = engine
        .search_with_guard_report(&guarded_query)
        .expect("guarded search");
    let partial = engine
        .search_with_guard_report(&query_with_guard_vectors(
            2,
            BTreeMap::from([(slot_content(), dense(vec![1.0, 0.0]))]),
        ))
        .expect_err("partial vector edge");
    let sparse = engine
        .search_with_guard_report(&query_with_guard_vectors(
            2,
            BTreeMap::from([
                (slot_content(), dense(vec![1.0, 0.0])),
                (
                    slot_style(),
                    SlotVector::Sparse {
                        dim: 3,
                        entries: Vec::new(),
                    },
                ),
            ]),
        ))
        .expect_err("sparse vector edge");

    write_json(&root, "guard-query.json", &guarded_query);
    write_json(
        &root,
        "candidate-slot-readback.json",
        &candidate_slot_readback(&candidate_rows),
    );
    write_json(&root, "before-unguarded-hits.json", &before);
    write_json(&root, "after-guarded-hits.json", &guarded.hits);
    write_json(
        &root,
        "dropped-guard-hits.json",
        &guarded.dropped_guard_hits,
    );
    write_json(
        &root,
        "edge-errors.json",
        &json!({
            "partial_guard_vectors": {
                "code": partial.code,
                "message": partial.message,
            },
            "sparse_guard_vector": {
                "code": sparse.code,
                "message": sparse.message,
            },
        }),
    );
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "before_ids": ids(&before),
            "after_ids": ids(&guarded.hits),
            "dropped_ids": guarded
                .dropped_guard_hits
                .iter()
                .map(|hit| hit.cx_id)
                .collect::<Vec<_>>(),
            "query_guard_vector_slots": [slot_content(), slot_style()],
            "candidate_slot_rows": candidate_rows
                .iter()
                .map(|row| row.cx_id)
                .collect::<Vec<_>>(),
            "partial_error_code": partial.code,
            "sparse_error_code": sparse.code,
            "style_slot_failure_recorded": guarded
                .dropped_guard_hits
                .first()
                .and_then(|hit| hit.verdict.as_ref())
                .map(|verdict| verdict
                    .per_slot
                    .iter()
                    .any(|slot| slot.slot == slot_style() && !slot.pass)),
        }),
    );

    println!(
        "FSV_SEXTANT_ISSUE359_MULTISLOT before={} after={} dropped={} rows={} partial_code={} sparse_code={}",
        before.len(),
        guarded.hits.len(),
        guarded.dropped_guard_hits.len(),
        candidate_rows.len(),
        partial.code,
        sparse.code,
    );
}

fn multi_slot_fixture() -> (SearchEngine, Vec<calyx_core::Constellation>) {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(slot_content(), 2, 42)).unwrap();
    let mut engine = SearchEngine::new(map);
    insert(&engine, cx(4), dense(vec![1.0, 0.0]), 4);
    insert(&engine, cx(5), dense(vec![0.95, 0.05]), 5);

    let rows = vec![
        row_with_slots(
            cx(4),
            vec![
                (slot_content(), dense(vec![1.0, 0.0])),
                (slot_style(), dense(vec![0.0, 1.0, 0.0])),
            ],
            4,
        ),
        row_with_slots(
            cx(5),
            vec![
                (slot_content(), dense(vec![1.0, 0.0])),
                (slot_style(), dense(vec![1.0, 0.0, 0.0])),
            ],
            5,
        ),
    ];
    for row in &rows {
        engine.put_constellation(row.clone());
    }
    (engine, rows)
}

fn insert(engine: &SearchEngine, cx_id: CxId, vector: SlotVector, seq: u64) {
    engine
        .indexes
        .insert(slot_content(), cx_id, vector, seq)
        .unwrap();
}

fn base_query(k: usize) -> Query {
    let mut query = Query::new("guarded multi-slot")
        .with_slots(vec![slot_content()])
        .with_vector(dense(vec![1.0, 0.0]));
    query.k = k;
    query
}

fn full_guard_query(k: usize) -> Query {
    query_with_guard_vectors(
        k,
        BTreeMap::from([
            (slot_content(), dense(vec![1.0, 0.0])),
            (slot_style(), dense(vec![0.0, 1.0, 0.0])),
        ]),
    )
}

fn query_with_guard_vectors(k: usize, guard_vectors: BTreeMap<SlotId, SlotVector>) -> Query {
    base_query(k)
        .with_guard(QueryGuard::InRegionOnly(multi_slot_profile()))
        .with_guard_vectors(guard_vectors)
        .explain(true)
}

fn multi_slot_profile() -> GuardProfile {
    let mut tau = BTreeMap::new();
    tau.insert(slot_content(), 0.70);
    tau.insert(slot_style(), 0.70);
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic-sextant-multislot-readback".to_string(),
        tau,
        required_slots: vec![slot_content(), slot_style()],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
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
            pointer: Some(format!("zfs://calyx/guarded-multislot-readback/{seq}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("guard-region".to_string()),
            value: AnchorValue::Enum("trusted".to_string()),
            source: "issue359-fsv".to_string(),
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

fn candidate_slot_readback(rows: &[calyx_core::Constellation]) -> serde_json::Value {
    json!({
        "rows": rows
            .iter()
            .map(|row| json!({
                "cx_id": row.cx_id,
                "slot_vectors": row.slots,
            }))
            .collect::<Vec<_>>()
    })
}

fn ids(hits: &[calyx_sextant::Hit]) -> Vec<CxId> {
    hits.iter().map(|hit| hit.cx_id).collect()
}

const fn slot_content() -> SlotId {
    SlotId::new(8)
}

const fn slot_style() -> SlotId {
    SlotId::new(9)
}
