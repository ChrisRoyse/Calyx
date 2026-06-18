use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, FixedClock, Input, LensId, Modality, SlotId, SlotVector};
use calyx_ledger::{
    ActorId, DirectoryLedgerStore, EntryKind, ForgeBackend, LedgerAppender, LedgerCfStore,
    MemoryLedgerStore, QueryId, RecordedSlot, ReproduceContext, ReproduceInputResolver,
    ReproduceLensRegistry, SubjectId, build_reproduce_context, decode, lookup_frozen_lens,
    remeasure_slots, remeasure_slots_with_input_resolver,
};
use serde_json::json;

#[test]
fn remeasure_slots_is_idempotent_for_same_seed() {
    let ctx = context_with_slots(vec![slot(1, 9, b"alpha"), slot(2, 10, b"bravo")]);
    let registry = MockRegistry::from_slots(&ctx.recorded_slots);
    let mut forge = MockForge::default();

    let first = remeasure_slots(&ctx, &registry, &mut forge).unwrap();
    let second = remeasure_slots(&ctx, &registry, &mut forge).unwrap();

    assert_eq!(first, second);
    assert_eq!(forge.seeds, vec![9, 10, 9, 10]);
    assert_eq!(first[0].vector, vector_for(b"alpha"));
}

#[test]
fn weights_mismatch_fails_with_frozen_violation() {
    let slot = slot(3, 8, b"charlie");
    let mut registry = MockRegistry::from_slots(std::slice::from_ref(&slot));
    registry.weights.insert(slot.lens_id, [0xff; 32]);

    let error = lookup_frozen_lens(&registry, slot.lens_id, &slot.weights_sha256).unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
}

#[test]
fn forge_seed_zero_is_activated_and_repeatable() {
    let ctx = context_with_slots(vec![slot(4, 0, b"delta")]);
    let registry = MockRegistry::from_slots(&ctx.recorded_slots);
    let mut forge = MockForge::default();

    let first = remeasure_slots(&ctx, &registry, &mut forge).unwrap();
    let second = remeasure_slots(&ctx, &registry, &mut forge).unwrap();

    assert_eq!(forge.seeds, vec![0, 0]);
    assert_eq!(first, second);
}

#[test]
fn empty_recorded_slots_returns_empty_remeasurement() {
    let ctx = context_with_slots(Vec::new());
    let registry = MockRegistry::default();
    let mut forge = MockForge::default();

    let out = remeasure_slots(&ctx, &registry, &mut forge).unwrap();

    assert!(out.is_empty());
    assert!(forge.seeds.is_empty());
}

#[test]
fn retired_lens_succeeds_only_when_frozen_snapshot_is_present() {
    let slot = slot(5, 12, b"echo");
    let present = MockRegistry::from_slots(std::slice::from_ref(&slot));
    let absent = MockRegistry::default();
    let mut forge = MockForge::default();

    let ok = remeasure_slots(
        &context_with_slots(vec![slot.clone()]),
        &present,
        &mut forge,
    )
    .unwrap();
    let error = remeasure_slots(&context_with_slots(vec![slot]), &absent, &mut forge).unwrap_err();

    assert_eq!(ok.len(), 1);
    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
}

#[test]
fn missing_forge_seed_in_measure_payload_fails_closed() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(100)).unwrap();
    append_measure_without_seed(&mut appender);
    let answer_id = answer_id();
    append_answer(&mut appender, &answer_id, vec![0]);

    let store = appender.into_store();
    let error = build_reproduce_context(&store, &answer_id).unwrap_err();

    assert_eq!(error.code, "CALYX_REPRODUCE_NONDETERMINISTIC");
}

#[test]
fn build_reproduce_context_reads_answer_and_measure_refs() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(200)).unwrap();
    let slots = [slot(6, 16, b"foxtrot"), slot(7, 17, b"golf")];
    append_measure(&mut appender, &slots[0]);
    append_measure(&mut appender, &slots[1]);
    let answer_id = answer_id();
    append_answer(&mut appender, &answer_id, vec![0, 1]);

    let ctx = build_reproduce_context(&appender.into_store(), &answer_id).unwrap();
    let mut expected = slots.to_vec();
    for slot in &mut expected {
        slot.input = None;
    }

    assert_eq!(ctx.ledger_entries.len(), 3);
    assert_eq!(ctx.recorded_slots, expected);
}

