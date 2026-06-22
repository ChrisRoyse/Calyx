use calyx_core::{CxId, FixedClock, Input, LensId, Modality, SlotId, SlotVector};
use calyx_ledger::{
    ActorId, EntryKind, FusionMode, FusionWeights, HitRef, LedgerAppender, LedgerCfStore,
    MemoryLedgerStore, QueryId, RecordedSlot, SlotWeight, SubjectId, assert_reproduced,
    assert_within_tolerance, decode, reproduce_with_input_resolver, rerun_fusion,
};
use serde_json::json;

mod reproduce_support;
use reproduce_support::{
    RecordingForge, RecordingRegistry, SlotInputResolver, cx, decode_vector_bytes, dense,
    encode_vector_bytes, hex, rrf,
};

#[test]
fn tolerance_accepts_sub_millidrift() {
    let cx1 = cx(1);
    let cx2 = cx(2);
    let original = vec![hit(cx1, 0.9), hit(cx2, 0.7)];
    let reproduced = vec![hit(cx1, 0.9005), hit(cx2, 0.7002)];

    let (ok, max_drift) = assert_within_tolerance(&original, &reproduced, 1.0e-3);

    assert!(ok);
    assert!((max_drift - 0.0005).abs() < 0.000_001);
}

#[test]
fn tolerance_rejects_excess_drift_and_assertion_has_catalog_code() {
    let cx1 = cx(1);
    let cx2 = cx(2);
    let original = vec![hit(cx1, 0.9), hit(cx2, 0.7)];
    let reproduced = vec![hit(cx1, 0.9015), hit(cx2, 0.7)];

    let (ok, max_drift) = assert_within_tolerance(&original, &reproduced, 1.0e-3);
    let error = assert_reproduced(&calyx_ledger::ReproduceResult {
        reproduced: ok,
        max_drift,
        original_hits: original,
        reproduced_hits: reproduced,
    })
    .unwrap_err();

    assert!(!ok);
    assert!((max_drift - 0.0015).abs() < 0.000_001);
    assert_eq!(error.code, "CALYX_REPRODUCE_DRIFT_EXCEEDED");
}

#[test]
fn tolerance_edges_cover_missing_and_empty_hits() {
    assert_eq!(assert_within_tolerance(&[], &[], 1.0e-3), (true, 0.0));
    assert_eq!(
        assert_within_tolerance(&[hit(cx(1), 1.0)], &[], 1.0e-3),
        (false, 1.0)
    );
    assert_eq!(
        assert_within_tolerance(&[hit(cx(1), 1.0)], &[hit(cx(2), 1.0)], 1.0e-3),
        (false, 1.0)
    );
    assert_eq!(
        assert_within_tolerance(&[hit(cx(1), 0.5)], &[hit(cx(1), 0.5)], 1.0e-3),
        (true, 0.0)
    );
}

#[test]
fn rerun_fusion_applies_weighted_rrf_from_remeasured_vectors() {
    let candidates = vec![cx(1), cx(2)];
    let weights = FusionWeights {
        mode: FusionMode::WeightedRrf,
        k: 2,
        candidates: candidates.clone(),
        weights: vec![
            SlotWeight {
                slot_id: SlotId::new(0),
                weight: 1.0,
            },
            SlotWeight {
                slot_id: SlotId::new(1),
                weight: 0.5,
            },
        ],
        single_slot: None,
    };
    let remeasured = vec![
        remeasured_slot(0, dense(&[0.9, 0.7])),
        remeasured_slot(1, dense(&[0.8, 0.6])),
    ];

    let hits = rerun_fusion(&remeasured, &weights).unwrap();

    assert_eq!(hits[0].cx_id, candidates[0]);
    assert_eq!(hits[1].cx_id, candidates[1]);
    assert!((hits[0].score - (rrf(1.0, 1) + rrf(0.5, 1))).abs() < 0.000_001);
    assert!((hits[1].score - (rrf(1.0, 2) + rrf(0.5, 2))).abs() < 0.000_001);
}

#[test]
fn reproduce_end_to_end_writes_reproduce_admin_entry() {
    let candidates = vec![cx(1), cx(2)];
    let slots = vec![
        recorded_slot(0, 10, b"slot-a", dense(&[0.9, 0.7])),
        recorded_slot(1, 11, b"slot-b", dense(&[0.8, 0.6])),
    ];
    let fusion = fusion_weights(candidates.clone());
    let original = vec![
        hit(candidates[0], rrf(1.0, 1) + rrf(0.5, 1)),
        hit(candidates[1], rrf(1.0, 2) + rrf(0.5, 2)),
    ];
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(500)).unwrap();
    append_measure(&mut appender, &slots[0]);
    append_measure(&mut appender, &slots[1]);
    let answer_id = answer_id();
    append_answer(&mut appender, &answer_id, &[0, 1], &fusion, &original);
    let mut store = appender.into_store();
    let registry = RecordingRegistry::from_slots_with_vectors(&slots, vector_for_encoded_slot);
    let resolver = SlotInputResolver::from_slots(&slots, "missing test input");
    let mut forge = RecordingForge::default();

    let result =
        reproduce_with_input_resolver(&mut store, &registry, &mut forge, &resolver, &answer_id)
            .unwrap();
    let rows = store.scan().unwrap();
    let entry = decode(&rows[3].bytes).unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&entry.payload).unwrap();

    assert!(result.reproduced);
    assert_eq!(result.max_drift, 0.0);
    assert_eq!(rows.len(), 4);
    assert_eq!(entry.kind, EntryKind::Admin);
    assert_eq!(payload["type"], "reproduce_v1");
    assert_eq!(payload["reproduced"], true);
    assert_eq!(payload["max_drift"], 0.0);
    assert_eq!(forge.seeds, vec![10, 11]);
}

