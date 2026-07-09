//! Full-state verification for the reproduce write path's head-anchor invariant.
//!
//! Regression guard for issue #1300: `append_reproduce_entry` used to write a
//! ledger row via `put_new` while ignoring `head_anchor()` and never calling
//! `put_head_anchor`. On an anchor-backed store that left the external head
//! witness stale, so the next `LedgerAppender::open` recovered
//! `next_seq = anchor.height` and the next commit collided at the reproduce row,
//! permanently wedging the ledger with `CALYX_LEDGER_APPEND_ONLY_VIOLATION`.
//!
//! These tests verify against the physical source of truth: the on-disk
//! `_head_anchor.json` witness written by `DirectoryLedgerStore`, the ledger row
//! files, and a full `verify_chain` pass. They are real regression tests (not
//! `#[ignore]`d manual FSV) so the invariant is enforced in CI.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{FixedClock, Input, LensId, Modality, SlotId, SlotVector};
use calyx_ledger::{
    ActorId, DirectoryLedgerStore, EntryKind, FusionMode, FusionWeights, HitRef, LedgerAppender,
    LedgerCfStore, LedgerHeadAnchor, QueryId, RecordedSlot, ReproduceResult, SlotWeight, SubjectId,
    VerifyResult, append_reproduce_entry, decode, reproduce_with_input_resolver, verify_chain,
};
use serde_json::json;

mod reproduce_support;
use reproduce_support::{
    RecordingForge, RecordingRegistry, SlotInputResolver, cx, dense, hex, rrf,
};

const ACTOR: &str = "reproduce-anchor-fsv";

/// Happy path: a full reproduce run advances the external head witness so a
/// later appender open recovers the true tip and does NOT wedge.
#[test]
fn reproduce_append_advances_head_anchor_and_does_not_wedge() {
    let dir = scratch_dir("happy");
    let (slots, answer_id, fusion, original_hits) = scenario();

    // Seed the ledger through the single write path: 2 measures + 1 answer.
    // `LedgerAppender::commit_prepared` advances the anchor to height 3.
    write_answer_ledger(&dir, &slots, &answer_id, &fusion, &original_hits);
    let anchor_before = read_anchor(&dir);
    assert_eq!(
        anchor_before.height, 3,
        "answer seed leaves anchor at height 3"
    );
    let rows_before = scan_rows(&dir);
    assert_eq!(rows_before.len(), 3);

    // Run the reproduce write path. It must append row seq 3 AND advance the anchor.
    let mut store = DirectoryLedgerStore::open(&dir).unwrap();
    let registry = RecordingRegistry::from_slots_with_vectors(&slots, vector_for_recorded_slot);
    let resolver = SlotInputResolver::from_slots(&slots, "missing anchor-fsv input");
    let mut forge = RecordingForge::default();
    let result =
        reproduce_with_input_resolver(&mut store, &registry, &mut forge, &resolver, &answer_id)
            .expect("reproduce append");
    assert!(result.reproduced, "scenario reproduces bit-parity");

    // --- Full state verification against the physical source of truth. ---
    let rows_after = scan_rows(&dir);
    assert_eq!(rows_after.len(), 4, "reproduce row appended at seq 3");
    let reproduce_entry = decode(&rows_after[3].bytes).unwrap();
    assert_eq!(reproduce_entry.seq, 3);
    assert_eq!(reproduce_entry.kind, EntryKind::Admin);

    // 1. The on-disk head witness advanced to height 4 with the reproduce tip hash.
    let anchor_path = dir.join("_head_anchor.json");
    assert!(anchor_path.exists(), "head anchor file physically present");
    let anchor_after = read_anchor(&dir);
    assert_eq!(
        anchor_after.height, 4,
        "anchor advanced past the reproduce row"
    );
    assert_eq!(
        anchor_after.tip_hash, reproduce_entry.entry_hash,
        "anchor tip hash matches the reproduce row"
    );

    // 2. A reopened appender recovers next_seq=4 from the anchor (the pre-fix bug
    //    recovered 3 and collided). 3. A fresh append lands at seq 4 — no wedge.
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&dir).unwrap(),
        FixedClock::new(9_000),
    )
    .expect("reopen appender after reproduce");
    assert_eq!(
        appender.next_seq(),
        4,
        "appender recovers tip past reproduce row"
    );
    let next_ref = appender
        .append(
            EntryKind::Admin,
            SubjectId::Query(answer_id.clone()),
            serde_json::to_vec(&json!({"note": "post-reproduce commit"})).unwrap(),
            ActorId::Service(ACTOR.to_string()),
        )
        .expect("post-reproduce append must not wedge");
    assert_eq!(next_ref.seq, 4, "post-reproduce commit lands at seq 4");

    // 4. The whole chain verifies Intact against the (now-consistent) anchor.
    let store = DirectoryLedgerStore::open(&dir).unwrap();
    match verify_chain(&store, 0..5).unwrap() {
        VerifyResult::Intact { count } => assert_eq!(count, 5),
        other => panic!("expected Intact chain after reproduce, got {other:?}"),
    }

    println!(
        "HAPPY anchor {}->{} rows {}->{} tip={}",
        anchor_before.height,
        anchor_after.height,
        rows_before.len(),
        rows_after.len(),
        hex(&anchor_after.tip_hash),
    );
}