#[test]
fn resolved_input_hash_mismatch_fails_closed() {
    let mut slot = slot(8, 18, b"hotel");
    slot.input_hash = [0xaa; 32];
    let ctx = context_with_slots(vec![slot]);
    let registry = MockRegistry::from_slots(&ctx.recorded_slots);
    let mut forge = MockForge::default();

    let error = remeasure_slots(&ctx, &registry, &mut forge).unwrap_err();

    assert_eq!(error.code, "CALYX_LEDGER_CORRUPT");
}

#[test]
#[ignore = "manual gpuhost FSV for PH36 reproduce re-measure bytes"]
fn reproduce_remeasure_gpuhost_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).expect("create fsv root");
    let ledger_dir = root.join("ledger-cf");
    reset_child_dir(&root, &ledger_dir);

    let input = Input::new(Modality::Text, b"gpuhost reproduce slot zero".to_vec());
    let slot = slot_with_input(9, 0, 52, input.clone());
    let expected = vector_for(&input.bytes);
    let answer_id = answer_id();
    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap()
        .len();

    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(300),
    )
    .unwrap();
    append_measure(&mut appender, &slot);
    append_answer(&mut appender, &answer_id, vec![0]);
    drop(appender);

    let store = DirectoryLedgerStore::open(&ledger_dir).unwrap();
    let ctx = build_reproduce_context(&store, &answer_id).unwrap();
    let registry = MockRegistry::from_slots(std::slice::from_ref(&slot));
    let resolver = SingleInputResolver { input };
    let mut forge = MockForge::default();
    let remeasured =
        remeasure_slots_with_input_resolver(&ctx, &registry, &mut forge, &resolver).unwrap();

    let mut bad_registry = MockRegistry::from_slots(std::slice::from_ref(&slot));
    bad_registry.weights.insert(slot.lens_id, [0xee; 32]);
    let mismatch = remeasure_slots_with_input_resolver(
        &ctx,
        &bad_registry,
        &mut MockForge::default(),
        &resolver,
    )
    .unwrap_err();

    let rows = store.scan().unwrap();
    let max_diff = max_abs_diff(&expected, &remeasured[0].vector);
    let readback = json!({
        "before_rows": before_rows,
        "after_rows": rows.len(),
        "answer_id_hex": hex(&answer_id),
        "ledger_rows": rows.iter().map(|row| {
            let entry = decode(&row.bytes).unwrap();
            json!({
                "seq": row.seq,
                "kind": entry.kind.as_str(),
                "bytes_sha256": hex(blake3::hash(&row.bytes).as_bytes()),
            })
        }).collect::<Vec<_>>(),
        "recorded_slots": ctx.recorded_slots.len(),
        "original_slot0": expected,
        "remeasured_slot0": remeasured[0].vector,
        "slot0_max_abs_diff": max_diff,
        "weights_mismatch_error": mismatch.code,
        "forge_seeds": forge.seeds,
    });
    let path = root.join("reproduce-remeasure-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();

    println!("PH36_REPRODUCE_FSV_ROOT={}", root.display());
    println!("PH36_REPRODUCE_READBACK={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before_rows, 0);
    assert_eq!(rows.len(), 2);
    assert!(max_diff <= 1.0e-3);
    assert_eq!(mismatch.code, "CALYX_LENS_FROZEN_VIOLATION");
}

#[derive(Default)]
struct MockForge {
    seeds: Vec<u64>,
}

impl ForgeBackend for MockForge {
    fn activate_determinism(&mut self, seed: u64) -> calyx_core::Result<()> {
        self.seeds.push(seed);
        Ok(())
    }
}

#[derive(Default)]
struct MockRegistry {
    weights: BTreeMap<LensId, [u8; 32]>,
}

impl MockRegistry {
    fn from_slots(slots: &[RecordedSlot]) -> Self {
        Self {
            weights: slots
                .iter()
                .map(|slot| (slot.lens_id, slot.weights_sha256))
                .collect(),
        }
    }
}

impl ReproduceLensRegistry for MockRegistry {
    fn frozen_weights_sha256(&self, lens_id: LensId) -> calyx_core::Result<[u8; 32]> {
        self.weights.get(&lens_id).copied().ok_or_else(|| {
            calyx_core::CalyxError::lens_frozen_violation(format!(
                "lens {lens_id} has no frozen snapshot"
            ))
        })
    }

    fn measure_frozen(&self, _lens_id: LensId, input: &Input) -> calyx_core::Result<SlotVector> {
        Ok(vector_for(&input.bytes))
    }
}

struct SingleInputResolver {
    input: Input,
}

impl ReproduceInputResolver for SingleInputResolver {
    fn resolve_input(&self, _slot: &RecordedSlot) -> calyx_core::Result<Input> {
        Ok(self.input.clone())
    }
}

fn context_with_slots(recorded_slots: Vec<RecordedSlot>) -> ReproduceContext {
    ReproduceContext {
        answer_id: answer_id(),
        ledger_entries: Vec::new(),
        recorded_slots,
    }
}

fn slot(seed: u8, forge_seed: u64, bytes: &[u8]) -> RecordedSlot {
    slot_with_input(
        seed,
        seed as u16,
        forge_seed,
        Input::new(Modality::Text, bytes.to_vec()),
    )
}

fn slot_with_input(seed: u8, slot_id: u16, forge_seed: u64, input: Input) -> RecordedSlot {
    RecordedSlot {
        cx_id: CxId::from_bytes([seed; 16]),
        slot_id: SlotId::new(slot_id),
        lens_id: LensId::from_bytes([0x80 | seed; 16]),
        weights_sha256: [0x40 | seed; 32],
        input_hash: *blake3::hash(&input.bytes).as_bytes(),
        corpus_shard_hash: Some([0x20 | seed; 32]),
        forge_seed,
        input: Some(input),
    }
}

fn append_measure<S, C>(appender: &mut LedgerAppender<S, C>, slot: &RecordedSlot)
where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    let mut payload = slot_payload(slot);
    payload.as_object_mut().unwrap().remove("input");
    appender
        .append(
            EntryKind::Measure,
            SubjectId::Cx(slot.cx_id),
            serde_json::to_vec(&payload).unwrap(),
            ActorId::Service("reproduce-test".to_string()),
        )
        .unwrap();
}