#[test]
fn missing_fusion_weights_fails_without_reproduce_row() {
    let slot = recorded_slot(2, 12, b"slot-c", dense(&[1.0]));
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(600)).unwrap();
    append_measure(&mut appender, &slot);
    let answer_id = answer_id();
    appender
        .append(
            EntryKind::Answer,
            SubjectId::Query(answer_id.clone()),
            serde_json::to_vec(&json!({"measure_refs": [0], "original_hits": []})).unwrap(),
            ActorId::Service("reproduce-fusion-test".to_string()),
        )
        .unwrap();
    let mut store = appender.into_store();
    let registry = RecordingRegistry::from_slots_with_vectors(
        std::slice::from_ref(&slot),
        vector_for_encoded_slot,
    );
    let resolver = SlotInputResolver::from_slots(std::slice::from_ref(&slot), "missing test input");
    let mut forge = RecordingForge::default();

    let error =
        reproduce_with_input_resolver(&mut store, &registry, &mut forge, &resolver, &answer_id)
            .unwrap_err();

    assert_eq!(error.code, "CALYX_LEDGER_CORRUPT");
    assert_eq!(store.scan().unwrap().len(), 2);
}

#[test]
fn remeasure_error_is_propagated_without_partial_reproduce_row() {
    let mut slot = recorded_slot(3, 13, b"slot-d", dense(&[1.0]));
    slot.weights_sha256 = [0xee; 32];
    let fusion = FusionWeights {
        mode: FusionMode::Rrf,
        k: 1,
        candidates: vec![cx(1)],
        weights: Vec::new(),
        single_slot: None,
    };
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(700)).unwrap();
    append_measure(&mut appender, &slot);
    let answer_id = answer_id();
    append_answer(
        &mut appender,
        &answer_id,
        &[0],
        &fusion,
        &[hit(cx(1), rrf(1.0, 1))],
    );
    let mut store = appender.into_store();
    let registry = RecordingRegistry::default();
    let resolver = SlotInputResolver::from_slots(std::slice::from_ref(&slot), "missing test input");
    let mut forge = RecordingForge::default();

    let error =
        reproduce_with_input_resolver(&mut store, &registry, &mut forge, &resolver, &answer_id)
            .unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
    assert_eq!(store.scan().unwrap().len(), 2);
}

fn recorded_slot(slot_id: u16, seed: u64, label: &[u8], vector: SlotVector) -> RecordedSlot {
    let input = Input::new(Modality::Text, encode_vector_bytes(label, &vector));
    RecordedSlot {
        cx_id: cx((slot_id + 10) as u8),
        slot_id: SlotId::new(slot_id),
        lens_id: LensId::from_bytes([0xa0 | slot_id as u8; 16]),
        weights_sha256: [0x40 | slot_id as u8; 32],
        input_hash: *blake3::hash(&input.bytes).as_bytes(),
        corpus_shard_hash: None,
        forge_seed: seed,
        input: Some(input),
    }
}

fn append_measure<S, C>(appender: &mut LedgerAppender<S, C>, slot: &RecordedSlot)
where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    appender
        .append(
            EntryKind::Measure,
            SubjectId::Cx(slot.cx_id),
            serde_json::to_vec(&json!({
                "cx_id": slot.cx_id.to_string(),
                "slot_id": slot.slot_id.get(),
                "lens_id": slot.lens_id.to_string(),
                "weights_sha256": hex(&slot.weights_sha256),
                "input_hash": hex(&slot.input_hash),
                "forge_seed": slot.forge_seed,
            }))
            .unwrap(),
            ActorId::Service("reproduce-fusion-test".to_string()),
        )
        .unwrap();
}

fn append_answer<S, C>(
    appender: &mut LedgerAppender<S, C>,
    answer_id: &QueryId,
    refs: &[u64],
    fusion: &FusionWeights,
    original_hits: &[HitRef],
) where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    appender
        .append(
            EntryKind::Answer,
            SubjectId::Query(answer_id.clone()),
            serde_json::to_vec(&json!({
                "measure_refs": refs,
                "fusion_weights": fusion,
                "original_hits": original_hits,
            }))
            .unwrap(),
            ActorId::Service("reproduce-fusion-test".to_string()),
        )
        .unwrap();
}

fn fusion_weights(candidates: Vec<CxId>) -> FusionWeights {
    FusionWeights {
        mode: FusionMode::WeightedRrf,
        k: 2,
        candidates,
        weights: vec![
            SlotWeight {
                slot_id: SlotId::new(0),
                weight: 1.0,
            },
            SlotWeight {
                slot_id: SlotId::new(1),
                weight: 0.5,
            },
        ],
        single_slot: None,
    }
}

fn remeasured_slot(slot: u16, vector: SlotVector) -> calyx_ledger::RemeasuredSlot {
    calyx_ledger::RemeasuredSlot {
        cx_id: cx((slot + 10) as u8),
        slot_id: SlotId::new(slot),
        lens_id: LensId::from_bytes([slot as u8; 16]),
        input_hash: [slot as u8; 32],
        forge_seed: u64::from(slot),
        vector,
    }
}

fn hit(cx_id: CxId, score: f32) -> HitRef {
    HitRef { cx_id, score }
}

fn vector_for_encoded_slot(slot: &RecordedSlot) -> SlotVector {
    decode_vector_bytes(&slot.input.as_ref().unwrap().bytes)
}

fn answer_id() -> QueryId {
    b"answer-fusion-test".to_vec()
}
