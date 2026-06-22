use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{FixedClock, Input, LensId, Modality, SlotId, SlotVector};
use calyx_ledger::{
    ActorId, DirectoryLedgerStore, EntryKind, FusionMode, FusionWeights, HitRef, LedgerAppender,
    LedgerCfStore, LedgerRow, QueryId, RecordedSlot, SlotWeight, SubjectId, VerifyResult,
    assert_reproduced, decode, reproduce_with_input_resolver, verify_chain,
};
use serde_json::{Value, json};

mod reproduce_support;
use reproduce_support::{
    RecordingForge, RecordingRegistry, SlotInputResolver, cx, dense, hex, reset_child_dir, rrf,
};

#[test]
#[ignore = "manual FSV for PH36 reproduce fusion ledger bytes"]
fn reproduce_fusion_manual_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).expect("create fsv root");

    let happy = run_happy_path(&root);
    let missing = run_missing_fusion_edge(&root);
    let drift = run_drift_edge(&root);
    let remeasure = run_remeasure_error_edge(&root);

    let readback = json!({
        "happy_path": happy,
        "missing_fusion_weights_edge": missing,
        "drift_exceeded_edge": drift,
        "remeasure_error_edge": remeasure,
    });
    let path = root.join("reproduce-fusion-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();

    println!("PH36_REPRODUCE_FUSION_FSV_ROOT={}", root.display());
    println!("PH36_REPRODUCE_FUSION_READBACK={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["happy_path"]["before_rows"], 0);
    assert_eq!(readback["happy_path"]["after_reproduce_rows"], 4);
    assert_eq!(
        readback["happy_path"]["admin_payload"]["type"],
        "reproduce_v1"
    );
    assert_eq!(readback["happy_path"]["result"]["reproduced"], true);
    assert_eq!(readback["happy_path"]["result"]["max_drift"], 0.0);
    assert_eq!(
        readback["missing_fusion_weights_edge"]["error"],
        "CALYX_LEDGER_CORRUPT"
    );
    assert_eq!(
        readback["missing_fusion_weights_edge"]["after_rows"],
        readback["missing_fusion_weights_edge"]["before_reproduce_rows"]
    );
    assert_eq!(
        readback["drift_exceeded_edge"]["assert_error"],
        "CALYX_REPRODUCE_DRIFT_EXCEEDED"
    );
    assert_eq!(
        readback["drift_exceeded_edge"]["result"]["reproduced"],
        false
    );
    assert_eq!(
        readback["remeasure_error_edge"]["error"],
        "CALYX_LENS_FROZEN_VIOLATION"
    );
    assert_eq!(
        readback["remeasure_error_edge"]["after_rows"],
        readback["remeasure_error_edge"]["before_reproduce_rows"]
    );
}

fn run_happy_path(root: &Path) -> Value {
    let ledger_dir = root.join("happy-ledger-cf");
    reset_child_dir(root, &ledger_dir);
    let (slots, answer_id, fusion, original_hits) = scenario();
    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap()
        .len();
    write_answer_ledger(&ledger_dir, &slots, &answer_id, &fusion, &original_hits);
    let before_reproduce_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap()
        .len();

    let mut store = DirectoryLedgerStore::open(&ledger_dir).unwrap();
    let registry = RecordingRegistry::from_slots_with_vectors(&slots, vector_for_recorded_slot);
    let resolver = SlotInputResolver::from_slots(&slots, "missing fsv input");
    let mut forge = RecordingForge::default();
    let result =
        reproduce_with_input_resolver(&mut store, &registry, &mut forge, &resolver, &answer_id)
            .unwrap();
    let rows = store.scan().unwrap();
    let admin = decode(&rows[3].bytes).unwrap();
    let admin_payload: Value = serde_json::from_slice(&admin.payload).unwrap();

    json!({
        "before_rows": before_rows,
        "before_reproduce_rows": before_reproduce_rows,
        "after_reproduce_rows": rows.len(),
        "answer_id_hex": hex(&answer_id),
        "row_files": row_files(&rows),
        "rows": row_readback(&rows),
        "chain": chain_readback(&store, rows.len() as u64),
        "forge_seeds": forge.seeds,
        "result": result,
        "admin_payload": admin_payload,
    })
}

fn run_missing_fusion_edge(root: &Path) -> Value {
    let ledger_dir = root.join("missing-fusion-ledger-cf");
    reset_child_dir(root, &ledger_dir);
    let (slots, answer_id, _fusion, original_hits) = scenario();
    write_answer_payload(
        &ledger_dir,
        &slots,
        &answer_id,
        json!({"measure_refs": [0, 1], "original_hits": original_hits}),
    );
    let before_reproduce_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap()
        .len();

    let mut store = DirectoryLedgerStore::open(&ledger_dir).unwrap();
    let error = reproduce_with_input_resolver(
        &mut store,
        &RecordingRegistry::from_slots_with_vectors(&slots, vector_for_recorded_slot),
        &mut RecordingForge::default(),
        &SlotInputResolver::from_slots(&slots, "missing fsv input"),
        &answer_id,
    )
    .unwrap_err();
    let rows = store.scan().unwrap();

    json!({
        "before_reproduce_rows": before_reproduce_rows,
        "after_rows": rows.len(),
        "error": error.code,
        "rows": row_readback(&rows),
    })
}

fn run_drift_edge(root: &Path) -> Value {
    let ledger_dir = root.join("drift-ledger-cf");
    reset_child_dir(root, &ledger_dir);
    let (slots, answer_id, fusion, mut original_hits) = scenario();
    original_hits[0].score += 0.01;
    write_answer_ledger(&ledger_dir, &slots, &answer_id, &fusion, &original_hits);

    let mut store = DirectoryLedgerStore::open(&ledger_dir).unwrap();
    let result = reproduce_with_input_resolver(
        &mut store,
        &RecordingRegistry::from_slots_with_vectors(&slots, vector_for_recorded_slot),
        &mut RecordingForge::default(),
        &SlotInputResolver::from_slots(&slots, "missing fsv input"),
        &answer_id,
    )
    .unwrap();
    let assert_error = assert_reproduced(&result).unwrap_err();
    let rows = store.scan().unwrap();
    let admin_payload: Value = serde_json::from_slice(&decode(&rows[3].bytes).unwrap().payload)
        .expect("decode drift reproduce payload");

    json!({
        "after_rows": rows.len(),
        "result": result,
        "assert_error": assert_error.code,
        "admin_payload": admin_payload,
        "rows": row_readback(&rows),
    })
}

fn run_remeasure_error_edge(root: &Path) -> Value {
    let ledger_dir = root.join("remeasure-error-ledger-cf");
    reset_child_dir(root, &ledger_dir);
    let (slots, answer_id, fusion, original_hits) = scenario();
    write_answer_ledger(&ledger_dir, &slots, &answer_id, &fusion, &original_hits);
    let before_reproduce_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap()
        .len();

    let mut registry = RecordingRegistry::from_slots_with_vectors(&slots, vector_for_recorded_slot);
    registry.weights.insert(slots[0].lens_id, [0xee; 32]);
    let mut store = DirectoryLedgerStore::open(&ledger_dir).unwrap();
    let error = reproduce_with_input_resolver(
        &mut store,
        &registry,
        &mut RecordingForge::default(),
        &SlotInputResolver::from_slots(&slots, "missing fsv input"),
        &answer_id,
    )
    .unwrap_err();
    let rows = store.scan().unwrap();

    json!({
        "before_reproduce_rows": before_reproduce_rows,
        "after_rows": rows.len(),
        "error": error.code,
        "rows": row_readback(&rows),
    })
}

fn write_answer_ledger(
    ledger_dir: &Path,
    slots: &[RecordedSlot],
    answer_id: &QueryId,
    fusion: &FusionWeights,
    original_hits: &[HitRef],
) {
    write_answer_payload(
        ledger_dir,
        slots,
        answer_id,
        json!({"measure_refs": [0, 1], "fusion_weights": fusion, "original_hits": original_hits}),
    );
}

fn write_answer_payload(
    ledger_dir: &Path,
    slots: &[RecordedSlot],
    answer_id: &QueryId,
    payload: Value,
) {
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(ledger_dir).unwrap(),
        FixedClock::new(1_000),
    )
    .unwrap();
    for slot in slots {
        append_measure(&mut appender, slot);
    }
    appender
        .append(
            EntryKind::Answer,
            SubjectId::Query(answer_id.clone()),
            serde_json::to_vec(&payload).unwrap(),
            ActorId::Service("reproduce-fusion-fsv".to_string()),
        )
        .unwrap();
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
            ActorId::Service("reproduce-fusion-fsv".to_string()),
        )
        .unwrap();
}

