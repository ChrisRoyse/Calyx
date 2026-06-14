//! Full-State Verification for #755 durable reactive state/readbacks/adapters.
//!
//! Source of truth: Aster `reactive` CF rows plus Ledger CF bytes in a durable
//! vault. The happy path drives the real PH72 stream ingester, which calls the
//! reactive hook after each `ingest_at`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::dedup::{
    DedupAction, DedupPolicy, EpochSecs, IngestInput, TauStrategy, TctCosineConfig,
};
use calyx_aster::recurrence::OccurrenceContext;
use calyx_aster::stream::{
    BackpressureGuard, PostIngestHook, QuantizeOnlineConfig, StreamIngester,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    CalyxError, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, LensId, Modality,
    SlotId, SlotVector, SystemClock, VaultId, VaultStore,
};
use calyx_forge::quant::QuantLevel;
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_loom::{
    AgreementDriftTracker, ReactiveEngine, ReactiveRowKind, ReactiveSignalSet, RecurrenceSignals,
    TriggerCondition, decode_audit_entry, decode_trigger_fired, reactive_row_key,
};
use calyx_ward::{GuardId, GuardPolicy, GuardProfile, NoveltyAction};

const PANEL_VERSION: u32 = 41;
const SERIES_RAW: &[u8] = b"issue755-recurring-stream";
const SLOT_CONTENT: SlotId = SlotId::new(0);
const SLOT_TIME: SlotId = SlotId::new(20);
const SLOT_DRIFT: SlotId = SlotId::new(7);
const SALT: &[u8] = b"issue755-reactive-durable-salt";

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn stream_event_recurs_persists_trigger_rows_and_ledger_ref() {
    let dir = root().join("stream-event-recurs");
    clean(&dir);
    let vault = Arc::new(open_stream_vault(&dir));
    let series = vault.cx_id_for_input(SERIES_RAW, PANEL_VERSION);
    seed_setup_ledger(vault.as_ref());

    let engine = Arc::new(Mutex::new(ReactiveEngine::new(Arc::new(FixedClock::new(
        1_786_320_000,
    )))));
    let trigger = engine
        .lock()
        .unwrap()
        .register(
            TriggerCondition::EventRecurs {
                series,
                min_occurrences: 3,
            },
            Some("issue755".to_string()),
        )
        .unwrap();

    let hook: PostIngestHook<SystemClock> = {
        let engine = Arc::clone(&engine);
        Arc::new(move |vault, cx_id, ledger_ref| {
            let signals = ReactiveSignalSet::new(vault);
            engine
                .lock()
                .map_err(|_| CalyxError::backpressure("reactive engine lock poisoned"))?
                .evaluate_post_ingest_durable(vault, cx_id, ledger_ref, &signals)
                .map(|_| ())
        })
    };
    let ingester = StreamIngester::new_with_post_ingest_hook(
        Arc::clone(&vault),
        stream_config(),
        BackpressureGuard::new(8, 0),
        Some(hook),
    );
    for (index, t) in [100, 200, 300].into_iter().enumerate() {
        ingester
            .send(stream_input(index), EpochSecs(t))
            .expect("stream send");
    }
    let stats = ingester.drain_and_close().expect("stream drain");
    assert_eq!(stats.ingested, 3);

    let audit = audit_entries(vault.as_ref(), trigger);
    let fired = fired_events(vault.as_ref());
    assert_eq!(
        audit.iter().map(|entry| entry.matched).collect::<Vec<_>>(),
        vec![false, false, true]
    );
    assert_eq!(fired.len(), 1);
    let matched_ref = audit
        .iter()
        .find(|entry| entry.matched)
        .expect("matched audit row")
        .ledger_ref
        .clone();
    assert_eq!(fired[0].ledger_ref, matched_ref);
    assert_eq!(
        fired[0].ledger_ref.hash,
        ledger_hash(vault.as_ref(), fired[0].ledger_ref.seq)
    );

    let before_reopen_rows = reactive_rows(vault.as_ref()).len();
    vault.flush().unwrap();
    drop(vault);
    let reopened = open_stream_vault(&dir);
    let reopened_audit = audit_entries(&reopened, trigger);
    let reopened_fired = fired_events(&reopened);
    assert_eq!(reopened_audit.len(), 3);
    assert_eq!(reopened_fired.len(), 1);
    let reopened_ref = reopened_fired[0].ledger_ref.clone();

    write_artifact(
        "stream-event-recurs.json",
        serde_json::json!({
            "issue": 755,
            "vault": dir.display().to_string(),
            "series": series,
            "trigger": trigger,
            "stats": {
                "ingested": stats.ingested,
                "batches": stats.batches,
                "quantized": stats.quantized,
            },
            "reactive_rows_before_reopen": before_reopen_rows,
            "audit_matched": reopened_audit.iter().map(|entry| entry.matched).collect::<Vec<_>>(),
            "fired_ledger_seq": reopened_ref.seq,
            "fired_ledger_hash": hex(&reopened_ref.hash),
            "ledger_row_hash": hex(&ledger_hash(&reopened, reopened_ref.seq)),
        }),
    );
}

