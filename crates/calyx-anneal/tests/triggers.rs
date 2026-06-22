use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    AnnealLedgerAction, AssayMetrics, AsterAnnealLedgerStore, AsterHealthStore, BudgetHandle,
    ChecksumDetector, ChecksumEntry, ComponentHealth, ComponentKind, DegradeRegistry, EndpointUrl,
    FaultDetector, FaultKind, FaultMonitor, HealthStorage, HttpProbe, LensProbeDetector,
    ProbeStatus, SignalDecayDetector, SignalSample, StaleDetector, StaleEntry, TauDriftDetector,
    TauDriftSample, WardMetrics, decode_anneal_ledger_payload, decode_health_value,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, Clock, FixedClock, LensId, Result, Seq, SlotId};
use calyx_ledger::{ActorId, LedgerAppender, MemoryLedgerStore};
use proptest::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};

mod fsv_support;
use fsv_support::vault_id;

const TEST_TS: u64 = 1_785_601_400;

#[test]
fn checksum_detector_fires_only_on_mismatch() {
    let path = write_temp_file("checksum-good.bin", b"good-ann-index");
    let component = ComponentKind::ann_index(SlotId::new(0));
    let registry = memory_registry(MemoryHealthStore::default());
    let good = ChecksumDetector::new(
        vec![(
            component.clone(),
            ChecksumEntry::new(&path, sha256(b"good-ann-index")),
        )],
        clock(),
    );
    assert!(good.check(&registry).is_empty());

    let bad = ChecksumDetector::new(
        vec![(component.clone(), ChecksumEntry::new(&path, [0; 32]))],
        clock(),
    );
    let events = bad.check(&registry);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].component, component);
    assert_eq!(events[0].fault_kind, FaultKind::Corruption);
}

#[test]
fn signal_decay_detector_uses_exclusive_signal_floor() {
    let registry = memory_registry(MemoryHealthStore::default());
    let detector = SignalDecayDetector::new(
        Arc::new(StaticAssay {
            samples: Ok(vec![signal(1, 0.04), signal(2, 0.06)]),
        }),
        ComponentKind::lens_endpoint(lens(9)),
        clock(),
    );

    let events = detector.check(&registry);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].component, ComponentKind::lens_endpoint(lens(1)));
    assert_eq!(events[0].fault_kind, FaultKind::SignalDecayed);
}

proptest! {
    #[test]
    fn signal_decay_fires_iff_bits_below_floor(bits in 0.0f64..0.1) {
        let registry = memory_registry(MemoryHealthStore::default());
        let detector = SignalDecayDetector::new(
            Arc::new(StaticAssay { samples: Ok(vec![signal(1, bits)]) }),
            ComponentKind::lens_endpoint(lens(9)),
            clock(),
        );
        prop_assert_eq!(detector.check(&registry).is_empty(), bits >= 0.05);
    }
}

#[test]
fn edge_cases_cover_all_probe_fail_tau_boundary_and_empty_stale() {
    let registry = memory_registry(MemoryHealthStore::default());
    let l1 = lens(1);
    let l2 = lens(2);
    let probes = LensProbeDetector::new(
        vec![
            (l1, EndpointUrl::new("http://tei-1/health")),
            (l2, EndpointUrl::new("http://tei-2/health")),
        ],
        Arc::new(TimeoutProbe),
        clock(),
    );
    assert_eq!(probes.check(&registry).len(), 2);

    let tau = TauDriftDetector::new(
        Arc::new(StaticWard {
            samples: Ok(vec![TauDriftSample {
                component: ComponentKind::GuardProfile {
                    slot_id: SlotId::new(1),
                },
                tau: 0.80,
                far: 0.81,
                drift_tolerance: 0.01,
            }]),
        }),
        ComponentKind::GuardProfile {
            slot_id: SlotId::new(1),
        },
        clock(),
    );
    assert!(tau.check(&registry).is_empty());

    let stale = StaleDetector::new(Vec::<StaleEntry>::new(), 60, clock());
    assert!(stale.check(&registry).is_empty());
}

#[test]
fn fail_closed_metrics_error_and_probe_panic_emit_faults() {
    let registry = memory_registry(MemoryHealthStore::default());
    let metrics_error = TauDriftDetector::new(
        Arc::new(StaticWard {
            samples: Err(test_error("CALYX_WARD_METRICS_UNAVAILABLE")),
        }),
        ComponentKind::GuardProfile {
            slot_id: SlotId::new(4),
        },
        clock(),
    );
    assert_eq!(
        metrics_error.check(&registry)[0].fault_kind,
        FaultKind::MetricsUnavailable
    );

    let probe_panic = LensProbeDetector::new(
        vec![(lens(7), EndpointUrl::new("http://tei-panic/health"))],
        Arc::new(PanicProbe),
        clock(),
    );
    let events = with_silent_panic_hook(|| probe_panic.check(&registry));
    assert_eq!(events[0].fault_kind, FaultKind::ProbeError);
}

#[test]
fn monitor_applies_registry_state_and_logs_fault_event() {
    let path = write_temp_file("monitor-bad.bin", b"changed");
    let store = MemoryHealthStore::default();
    let mut registry = memory_registry(store);
    let mut ledger = memory_ledger();
    let component = ComponentKind::ann_index(SlotId::new(3));
    let detector = ChecksumDetector::new(
        vec![(component.clone(), ChecksumEntry::new(path, [0; 32]))],
        clock(),
    );
    let mut monitor = FaultMonitor::new(vec![Box::new(detector)], BudgetHandle::new(8), 10_000);

    let events = monitor.run_once(&mut registry, &mut ledger).unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        registry.health(&component),
        ComponentHealth::Degraded { .. }
    ));
    let recent = ledger.read_recent(2).unwrap();
    assert!(
        recent
            .iter()
            .any(|entry| entry.action == AnnealLedgerAction::FaultEvent)
    );
    assert!(recent.iter().any(|entry| entry.fault.is_some()));
}