fn append_measure_without_seed<S, C>(appender: &mut LedgerAppender<S, C>)
where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    let slot = slot(10, 44, b"india");
    let mut payload = slot_payload(&slot);
    payload.as_object_mut().unwrap().remove("forge_seed");
    payload.as_object_mut().unwrap().remove("input");
    appender
        .append(
            EntryKind::Measure,
            SubjectId::Cx(slot.cx_id),
            serde_json::to_vec(&payload).unwrap(),
            ActorId::Service("reproduce-test".to_string()),
        )
        .unwrap();
}

fn append_answer<S, C>(appender: &mut LedgerAppender<S, C>, answer_id: &QueryId, refs: Vec<u64>)
where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    appender
        .append(
            EntryKind::Answer,
            SubjectId::Query(answer_id.clone()),
            serde_json::to_vec(&json!({"measure_refs": refs})).unwrap(),
            ActorId::Service("reproduce-test".to_string()),
        )
        .unwrap();
}

fn slot_payload(slot: &RecordedSlot) -> serde_json::Value {
    json!({
        "cx_id": slot.cx_id.to_string(),
        "slot_id": slot.slot_id.get(),
        "lens_id": slot.lens_id.to_string(),
        "weights_sha256": hex(&slot.weights_sha256),
        "input_hash": hex(&slot.input_hash),
        "corpus_shard_hash": hex(&slot.corpus_shard_hash.unwrap()),
        "forge_seed": slot.forge_seed,
        "input": slot.input,
    })
}

fn vector_for(bytes: &[u8]) -> SlotVector {
    SlotVector::Dense {
        dim: 3,
        data: vec![
            bytes.len() as f32,
            bytes.first().copied().unwrap_or(0) as f32,
            1.0,
        ],
    }
}

fn max_abs_diff(a: &SlotVector, b: &SlotVector) -> f32 {
    let (Some(a), Some(b)) = (a.as_dense(), b.as_dense()) else {
        return f32::INFINITY;
    };
    a.iter()
        .zip(b.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max)
}

fn answer_id() -> QueryId {
    b"answer-reproduce-test".to_vec()
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph36-reproduce-fsv"))
}

fn reset_child_dir(root: &Path, child: &Path) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if child.exists() {
        let child_canonical = child.canonicalize().expect("canonical child path");
        assert!(child_canonical.starts_with(&root));
        fs::remove_dir_all(&child_canonical).expect("remove stale child dir");
    }
    fs::create_dir_all(child).expect("create child dir");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