#[test]
fn queue_overflow_persists_warning_audit_row() {
    let dir = test_dir("queue-overflow");
    clean(&dir);
    let vault = open_stream_vault(&dir);
    let series = put_base(&vault, b"issue755-overflow-series");
    calyx_loom::recurrence::SeriesStore::new(&vault)
        .append_occurrence(
            series,
            EpochSecs(1),
            OccurrenceContext::new(b"overflow".to_vec()).unwrap(),
        )
        .unwrap();

    let mut engine = ReactiveEngine::with_caps(Arc::new(FixedClock::new(55)), 8, 2, 64);
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
    let err = engine
        .evaluate_post_ingest_durable(&vault, series, lref(9), &RecurrenceSignals::new(&vault))
        .expect_err("queue overflow");
    assert_eq!(err.code, "CALYX_REACTIVE_QUEUE_FULL");
    let warnings = all_audit_entries(&vault)
        .into_iter()
        .filter(|entry| entry.code.as_deref() == Some("CALYX_REACTIVE_QUEUE_FULL"))
        .collect::<Vec<_>>();
    assert_eq!(warnings.len(), 1);
    let warning_ledger_payloads = ledger_payloads(&vault)
        .into_iter()
        .filter(|payload| payload["tag"] == "reactive_state_v1" && payload["warning_count"] == 1)
        .collect::<Vec<_>>();
    assert_eq!(warning_ledger_payloads.len(), 1);
    write_artifact(
        "queue-overflow.json",
        serde_json::json!({
            "issue": 755,
            "vault": dir.display().to_string(),
            "audit_warning_count": warnings.len(),
            "ledger_warning_payloads": warning_ledger_payloads,
        }),
    );
}

#[test]
fn ward_adapter_fires_new_region_from_real_slot_rows() {
    let dir = test_dir("ward-new-region");
    clean(&dir);
    let vault = open_stream_vault(&dir);
    let matched = put_slot_cx(&vault, b"matched", SLOT_CONTENT, [1.0, 0.0]);
    let produced = put_slot_cx(&vault, b"produced", SLOT_CONTENT, [0.0, 1.0]);
    let profile = guard_profile();

    let mut engine = ReactiveEngine::new(Arc::new(FixedClock::new(77)));
    engine
        .register(TriggerCondition::NewRegion { tau_override: None }, None)
        .unwrap();
    let signals = ReactiveSignalSet::new(&vault).with_ward_novelty(profile, matched, false);
    let fired = engine
        .evaluate_post_ingest_durable(&vault, produced, lref(11), &signals)
        .unwrap();

    assert_eq!(fired, 1);
    assert_eq!(fired_events(&vault).len(), 1);
}

#[test]
fn drift_adapter_tracks_previous_slot_snapshot() {
    let dir = test_dir("drift");
    clean(&dir);
    let vault = open_stream_vault(&dir);
    let first = put_slot_cx(&vault, b"drift-a", SLOT_DRIFT, [1.0, 0.0]);
    let second = put_slot_cx(&vault, b"drift-b", SLOT_DRIFT, [0.0, 1.0]);
    let tracker = AgreementDriftTracker::new();
    let mut engine = ReactiveEngine::new(Arc::new(FixedClock::new(88)));
    engine
        .register(
            TriggerCondition::DriftDetected {
                slot: SLOT_DRIFT,
                drift_threshold: 0.5,
            },
            None,
        )
        .unwrap();

    let first_signals = ReactiveSignalSet::new(&vault).with_agreement_drift(first, &tracker);
    assert_eq!(
        engine
            .evaluate_post_ingest_durable(&vault, first, lref(21), &first_signals)
            .unwrap(),
        0
    );
    let second_signals = ReactiveSignalSet::new(&vault).with_agreement_drift(second, &tracker);
    assert_eq!(
        engine
            .evaluate_post_ingest_durable(&vault, second, lref(22), &second_signals)
            .unwrap(),
        1
    );
    assert_eq!(fired_events(&vault).len(), 1);
}

fn open_stream_vault(dir: &Path) -> AsterVault<SystemClock> {
    let options = VaultOptions {
        dedup_policy: Some(recurrence_policy()),
        ..VaultOptions::default()
    };
    AsterVault::new_durable(dir, vault_id(), SALT.to_vec(), options).unwrap()
}

fn recurrence_policy() -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![SLOT_CONTENT],
            TauStrategy::PerSlot(vec![(SLOT_CONTENT, 0.90)]),
            DedupAction::RecurrenceSeries,
        )
        .unwrap(),
    )
}