#[ignore = "manual FSV for #401 fault ledger and health readback"]
#[test]
fn ph44_fault_detectors_manual_fsv() {
    let root = fsv_root();
    let vault_dir = reset_dir(&root.join(format!("vault-{}", std::process::id())));
    let artifact = root.join("ann-index.bin");
    fs::create_dir_all(&root).unwrap();
    fs::write(&artifact, b"known ann bytes").unwrap();
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue401-fault-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let mut registry = DegradeRegistry::open(clock(), AsterHealthStore::new(&vault)).unwrap();
    let mut ledger = aster_ledger(&vault);
    let component = ComponentKind::ann_index(SlotId::new(0));
    let before = registry.health(&component).clone();
    let detector = ChecksumDetector::new(
        vec![(component.clone(), ChecksumEntry::new(&artifact, [0; 32]))],
        clock(),
    );
    let mut monitor = FaultMonitor::new(vec![Box::new(detector)], BudgetHandle::new(8), 10_000);

    let events = monitor.run_once(&mut registry, &mut ledger).unwrap();
    vault.flush().unwrap();
    let health_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealHealth)
        .unwrap();
    let ledger_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap();
    let decoded_health = health_rows
        .iter()
        .map(|(_, value)| decode_health_value(value).unwrap())
        .collect::<Vec<_>>();
    let fault_payloads = ledger_rows
        .iter()
        .filter_map(|(_, value)| calyx_ledger::decode(value).ok())
        .filter_map(|entry| decode_anneal_ledger_payload(&entry.payload).ok())
        .filter(|entry| entry.fault.is_some())
        .collect::<Vec<_>>();
    let fsv_dir = root.join("fsv");
    fs::create_dir_all(&fsv_dir).unwrap();
    let readback = json!({
        "source_of_truth": "Aster anneal_health CF plus ledger FaultEvent rows",
        "vault": vault_dir,
        "artifact": artifact,
        "before_health": before,
        "events": events,
        "registry_after": registry.health(&component),
        "decoded_health": decoded_health,
        "fault_payloads": fault_payloads,
    });
    let path = fsv_dir.join("ph44-fault-detectors-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("PH44_FAULT_FSV {}", path.display());
}

#[derive(Clone, Default)]
struct MemoryHealthStore {
    inner: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl HealthStorage for MemoryHealthStore {
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<Seq> {
        let mut rows = self.inner.lock().unwrap();
        rows.insert(key, value);
        Ok(rows.len() as Seq)
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

#[derive(Clone)]
struct StaticAssay {
    samples: Result<Vec<SignalSample>>,
}

impl AssayMetrics for StaticAssay {
    fn signal_samples(&self) -> Result<Vec<SignalSample>> {
        self.samples.clone()
    }
}

#[derive(Clone)]
struct StaticWard {
    samples: Result<Vec<TauDriftSample>>,
}

impl WardMetrics for StaticWard {
    fn tau_drift_samples(&self) -> Result<Vec<TauDriftSample>> {
        self.samples.clone()
    }
}

struct TimeoutProbe;

impl HttpProbe for TimeoutProbe {
    fn probe(&self, _endpoint: &EndpointUrl) -> Result<ProbeStatus> {
        Err(test_error("CALYX_LENS_UNREACHABLE"))
    }
}

struct PanicProbe;

impl HttpProbe for PanicProbe {
    fn probe(&self, _endpoint: &EndpointUrl) -> Result<ProbeStatus> {
        panic!("synthetic probe panic")
    }
}

fn memory_registry(store: MemoryHealthStore) -> DegradeRegistry<MemoryHealthStore> {
    DegradeRegistry::open(clock(), store).unwrap()
}

fn memory_ledger() -> calyx_anneal::AnnealLedger<MemoryLedgerStore, FixedClock> {
    let appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(TEST_TS)).unwrap();
    calyx_anneal::AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-trigger-test".to_string()),
    )
    .unwrap()
}

fn aster_ledger<C: Clock>(
    vault: &AsterVault<C>,
) -> calyx_anneal::AnnealLedger<AsterAnnealLedgerStore<'_, C>, FixedClock> {
    let appender =
        LedgerAppender::open(AsterAnnealLedgerStore::new(vault), FixedClock::new(TEST_TS)).unwrap();
    calyx_anneal::AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-trigger-fsv".to_string()),
    )
    .unwrap()
}

fn signal(byte: u8, bits_per_anchor: f64) -> SignalSample {
    SignalSample {
        lens_id: lens(byte),
        bits_per_anchor,
    }
}

fn lens(byte: u8) -> LensId {
    LensId::from_bytes([byte; 16])
}

fn clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock::new(TEST_TS))
}

fn test_error(code: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: "synthetic fault".to_string(),
        remediation: "synthetic remediation",
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn write_temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("calyx-issue401-{name}-{}", std::process::id()));
    fs::write(&path, bytes).unwrap();
    path
}

fn with_silent_panic_hook<T>(work: impl FnOnce() -> T) -> T {
    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = work();
    std::panic::set_hook(old_hook);
    result
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-issue401-fault-fsv"))
}

fn reset_dir(path: &Path) -> PathBuf {
    if path.exists() {
        fs::remove_dir_all(path).unwrap();
    }
    fs::create_dir_all(path).unwrap();
    path.to_path_buf()
}
