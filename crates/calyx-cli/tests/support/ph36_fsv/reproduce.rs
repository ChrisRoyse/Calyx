use std::collections::BTreeMap;
use std::path::Path;

use calyx_core::{FixedClock, Input, LensId, Modality, SlotId, SlotVector};
use calyx_ledger::{
    ActorId, DirectoryLedgerStore, EntryKind, ForgeBackend, FusionMode, FusionWeights, HitRef,
    LedgerAppender, LedgerCfStore, LedgerRow, QueryId, RecordedSlot, ReproduceInputResolver,
    ReproduceLensRegistry, SlotWeight, SubjectId, VerifyResult, decode,
    reproduce_with_input_resolver, verify_chain,
};
use serde_json::{Value, json};

use super::common::{cx, dense, hex, hit, reset_dir};

pub fn run_reproduce_fsv(root: &Path) -> Value {
    let ledger_dir = root.join("reproduce-ledger-cf");
    reset_dir(&ledger_dir);
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
    let registry = MockRegistry::from_slots(&slots);
    let resolver = Resolver::from_slots(&slots);
    let mut forge = MockForge::default();
    let result =
        reproduce_with_input_resolver(&mut store, &registry, &mut forge, &resolver, &answer_id)
            .unwrap();
    let rows = store.scan().unwrap();
    let admin = decode(&rows[3].bytes).unwrap();
    let admin_payload: Value = serde_json::from_slice(&admin.payload).unwrap();

    json!({
        "ledger_dir": ledger_dir,
        "before_rows": before_rows,
        "before_reproduce_rows": before_reproduce_rows,
        "after_reproduce_rows": rows.len(),
        "answer_id_hex": hex(&answer_id),
        "result": result,
        "admin_payload": admin_payload,
        "original_score_bytes_hex": score_bytes_hex(&result.original_hits),
        "reproduced_score_bytes_hex": score_bytes_hex(&result.reproduced_hits),
        "row_hashes": row_hashes(&rows),
        "chain": chain_readback(&store, rows.len() as u64),
        "forge_seeds": forge.seeds,
    })
}

fn write_answer_ledger(
    ledger_dir: &Path,
    slots: &[RecordedSlot],
    answer_id: &QueryId,
    fusion: &FusionWeights,
    original_hits: &[HitRef],
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
            serde_json::to_vec(&json!({
                "measure_refs": [0, 1],
                "fusion_weights": fusion,
                "original_hits": original_hits,
            }))
            .unwrap(),
            ActorId::Service("ph36-fsv-integration".to_string()),
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
            ActorId::Service("ph36-fsv-integration".to_string()),
        )
        .unwrap();
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
    vectors: BTreeMap<LensId, SlotVector>,
}

impl MockRegistry {
    fn from_slots(slots: &[RecordedSlot]) -> Self {
        Self {
            weights: slots
                .iter()
                .map(|slot| (slot.lens_id, slot.weights_sha256))
                .collect(),
            vectors: slots
                .iter()
                .map(|slot| (slot.lens_id, vector_for_slot(slot.slot_id)))
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

    fn measure_frozen(&self, lens_id: LensId, _input: &Input) -> calyx_core::Result<SlotVector> {
        self.vectors
            .get(&lens_id)
            .cloned()
            .ok_or_else(|| calyx_core::CalyxError::lens_unreachable("missing vector"))
    }
}

struct Resolver {
    inputs: BTreeMap<[u8; 32], Input>,
}

impl Resolver {
    fn from_slots(slots: &[RecordedSlot]) -> Self {
        Self {
            inputs: slots
                .iter()
                .map(|slot| (slot.input_hash, slot.input.clone().unwrap()))
                .collect(),
        }
    }
}

impl ReproduceInputResolver for Resolver {
    fn resolve_input(&self, slot: &RecordedSlot) -> calyx_core::Result<Input> {
        self.inputs
            .get(&slot.input_hash)
            .cloned()
            .ok_or_else(|| calyx_core::CalyxError::ledger_corrupt("missing fsv input"))
    }
}

fn scenario() -> (Vec<RecordedSlot>, QueryId, FusionWeights, Vec<HitRef>) {
    let candidates = vec![cx(1), cx(2)];
    let slots = vec![recorded_slot(0), recorded_slot(1)];
    let fusion = FusionWeights {
        mode: FusionMode::WeightedRrf,
        k: 2,
        candidates: candidates.clone(),
        weights: vec![slot_weight(0, 1.0), slot_weight(1, 0.5)],
        single_slot: None,
    };
    let original_hits = vec![
        hit(candidates[0], rrf(1.0, 1) + rrf(0.5, 1)),
        hit(candidates[1], rrf(1.0, 2) + rrf(0.5, 2)),
    ];
    (slots, b"ph36-fsv-answer".to_vec(), fusion, original_hits)
}

fn recorded_slot(slot: u16) -> RecordedSlot {
    let input = Input::new(Modality::Text, format!("ph36-fsv-slot-{slot}").into_bytes());
    RecordedSlot {
        cx_id: cx((slot + 10) as u8),
        slot_id: SlotId::new(slot),
        lens_id: LensId::from_bytes([0xa0 | slot as u8; 16]),
        weights_sha256: [0x40 | slot as u8; 32],
        input_hash: *blake3::hash(&input.bytes).as_bytes(),
        corpus_shard_hash: None,
        forge_seed: 0xDEAD_BEEF,
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

fn row_hashes(rows: &[LedgerRow]) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            let entry = decode(&row.bytes).unwrap();
            json!({"seq": row.seq, "kind": entry.kind.as_str(), "entry_hash": hex(&entry.entry_hash)})
        })
        .collect()
}

fn chain_readback(store: &DirectoryLedgerStore, end: u64) -> Value {
    match verify_chain(store, 0..end).unwrap() {
        VerifyResult::Intact { count } => json!({"status": "intact", "count": count}),
        VerifyResult::Broken { at_seq, .. } => json!({"status": "broken", "at_seq": at_seq}),
    }
}

fn score_bytes_hex(hits: &[HitRef]) -> Vec<String> {
    hits.iter()
        .map(|hit| hex(&hit.score.to_le_bytes()))
        .collect()
}

fn slot_weight(slot: u16, weight: f32) -> SlotWeight {
    SlotWeight {
        slot_id: SlotId::new(slot),
        weight,
    }
}

fn rrf(weight: f32, rank: usize) -> f32 {
    weight / (rank as f32 + 60.0)
}