fn stream_config() -> QuantizeOnlineConfig {
    QuantizeOnlineConfig::new(LensId::from_bytes([0x75; 16]), QuantLevel::Bits3p5)
}

fn stream_input(index: usize) -> IngestInput {
    IngestInput::new(SERIES_RAW.to_vec(), PANEL_VERSION, Modality::Text)
        .with_slot(
            SLOT_CONTENT,
            SlotVector::Dense {
                dim: 2,
                data: vec![1.0, 0.0],
            },
        )
        .with_slot(
            SLOT_TIME,
            SlotVector::Dense {
                dim: 2,
                data: temporal_vec(index).to_vec(),
            },
        )
        .with_temporal_slot(SLOT_TIME)
}

fn put_slot_cx(
    vault: &AsterVault<SystemClock>,
    raw: &[u8],
    slot: SlotId,
    values: [f32; 2],
) -> CxId {
    let cx_id = vault.cx_id_for_input(raw, PANEL_VERSION);
    let mut slots = BTreeMap::new();
    slots.insert(
        slot,
        SlotVector::Dense {
            dim: 2,
            data: values.to_vec(),
        },
    );
    let cx = constellation(vault, raw, cx_id, slots);
    vault.put(cx).unwrap();
    cx_id
}

fn put_base(vault: &AsterVault<SystemClock>, raw: &[u8]) -> CxId {
    let cx_id = vault.cx_id_for_input(raw, PANEL_VERSION);
    vault
        .put(constellation(vault, raw, cx_id, BTreeMap::new()))
        .unwrap();
    cx_id
}

fn constellation(
    vault: &AsterVault<SystemClock>,
    raw: &[u8],
    cx_id: CxId,
    slots: BTreeMap<SlotId, SlotVector>,
) -> Constellation {
    Constellation {
        cx_id,
        vault_id: vault.vault_id(),
        panel_version: PANEL_VERSION,
        created_at: 1,
        input_ref: InputRef {
            hash: *blake3::hash(raw).as_bytes(),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: lref(0),
        flags: CxFlags {
            ungrounded: true,
            redacted_input: true,
            ..CxFlags::default()
        },
    }
}

fn seed_setup_ledger(vault: &AsterVault<SystemClock>) {
    vault
        .append_ledger_entry(
            EntryKind::Admin,
            SubjectId::Query(b"issue755-setup".to_vec()),
            br#"{"issue":755,"setup":true}"#.to_vec(),
            ActorId::System,
        )
        .unwrap();
}

fn all_audit_entries(vault: &AsterVault<SystemClock>) -> Vec<calyx_loom::AuditEntry> {
    reactive_rows(vault)
        .into_iter()
        .filter_map(|(key, value)| {
            let parts = reactive_row_key(&key).unwrap();
            (parts.kind == ReactiveRowKind::Audit).then(|| decode_audit_entry(&value).unwrap())
        })
        .collect()
}

fn audit_entries(
    vault: &AsterVault<SystemClock>,
    trigger: calyx_loom::TriggerId,
) -> Vec<calyx_loom::AuditEntry> {
    all_audit_entries(vault)
        .into_iter()
        .filter(|entry| entry.trigger_id == trigger)
        .collect()
}

fn fired_events(vault: &AsterVault<SystemClock>) -> Vec<calyx_loom::TriggerFired> {
    reactive_rows(vault)
        .into_iter()
        .filter_map(|(key, value)| {
            let parts = reactive_row_key(&key).unwrap();
            (parts.kind == ReactiveRowKind::Fired).then(|| decode_trigger_fired(&value).unwrap())
        })
        .collect()
}

fn reactive_rows(vault: &AsterVault<SystemClock>) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Reactive)
        .unwrap()
}

fn ledger_hash(vault: &AsterVault<SystemClock>, seq: u64) -> [u8; 32] {
    let bytes = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .unwrap()
        .expect("ledger row");
    calyx_ledger::decode(&bytes).unwrap().entry_hash
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

fn guard_profile() -> GuardProfile {
    let guard_id: GuardId = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101".parse().unwrap();
    GuardProfile {
        guard_id,
        panel_version: u64::from(PANEL_VERSION),
        domain: "issue755".to_string(),
        tau: BTreeMap::from([(SLOT_CONTENT, 0.80)]),
        required_slots: vec![SLOT_CONTENT],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::NewRegion,
    }
}

fn temporal_vec(index: usize) -> [f32; 2] {
    match index {
        0 => [1.0, 0.0],
        1 => [0.0, 1.0],
        _ => [-1.0, 0.0],
    }
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
    std::env::temp_dir().join(format!("calyx-issue755-{name}-{}-{id}", std::process::id()))
}

fn root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE755_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-issue755-reactive-fsv"))
}

fn clean(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
}

fn write_artifact(name: &str, value: serde_json::Value) {
    let root = root();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join(name), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