fn scenario() -> (Vec<RecordedSlot>, QueryId, FusionWeights, Vec<HitRef>) {
    let candidates = vec![cx(1), cx(2)];
    let slots = vec![recorded_slot(0, 101), recorded_slot(1, 102)];
    let fusion = FusionWeights {
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
    let original_hits = vec![
        HitRef {
            cx_id: candidates[0],
            score: rrf(1.0, 1) + rrf(0.5, 1),
        },
        HitRef {
            cx_id: candidates[1],
            score: rrf(1.0, 2) + rrf(0.5, 2),
        },
    ];
    (slots, b"answer-fusion-fsv".to_vec(), fusion, original_hits)
}

fn recorded_slot(slot: u16, seed: u64) -> RecordedSlot {
    let input = Input::new(
        Modality::Text,
        format!("fusion-fsv-slot-{slot}").into_bytes(),
    );
    RecordedSlot {
        cx_id: cx((slot + 10) as u8),
        slot_id: SlotId::new(slot),
        lens_id: LensId::from_bytes([0xa0 | slot as u8; 16]),
        weights_sha256: [0x40 | slot as u8; 32],
        input_hash: *blake3::hash(&input.bytes).as_bytes(),
        corpus_shard_hash: None,
        forge_seed: seed,
        input: Some(input),
    }
}

fn vector_for_slot(slot: SlotId) -> SlotVector {
    match slot.get() {
        0 => dense(&[0.9, 0.7]),
        1 => dense(&[0.8, 0.6]),
        _ => dense(&[]),
    }
}

fn vector_for_recorded_slot(slot: &RecordedSlot) -> SlotVector {
    vector_for_slot(slot.slot_id)
}

fn row_readback(rows: &[LedgerRow]) -> Vec<Value> {
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let entry = decode(&row.bytes).unwrap();
            json!({
                "seq": row.seq,
                "encoded_seq": entry.seq,
                "kind": entry.kind.as_str(),
                "payload": serde_json::from_slice::<Value>(&entry.payload).unwrap_or(Value::Null),
                "prev_matches_prior": index == 0 && entry.prev_hash == [0; 32]
                    || index > 0 && entry.prev_hash == decode(&rows[index - 1].bytes).unwrap().entry_hash,
                "entry_hash": hex(&entry.entry_hash),
                "bytes_blake3": hex(blake3::hash(&row.bytes).as_bytes()),
            })
        })
        .collect()
}

fn chain_readback(store: &DirectoryLedgerStore, end: u64) -> Value {
    match verify_chain(store, 0..end).unwrap() {
        VerifyResult::Intact { count } => json!({"status": "intact", "count": count}),
        VerifyResult::Broken {
            at_seq,
            expected,
            found,
        } => json!({
            "status": "broken",
            "at_seq": at_seq,
            "expected": hex(&expected),
            "found": hex(&found),
        }),
        VerifyResult::Corrupt { at_seq, reason } => json!({
            "status": "corrupt",
            "at_seq": at_seq,
            "reason": reason,
        }),
    }
}

fn row_files(rows: &[LedgerRow]) -> Vec<String> {
    rows.iter()
        .map(|row| format!("{:016x}.ledger", row.seq))
        .collect()
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph36-reproduce-fusion-fsv"))
}
