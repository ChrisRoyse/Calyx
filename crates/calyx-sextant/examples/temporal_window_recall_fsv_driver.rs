//! FSV driver for issue #633 — windowed multi-primary temporal recall.
//!
//! Builds the hand-known two-slot fixture (slot 8 = seeds 1..=5, slot 9 =
//! seeds 11..=15; only seed 15 in-window at fused position 10) and writes one
//! readback JSON per scenario into `<out-dir>` so the persisted artifacts can
//! be judged against hand-computed expectations:
//!
//! ```text
//! cargo run -p calyx-sextant --example temporal_window_recall_fsv_driver -- <out-dir>
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, DecayFunction, InputRef, LedgerRef, Modality,
    PeriodicOptions, SlotId, SlotVector, VaultId,
};
use calyx_sextant::{
    HnswIndex, Query, SearchEngine, SlotIndexMap, TemporalFixedClock, TemporalPolicy, TimeWindow,
    WindowRecallPolicy, temporal_search, temporal_search_with_recall,
};
use serde_json::json;

const SLOT_A: SlotId = SlotId::new(8);
const SLOT_B: SlotId = SlotId::new(9);
const QUERY_TIME: i64 = 1_000_000;
const IN_WINDOW_SEED: u8 = 15;

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .expect("usage: temporal_window_recall_fsv_driver <out-dir>");
    let out_dir = Path::new(&out_dir);
    fs::create_dir_all(out_dir).expect("create out dir");

    let engine = two_slot_engine();
    let clock = TemporalFixedClock::new(QUERY_TIME);
    let window = TimeWindow::last_hours(1, &clock).expect("window");
    let policy = policy();

    let exhaustive = temporal_search(
        &engine,
        &query(2, None, Some(64)),
        Some(window),
        &policy,
        &clock,
        0,
    )
    .expect("exhaustive windowed search");
    write_json(out_dir, "exhaustive.json", &json!(exhaustive));

    let bounded = temporal_search_with_recall(
        &engine,
        &query(1, Some(2), Some(64)),
        Some(window),
        &policy,
        &clock,
        0,
        WindowRecallPolicy::Bounded { max_candidates: 10 },
    )
    .expect("bounded windowed search");
    write_json(out_dir, "bounded-deepen.json", &json!(bounded));

    let exhausted = temporal_search_with_recall(
        &engine,
        &query(1, Some(2), Some(64)),
        Some(window),
        &policy,
        &clock,
        0,
        WindowRecallPolicy::Bounded { max_candidates: 4 },
    )
    .expect_err("budget 4 cannot prove completeness");
    write_json(
        out_dir,
        "bounded-exhausted-error.json",
        &json!({
            "code": exhausted.code,
            "message": exhausted.message,
            "remediation": exhausted.remediation,
        }),
    );

    let ef_raised = temporal_search(
        &engine,
        &query(2, None, Some(2)),
        Some(window),
        &policy,
        &clock,
        0,
    )
    .expect("windowed search with small caller ef");
    write_json(out_dir, "ef-raised.json", &json!(ef_raised));

    let windowless = temporal_search(
        &engine,
        &query(2, Some(3), Some(64)),
        None,
        &policy,
        &clock,
        0,
    )
    .expect("windowless search");
    write_json(out_dir, "windowless-bounded.json", &json!(windowless));

    println!(
        "FSV_DRIVER_DONE out_dir={} in_window_seed={IN_WINDOW_SEED}",
        out_dir.display()
    );
}

fn two_slot_engine() -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(SLOT_A, 2, 42)).unwrap();
    map.register(HnswIndex::new(SLOT_B, 2, 43)).unwrap();
    let mut engine = SearchEngine::new(map);
    for rank in 1..=5_u8 {
        insert_doc(&mut engine, SLOT_A, rank, rank, out_of_window_created_at());
        let b_seed = rank + 10;
        let created_at = if b_seed == IN_WINDOW_SEED {
            in_window_created_at()
        } else {
            out_of_window_created_at()
        };
        insert_doc(&mut engine, SLOT_B, b_seed, rank, created_at);
    }
    engine
}

fn insert_doc(engine: &mut SearchEngine, slot: SlotId, seed: u8, slot_rank: u8, created_at: u64) {
    let vector = dense(vec![1.0, 0.2 * f32::from(slot_rank)]);
    engine
        .indexes
        .insert(slot, cx(seed), vector, u64::from(seed))
        .unwrap();
    engine.put_constellation(row(seed, created_at));
}

fn in_window_created_at() -> u64 {
    (QUERY_TIME - 600) as u64
}

fn out_of_window_created_at() -> u64 {
    (QUERY_TIME - 100_000) as u64
}

fn query(k: usize, recall_k: Option<usize>, ef: Option<usize>) -> Query {
    Query {
        k,
        recall_k,
        ef,
        ..Query::new("window recall fsv")
            .with_vector(dense(vec![1.0, 0.0]))
            .with_slots(vec![SLOT_A, SLOT_B])
    }
}

fn policy() -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        DecayFunction::Step,
        PeriodicOptions::new(None, None).expect("periodic"),
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
            pointer: Some(format!("zfs://calyx/window-recall-fsv/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("window-recall-fsv".to_string()),
            value: AnchorValue::Text("synthetic".to_string()),
            source: "issue-633".to_string(),
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

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn write_json(out_dir: &Path, name: &str, value: &serde_json::Value) {
    let path = out_dir.join(name);
    fs::write(&path, serde_json::to_vec_pretty(value).expect("json")).expect("write readback");
    println!("FSV_READBACK={}", path.display());
}
