use std::collections::BTreeMap;
use std::path::Path;

use calyx_core::{CalyxError, FixedClock, Input, LensId, Modality, SlotId, SlotVector};
use calyx_forge::{QuantLevel, Quantizer, TurboQuantCodec, new_seed, seed_id_hex};
use calyx_ledger::{
    ActorId, DirectoryLedgerStore, EntryKind, ForgeBackend, FusionMode, FusionWeights, HitRef,
    LedgerAppender, LedgerCfStore, LedgerRow, QueryId, RecordedSlot, RemeasuredSlot,
    ReproduceInputResolver, ReproduceLensRegistry, SlotWeight, SubjectId, VerifyResult, decode,
    reproduce_with_input_resolver, rerun_fusion, verify_chain,
};
use calyx_registry::{AlgorithmicLens, Registry};
use serde_json::{Value, json};

use super::common::{cx, hex, reset_dir};

pub fn run_reproduce_fsv(root: &Path) -> Value {
    let ledger_dir = root.join("reproduce-ledger-cf");
    reset_dir(&ledger_dir);
    let scenario = scenario();
    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap()
        .len();
    write_answer_ledger(
        &ledger_dir,
        &scenario.slots,
        &scenario.answer_id,
        &scenario.fusion,
        &scenario.original_hits,
    );
    let before_reproduce_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap()
        .len();

    let mut store = DirectoryLedgerStore::open(&ledger_dir).unwrap();
    let registry = scenario.registry;
    let resolver = Resolver::from_slots(&scenario.slots);
    let mut forge = DeterministicForge::default();
    let result = reproduce_with_input_resolver(
        &mut store,
        &registry,
        &mut forge,
        &resolver,
        &scenario.answer_id,
    )
    .unwrap();
    let rows = store.scan().unwrap();
    let admin = decode(&rows[3].bytes).unwrap();
    let admin_payload: Value = serde_json::from_slice(&admin.payload).unwrap();

    json!({
        "ledger_dir": ledger_dir,
        "before_rows": before_rows,
        "before_reproduce_rows": before_reproduce_rows,
        "after_reproduce_rows": rows.len(),
        "answer_id_hex": hex(&scenario.answer_id),
        "result": result,
        "admin_payload": admin_payload,
        "original_score_bytes_hex": score_bytes_hex(&result.original_hits),
        "reproduced_score_bytes_hex": score_bytes_hex(&result.reproduced_hits),
        "row_hashes": row_hashes(&rows),
        "chain": chain_readback(&store, rows.len() as u64),
        "registry": {
            "kind": "calyx-registry",
            "lens_ids": registry_lens_ids(&scenario.slots),
            "weights_sha256": registry_weight_hashes(&scenario.slots),
        },
        "forge": {
            "kind": "calyx-forge-turboquant-determinism",
            "seeds": forge.seeds,
            "seed_ids": forge.seed_ids,
        },
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
struct DeterministicForge {
    seeds: Vec<u64>,
    seed_ids: Vec<String>,
}

impl ForgeBackend for DeterministicForge {
    fn activate_determinism(&mut self, seed: u64) -> calyx_core::Result<()> {
        let rotation = new_seed(16, &seed.to_le_bytes());
        let codec =
            TurboQuantCodec::new(rotation.clone(), QuantLevel::Bits3p5).map_err(map_forge)?;
        let vector = deterministic_vector(seed);
        let encoded = codec.encode(&vector).map_err(map_forge)?;
        let decoded = codec.decode(&encoded).map_err(map_forge)?;
        if decoded.len() != vector.len() {
            return Err(CalyxError::forge_numerical_invariant(format!(
                "decoded determinism vector len {} != {}",
                decoded.len(),
                vector.len()
            )));
        }
        let replay = TurboQuantCodec::new(rotation, QuantLevel::Bits3p5).map_err(map_forge)?;
        let encoded_again = replay.encode(&vector).map_err(map_forge)?;
        if encoded != encoded_again {
            return Err(CalyxError::forge_numerical_invariant(
                "TurboQuant deterministic seed replay changed encoded bytes",
            ));
        }
        self.seeds.push(seed);
        self.seed_ids.push(seed_id_hex(&encoded.seed_id));
        Ok(())
    }
}

struct FrozenRegistry {
    inner: Registry,
}

impl FrozenRegistry {
    fn new() -> Self {
        Self {
            inner: Registry::new(),
        }
    }

    fn register_algorithmic(&mut self, name: String, input: &Input) -> (LensId, [u8; 32]) {
        let lens = AlgorithmicLens::byte_features(name, Modality::Text);
        let contract = lens.contract().clone();
        let lens_id = contract.lens_id();
        let weights_sha256 = contract.weights_sha256();
        self.inner
            .register_frozen_with_probe(lens, contract, input)
            .unwrap();
        (lens_id, weights_sha256)
    }

    fn measure(&self, lens_id: LensId, input: &Input) -> SlotVector {
        self.inner.measure(lens_id, input).unwrap()
    }
}

impl ReproduceLensRegistry for FrozenRegistry {
    fn frozen_weights_sha256(&self, lens_id: LensId) -> calyx_core::Result<[u8; 32]> {
        self.inner
            .frozen_contract(lens_id)
            .map(|contract| contract.weights_sha256())
            .ok_or_else(|| {
                calyx_core::CalyxError::lens_frozen_violation(format!(
                    "lens {lens_id} has no frozen snapshot"
                ))
            })
    }

    fn measure_frozen(&self, lens_id: LensId, input: &Input) -> calyx_core::Result<SlotVector> {
        self.inner.measure(lens_id, input)
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

struct Scenario {
    registry: FrozenRegistry,
    slots: Vec<RecordedSlot>,
    answer_id: QueryId,
    fusion: FusionWeights,
    original_hits: Vec<HitRef>,
}

fn scenario() -> Scenario {
    let mut registry = FrozenRegistry::new();
    let slots = (0..2)
        .map(|slot| recorded_slot(slot, &mut registry))
        .collect::<Vec<_>>();
    let candidates = (1..=16).map(cx).collect::<Vec<_>>();
    let fusion = FusionWeights {
        mode: FusionMode::WeightedRrf,
        k: 2,
        candidates,
        weights: vec![slot_weight(0, 1.0), slot_weight(1, 0.5)],
        single_slot: None,
    };
    let remeasured = slots
        .iter()
        .map(|slot| RemeasuredSlot {
            cx_id: slot.cx_id,
            slot_id: slot.slot_id,
            lens_id: slot.lens_id,
            input_hash: slot.input_hash,
            forge_seed: slot.forge_seed,
            vector: registry.measure(slot.lens_id, slot.input.as_ref().unwrap()),
        })
        .collect::<Vec<_>>();
    let original_hits = rerun_fusion(&remeasured, &fusion).unwrap();
    Scenario {
        registry,
        slots,
        answer_id: b"ph36-fsv-answer".to_vec(),
        fusion,
        original_hits,
    }
}

fn recorded_slot(slot: u16, registry: &mut FrozenRegistry) -> RecordedSlot {
    let input = Input::new(Modality::Text, format!("ph36-fsv-slot-{slot}").into_bytes());
    let (lens_id, weights_sha256) =
        registry.register_algorithmic(format!("ph36-fsv-algorithmic-{slot}"), &input);
    RecordedSlot {
        cx_id: cx((slot + 10) as u8),
        slot_id: SlotId::new(slot),
        lens_id,
        weights_sha256,
        input_hash: *blake3::hash(&input.bytes).as_bytes(),
        corpus_shard_hash: None,
        forge_seed: 0xDEAD_BEEF,
        input: Some(input),
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
        VerifyResult::Corrupt { at_seq, reason } => {
            json!({"status": "corrupt", "at_seq": at_seq, "reason": reason})
        }
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

fn registry_lens_ids(slots: &[RecordedSlot]) -> Vec<String> {
    slots.iter().map(|slot| slot.lens_id.to_string()).collect()
}

fn registry_weight_hashes(slots: &[RecordedSlot]) -> Vec<String> {
    slots.iter().map(|slot| hex(&slot.weights_sha256)).collect()
}

fn deterministic_vector(seed: u64) -> Vec<f32> {
    (0..16)
        .map(|idx| {
            let byte = seed.rotate_left(idx as u32).to_le_bytes()[0];
            (f32::from(byte) + 1.0) / 257.0
        })
        .collect()
}

fn map_forge(error: calyx_forge::ForgeError) -> CalyxError {
    CalyxError::forge_numerical_invariant(error.to_string())
}
