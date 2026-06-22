use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, FixedClock, LedgerRef, Result, SlotId, SystemClock, VaultId, VaultStore};
use calyx_ledger::decode as decode_ledger;
use calyx_loom::{
    CALYX_REACTIVE_DRAIN_OVERFLOW, CALYX_REACTIVE_REGISTRY_FULL,
    CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND, NoveltyVerdict, ReactiveEngine, ReactiveRowKind,
    ReactiveSignals, TriggerCondition, decode_trigger_fired, reactive_row_key,
};

const SALT: &[u8] = b"issue573-reactive-subscription";
static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct ScriptedSignals {
    occ: Cell<u64>,
    novelty: NoveltyVerdict,
    drift: f32,
}

impl ReactiveSignals for ScriptedSignals {
    fn novelty(&self, _cx_id: CxId, _tau: Option<f32>) -> Result<NoveltyVerdict> {
        Ok(self.novelty)
    }

    fn occurrence_count(&self, _series: CxId) -> Result<u64> {
        let next = self.occ.get() + 1;
        self.occ.set(next);
        Ok(next)
    }

    fn slot_drift(&self, _slot: SlotId) -> Result<f32> {
        Ok(self.drift)
    }
}

#[test]
fn durable_subscription_lifecycle_writes_ledger_and_observes_delta() {
    let dir = test_dir("lifecycle");
    clean(&dir);
    let vault = open_vault(&dir);
    let mut engine = ReactiveEngine::new(Arc::new(FixedClock::new(1_786_400_000)));
    let cx = CxId::from_bytes([0x57; 16]);

    let sub = engine
        .subscribe_durable(
            &vault,
            TriggerCondition::NewRegion { tau_override: None },
            Some("issue573".to_string()),
        )
        .unwrap();
    engine
        .evaluate_post_ingest_durable(&vault, cx, lref(7), &novel_signals())
        .unwrap();
    let observed = engine.observe_delta(sub).unwrap();
    engine.unsubscribe_durable(&vault, sub).unwrap();
    let err = engine.observe_delta(sub).unwrap_err();

    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].ledger_ref.seq, 7);
    assert_eq!(err.code, CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND);

    let subscription_actions = ledger_payloads(&vault)
        .into_iter()
        .filter(|payload| payload["tag"] == "reactive_subscription_v1")
        .map(|payload| payload["action"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        subscription_actions,
        vec!["SUBSCRIPTION_CREATED", "SUBSCRIPTION_REMOVED"]
    );

    let fired = reactive_fired_count(&vault);
    assert_eq!(fired, 1);
}

#[test]
fn subscribe_observe_delta_drains_once() {
    let mut engine = ReactiveEngine::new(Arc::new(FixedClock::new(1_000)));
    let sub = engine
        .subscribe(
            TriggerCondition::EventRecurs {
                series: cx(9),
                min_occurrences: 1,
            },
            None,
        )
        .unwrap();

    engine
        .evaluate_post_ingest(cx(9), lref(1), &novel_signals())
        .unwrap();

    let first = engine.observe_delta(sub).unwrap();
    let second = engine.observe_delta(sub).unwrap();

    assert_eq!(first.len(), 1);
    assert_eq!(first[0].ledger_ref.seq, 1);
    assert!(second.is_empty());
}

#[test]
fn independent_subscriptions_do_not_cross_drain() {
    let mut engine = ReactiveEngine::new(Arc::new(FixedClock::new(1_000)));
    let recurrence = engine
        .subscribe(
            TriggerCondition::EventRecurs {
                series: cx(9),
                min_occurrences: 1,
            },
            None,
        )
        .unwrap();
    let drift = engine
        .subscribe(
            TriggerCondition::DriftDetected {
                slot: SlotId::new(3),
                drift_threshold: 2.0,
            },
            None,
        )
        .unwrap();

    engine
        .evaluate_post_ingest(cx(9), lref(1), &novel_signals())
        .unwrap();

    assert_eq!(engine.observe_delta(recurrence).unwrap().len(), 1);
    assert!(engine.observe_delta(drift).unwrap().is_empty());
}

#[test]
fn subscription_drain_overflow_is_reported_on_observe() {
    let mut engine =
        ReactiveEngine::with_subscription_caps(Arc::new(FixedClock::new(1)), 8, 16, 16, 8, 2);
    let sub = engine
        .subscribe(TriggerCondition::NewRegion { tau_override: None }, None)
        .unwrap();
    let signals = novel_signals();
    for seq in 1..=3 {
        engine
            .evaluate_post_ingest(cx(1), lref(seq), &signals)
            .unwrap();
    }

    let err = engine.observe_delta(sub).unwrap_err();
    assert_eq!(err.code, CALYX_REACTIVE_DRAIN_OVERFLOW);
    let report = engine.observe_delta_report(sub).unwrap();
    let seqs = report
        .events
        .iter()
        .map(|event| event.ledger_ref.seq)
        .collect::<Vec<_>>();
    assert!(report.overflowed);
    assert_eq!(seqs, vec![2, 3]);
}

#[test]
fn subscription_capacity_rejects_without_leaking_trigger() {
    let mut engine =
        ReactiveEngine::with_subscription_caps(Arc::new(FixedClock::new(1)), 8, 16, 16, 1, 16);
    engine
        .subscribe(TriggerCondition::NewRegion { tau_override: None }, None)
        .unwrap();
    let err = engine
        .subscribe(TriggerCondition::NewRegion { tau_override: None }, None)
        .unwrap_err();

    assert_eq!(err.code, CALYX_REACTIVE_REGISTRY_FULL);
    assert_eq!(engine.subscriptions().len(), 1);
    assert_eq!(engine.registry().len(), 1);
}

fn novel_signals() -> ScriptedSignals {
    ScriptedSignals {
        occ: Cell::new(0),
        novelty: NoveltyVerdict::NewRegion,
        drift: 0.0,
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn reactive_fired_count(vault: &AsterVault<SystemClock>) -> usize {
    vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Reactive)
        .unwrap()
        .into_iter()
        .filter(|(key, value)| {
            let parts = reactive_row_key(key).unwrap();
            parts.kind == ReactiveRowKind::Fired && decode_trigger_fired(value).is_ok()
        })
        .count()
}

fn ledger_payloads(vault: &AsterVault<SystemClock>) -> Vec<serde_json::Value> {
    vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .filter_map(|(_, bytes)| {
            serde_json::from_slice(&decode_ledger(&bytes).unwrap().payload).ok()
        })
        .collect()
}

fn open_vault(dir: &Path) -> AsterVault<SystemClock> {
    AsterVault::new_durable(dir, vault_id(), SALT.to_vec(), VaultOptions::default()).unwrap()
}

fn lref(seq: u64) -> LedgerRef {
    LedgerRef {
        seq,
        hash: [seq as u8; 32],
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("calyx-issue573-{name}-{}-{id}", std::process::id()))
}

fn clean(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
}
