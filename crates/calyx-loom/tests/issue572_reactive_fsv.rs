//! Full-State Verification for PH72 · T02 (#572) — reactive trigger engine.
//!
//! Source of truth: the durable recurrence occurrence count on disk (read back
//! independently via `SeriesStore::occurrence_count` after flush + reopen) and
//! the engine's own audit log + fired-event queue. The `EventRecurs` condition
//! is driven by REAL appended occurrences — no mocked counts.
//!
//! Run: `cargo test -p calyx-loom --test issue572_reactive_fsv -- --nocapture`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use calyx_aster::dedup::EpochSecs;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Clock, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, SystemClock,
    VaultId, VaultStore,
};
use calyx_loom::recurrence::{OccurrenceContext, SeriesStore};
use calyx_loom::{ReactiveEngine, RecurrenceSignals, TriggerCondition};

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn salt() -> Vec<u8> {
    b"issue572-reactive-fsv-salt".to_vec()
}

fn root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE572_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-issue572-reactive-fsv"))
}

fn open_durable(dir: &std::path::Path) -> AsterVault<SystemClock> {
    AsterVault::new_durable(dir, vault_id(), salt(), VaultOptions::default()).unwrap()
}

/// Writes the base constellation a recurrence series hangs off, returning its id.
fn put_base<C: Clock>(vault: &AsterVault<C>, input: &[u8]) -> CxId {
    let cx_id = vault.cx_id_for_input(input, 41);
    let cx = Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 41,
        created_at: 100,
        input_ref: InputRef {
            hash: *blake3::hash(input).as_bytes(),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            redacted_input: true,
            ..CxFlags::default()
        },
    };
    vault.put(cx).expect("put base constellation");
    cx_id
}

fn lref(seq: u64) -> LedgerRef {
    LedgerRef {
        seq,
        hash: [seq as u8; 32],
    }
}