/// Boundary: a reproduce append on a fresh empty ledger creates the genesis row
/// and a height-1 anchor witness.
#[test]
fn append_reproduce_entry_on_empty_ledger_creates_genesis_anchor() {
    let dir = scratch_dir("genesis");
    let mut store = DirectoryLedgerStore::open(&dir).unwrap();
    assert!(
        store.head_anchor().unwrap().is_none(),
        "no anchor before first write"
    );
    assert!(scan_rows(&dir).is_empty(), "no rows before first write");

    let answer_id: QueryId = b"anchor-genesis".to_vec();
    let entry = append_reproduce_entry(&mut store, &answer_id, &trivial_result())
        .expect("genesis reproduce append");
    assert_eq!(entry.seq, 0, "genesis row at seq 0");
    assert_eq!(entry.prev_hash, [0_u8; 32], "genesis uses zero prev hash");

    let rows = scan_rows(&dir);
    assert_eq!(rows.len(), 1);
    let anchor = read_anchor(&dir);
    assert_eq!(anchor.height, 1, "genesis anchor at height 1");
    assert_eq!(anchor.tip_hash, entry.entry_hash);
    match verify_chain(&store, 0..1).unwrap() {
        VerifyResult::Intact { count } => assert_eq!(count, 1),
        other => panic!("expected Intact genesis chain, got {other:?}"),
    }
    println!(
        "GENESIS anchor height {} tip {}",
        anchor.height,
        hex(&anchor.tip_hash)
    );
}

/// Edge: an end-truncated ledger (rows removed below the anchor) must fail
/// closed on a reproduce append instead of silently overwriting the truncation
/// point. Proves the anchor is honored, not ignored.
#[test]
fn append_reproduce_entry_fails_closed_on_end_truncated_ledger() {
    let dir = scratch_dir("truncated");
    // Seed three admin rows so the anchor witness reaches height 3.
    seed_admin_rows(&dir, 3);
    let anchor_before = read_anchor(&dir);
    assert_eq!(anchor_before.height, 3);

    // Simulate tail truncation: physically delete the newest row (seq 2) but
    // leave the anchor witness claiming height 3.
    let victim = dir.join(format!("{:016x}.ledger", 2));
    assert!(victim.exists(), "row seq 2 exists before truncation");
    fs::remove_file(&victim).expect("remove tail row");
    assert_eq!(scan_rows(&dir).len(), 2, "ledger now end-truncated");

    // The reproduce append must refuse to write, surfacing a typed error.
    let mut store = DirectoryLedgerStore::open(&dir).unwrap();
    let answer_id: QueryId = b"anchor-truncated".to_vec();
    let error = append_reproduce_entry(&mut store, &answer_id, &trivial_result())
        .expect_err("reproduce append must fail closed on end-truncation");
    assert_eq!(
        error.code, "CALYX_LEDGER_CHAIN_BROKEN",
        "end-truncation surfaces a chain-broken error, got {error:?}"
    );

    // Full state verification: nothing was written, the anchor did not move.
    assert_eq!(
        scan_rows(&dir).len(),
        2,
        "no row written on failed reproduce append"
    );
    let anchor_after = read_anchor(&dir);
    assert_eq!(
        anchor_after.height, 3,
        "anchor unchanged after fail-closed append"
    );
    println!(
        "TRUNCATED failed closed: {} (anchor still {})",
        error.code, anchor_after.height
    );
}

// --- helpers ---

fn trivial_result() -> ReproduceResult {
    ReproduceResult {
        reproduced: true,
        max_drift: 0.0,
        original_hits: Vec::new(),
        reproduced_hits: Vec::new(),
    }
}

fn seed_admin_rows(dir: &Path, count: u64) {
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(dir).unwrap(),
        FixedClock::new(1_000),
    )
    .unwrap();
    for idx in 0..count {
        appender
            .append(
                EntryKind::Admin,
                SubjectId::Query(format!("seed-{idx}").into_bytes()),
                serde_json::to_vec(&json!({"seed": idx})).unwrap(),
                ActorId::Service(ACTOR.to_string()),
            )
            .unwrap();
    }
}

fn read_anchor(dir: &Path) -> LedgerHeadAnchor {
    DirectoryLedgerStore::open(dir)
        .unwrap()
        .head_anchor()
        .unwrap()
        .expect("head anchor present on disk")
}

fn scan_rows(dir: &Path) -> Vec<calyx_ledger::LedgerRow> {
    DirectoryLedgerStore::open(dir).unwrap().scan().unwrap()
}

fn write_answer_ledger(
    dir: &Path,
    slots: &[RecordedSlot],
    answer_id: &QueryId,
    fusion: &FusionWeights,
    original_hits: &[HitRef],
) {
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(dir).unwrap(),
        FixedClock::new(1_000),
    )
    .unwrap();
    for slot in slots {
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
                ActorId::Service(ACTOR.to_string()),
            )
            .unwrap();
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
            ActorId::Service(ACTOR.to_string()),
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
    (slots, b"answer-anchor-fsv".to_vec(), fusion, original_hits)
}

fn recorded_slot(slot: u16, seed: u64) -> RecordedSlot {
    let input = Input::new(
        Modality::Text,
        format!("anchor-fsv-slot-{slot}").into_bytes(),
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

fn vector_for_recorded_slot(slot: &RecordedSlot) -> SlotVector {
    match slot.slot_id.get() {
        0 => dense(&[0.9, 0.7]),
        1 => dense(&[0.8, 0.6]),
        _ => dense(&[]),
    }
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("calyx-reproduce-anchor-fsv")
        .join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("reset scratch dir");
    }
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}