/// Happy path: a real recurring event crosses `min_occurrences = 3` and fires
/// exactly once, on the third ingest — proven against the on-disk count.
#[test]
fn event_recurs_fires_on_real_third_occurrence() {
    let dir = root().join("vault");
    fs::remove_dir_all(&dir).ok();
    let vault = open_durable(&dir);
    let series = put_base(&vault, b"reactive-series-alpha");
    let store = SeriesStore::new(&vault);

    let mut engine = ReactiveEngine::new(Arc::new(FixedClock::new(1_000)));
    let trigger = engine
        .register(
            TriggerCondition::EventRecurs {
                series,
                min_occurrences: 3,
            },
            Some("fsv".to_string()),
        )
        .unwrap();

    // SoT BEFORE: zero occurrences on disk.
    assert_eq!(store.occurrence_count(series).unwrap(), 0);

    // Ingest the recurring event three times; evaluate the engine after each.
    let mut fires = Vec::new();
    let mut counts_after = Vec::new();
    for (i, t_k) in [100i64, 200, 300].into_iter().enumerate() {
        store
            .append_occurrence(
                series,
                EpochSecs(t_k),
                OccurrenceContext::new(b"ctx".to_vec()).unwrap(),
            )
            .unwrap();
        let signals = RecurrenceSignals::new(&vault);
        let fired = engine
            .evaluate_post_ingest(series, lref(i as u64 + 1), &signals)
            .unwrap();
        fires.push(fired);
        counts_after.push(store.occurrence_count(series).unwrap());
    }

    // Hand-computed expectation (2+2=4 discipline):
    //   counts 1,2,3 → fire only when count incremented AND >= 3 → [0,0,1].
    assert_eq!(
        fires,
        vec![0, 0, 1],
        "fires only on the threshold-crossing ingest"
    );
    assert_eq!(counts_after, vec![1, 2, 3], "on-disk count grows 1→2→3");

    // SoT cross-check: independent read of the occurrence count.
    assert_eq!(store.occurrence_count(series).unwrap(), 3);

    // The single fired event references the third ingest.
    let fired_events = engine.drain_fired();
    assert_eq!(fired_events.len(), 1);
    assert_eq!(fired_events[0].trigger_id, trigger);
    assert_eq!(fired_events[0].ledger_ref.seq, 3);

    // Audit log: exactly three evaluations, two no-match then one match.
    let matched: Vec<bool> = engine.audit_log().entries().map(|e| e.matched).collect();
    assert_eq!(matched, vec![false, false, true]);

    // Durability: flush, reopen, and re-read the count from disk.
    vault.flush().unwrap();
    drop(vault);
    let reopened = open_durable(&dir);
    let reopened_count = SeriesStore::new(&reopened)
        .occurrence_count(series)
        .unwrap();
    assert_eq!(reopened_count, 3, "occurrence count survives flush+reopen");

    let artifact = serde_json::json!({
        "issue": 572,
        "property": "EventRecurs fires once when a real recurrence crosses min_occurrences",
        "source_of_truth": {
            "occurrence_count": dir.join("cf").join("recurrence").display().to_string(),
            "engine_audit_log": "in-memory ring (asserted contents below)",
        },
        "min_occurrences": 3,
        "fires_per_ingest": fires,
        "on_disk_count_after_each_ingest": counts_after,
        "reopened_on_disk_count": reopened_count,
        "fired_event_ledger_seq": fired_events[0].ledger_ref.seq,
        "audit_matched_sequence": matched,
    });
    let out = root().join("issue572-reactive-fsv-artifact.json");
    fs::write(&out, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
}

/// Edge 1 (empty): a series with zero appended occurrences never fires.
#[test]
fn edge_empty_series_never_fires() {
    let dir = root().join("vault-empty");
    fs::remove_dir_all(&dir).ok();
    let vault = open_durable(&dir);
    let series = put_base(&vault, b"reactive-series-empty");

    let mut engine = ReactiveEngine::new(Arc::new(FixedClock::new(1)));
    engine
        .register(
            TriggerCondition::EventRecurs {
                series,
                min_occurrences: 1,
            },
            None,
        )
        .unwrap();

    let before = SeriesStore::new(&vault).occurrence_count(series).unwrap();
    let fired = engine
        .evaluate_post_ingest(series, lref(1), &RecurrenceSignals::new(&vault))
        .unwrap();
    let after = SeriesStore::new(&vault).occurrence_count(series).unwrap();

    assert_eq!(before, 0, "BEFORE: empty series");
    assert_eq!(after, 0, "AFTER: still empty");
    assert_eq!(fired, 0, "no occurrences → no fire");
    assert_eq!(engine.queue().len(), 0);
    println!("edge_empty: before={before} after={after} fired={fired}");
}

/// Edge 2 (registry max): registering past `max_triggers` fails closed.
#[test]
fn edge_registry_full_fails_closed() {
    let mut engine = ReactiveEngine::with_caps(Arc::new(FixedClock::new(1)), 1, 16, 16);
    let before = engine.registry().len();
    engine
        .register(TriggerCondition::NewRegion { tau_override: None }, None)
        .unwrap();
    let err = engine
        .register(TriggerCondition::NewRegion { tau_override: None }, None)
        .unwrap_err();
    let after = engine.registry().len();

    assert_eq!(before, 0, "BEFORE: empty registry");
    assert_eq!(after, 1, "AFTER: rejected registration left exactly one");
    assert_eq!(err.code, "CALYX_REACTIVE_REGISTRY_FULL");
    println!(
        "edge_registry_full: before={before} after={after} code={}",
        err.code
    );
}

/// Edge 3 (queue overflow): more fires than `max_queue_depth` discards the
/// oldest, returns `CALYX_REACTIVE_QUEUE_FULL`, and records the warning.
#[test]
fn edge_queue_overflow_warns() {
    let dir = root().join("vault-overflow");
    fs::remove_dir_all(&dir).ok();
    let vault = open_durable(&dir);
    let series = put_base(&vault, b"reactive-series-overflow");
    let store = SeriesStore::new(&vault);
    store
        .append_occurrence(
            series,
            EpochSecs(1),
            OccurrenceContext::new(b"c".to_vec()).unwrap(),
        )
        .unwrap();

    // Three EventRecurs(min=1) triggers but a queue depth of 2.
    let mut engine = ReactiveEngine::with_caps(Arc::new(FixedClock::new(1)), 8, 2, 64);
    for _ in 0..3 {
        engine
            .register(
                TriggerCondition::EventRecurs {
                    series,
                    min_occurrences: 1,
                },
                None,
            )
            .unwrap();
    }

    let before_q = engine.queue().len();
    let err = engine
        .evaluate_post_ingest(series, lref(7), &RecurrenceSignals::new(&vault))
        .unwrap_err();
    let after_q = engine.queue().len();

    assert_eq!(before_q, 0, "BEFORE: empty queue");
    assert_eq!(after_q, 2, "AFTER: bounded at max_queue_depth=2");
    assert_eq!(err.code, "CALYX_REACTIVE_QUEUE_FULL");
    assert_eq!(
        engine.audit_log().last().unwrap().code.as_deref(),
        Some("CALYX_REACTIVE_QUEUE_FULL"),
        "last audit row is the overflow warning"
    );
    println!(
        "edge_overflow: before_q={before_q} after_q={after_q} code={}",
        err.code
    );
}

/// Edge 4 (fail-closed): a recurrence-only source cannot evaluate `NewRegion`;
/// the engine propagates `CALYX_REACTIVE_SIGNAL_UNAVAILABLE` and fires nothing.
#[test]
fn edge_novelty_on_recurrence_source_fails_closed() {
    let dir = root().join("vault-novelty");
    fs::remove_dir_all(&dir).ok();
    let vault = open_durable(&dir);

    let mut engine = ReactiveEngine::new(Arc::new(FixedClock::new(1)));
    engine
        .register(TriggerCondition::NewRegion { tau_override: None }, None)
        .unwrap();
    let err = engine
        .evaluate_post_ingest(
            CxId::from_bytes([3u8; 16]),
            lref(1),
            &RecurrenceSignals::new(&vault),
        )
        .unwrap_err();
    assert_eq!(err.code, "CALYX_REACTIVE_SIGNAL_UNAVAILABLE");
    assert!(
        engine.queue().is_empty(),
        "no fire on a fail-closed evaluation"
    );
    println!("edge_failclosed: code={}", err.code);
}
