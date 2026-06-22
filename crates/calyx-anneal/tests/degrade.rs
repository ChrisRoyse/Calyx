use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    AsterAnnealLedgerStore, AsterHealthStore, CALYX_ANNEAL_HEAL_CONFIRMATION_REQUIRED,
    CALYX_ASTER_CF_UNAVAILABLE, ComponentHealth, ComponentKind, DegradeRegistry, HealthStorage,
    ScopeId, decode_health_value,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, Clock, FixedClock, LensId, Result, Seq, SlotId};
use calyx_ledger::{ActorId, LedgerAppender, MemoryLedgerStore};
use proptest::prelude::*;
use serde_json::json;

mod fsv_support;
use fsv_support::vault_id;

const TEST_TS: u64 = 1_785_600_400;

#[test]
fn failing_lens_is_excluded_from_active_lenses() {
    let store = MemoryHealthStore::default();
    let mut registry = memory_registry(store);
    let mut ledger = memory_ledger();
    let l1 = lens(1);
    let l2 = lens(2);

    registry
        .set_health(
            ComponentKind::lens_endpoint(l1),
            ComponentHealth::failing(TEST_TS, "synthetic endpoint down"),
            &mut ledger,
        )
        .unwrap();

    assert_eq!(registry.active_lenses(&[l1, l2]), vec![l2]);
}

#[test]
fn degraded_ann_requires_explicit_heal_confirmation() {
    let store = MemoryHealthStore::default();
    let mut registry = memory_registry(store);
    let mut ledger = memory_ledger();
    let ann = ComponentKind::ann_index(SlotId::new(0));

    registry
        .set_health(
            ann.clone(),
            ComponentHealth::degraded(TEST_TS, "checksum mismatch"),
            &mut ledger,
        )
        .unwrap();
    let error = registry
        .set_health(ann.clone(), ComponentHealth::Ok, &mut ledger)
        .unwrap_err();
    assert_eq!(error.code, CALYX_ANNEAL_HEAL_CONFIRMATION_REQUIRED);
    assert!(matches!(
        registry.health(&ann),
        ComponentHealth::Degraded { .. }
    ));

    registry.confirm_healed(ann.clone(), &mut ledger).unwrap();
    assert_eq!(registry.health(&ann), &ComponentHealth::Ok);
}

#[test]
fn edges_unknown_ok_all_failing_and_single_degraded() {
    let store = MemoryHealthStore::default();
    let mut registry = memory_registry(store);
    let mut ledger = memory_ledger();
    let l1 = lens(1);
    let l2 = lens(2);
    let unknown = ComponentKind::KernelIndex {
        scope: ScopeId::new("synthetic-scope"),
    };

    assert_eq!(registry.health(&unknown), &ComponentHealth::Ok);
    registry
        .set_health(
            ComponentKind::lens_endpoint(l1),
            ComponentHealth::failing(TEST_TS, "down"),
            &mut ledger,
        )
        .unwrap();
    registry
        .set_health(
            ComponentKind::lens_endpoint(l2),
            ComponentHealth::failing(TEST_TS, "down"),
            &mut ledger,
        )
        .unwrap();
    assert_eq!(registry.active_lenses(&[l1, l2]), Vec::<LensId>::new());

    let ann = ComponentKind::ann_index(SlotId::new(9));
    registry
        .set_health(
            ann.clone(),
            ComponentHealth::degraded(TEST_TS, "single degraded"),
            &mut ledger,
        )
        .unwrap();
    assert!(
        registry
            .degraded_components()
            .contains(&(ann, ComponentHealth::degraded(TEST_TS, "single degraded")))
    );
}

#[test]
fn reload_from_health_store_restores_snapshot() {
    let store = MemoryHealthStore::default();
    let mut registry = memory_registry(store.clone());
    let mut ledger = memory_ledger();
    let l1 = ComponentKind::lens_endpoint(lens(7));

    registry
        .set_health(
            l1.clone(),
            ComponentHealth::parked(TEST_TS, "below 0.05 bits"),
            &mut ledger,
        )
        .unwrap();

    let reopened = memory_registry(store);
    assert_eq!(
        reopened.health(&l1),
        &ComponentHealth::parked(TEST_TS, "below 0.05 bits")
    );
}

#[test]
fn cf_failure_surfaces_error_and_memory_state_survives() {
    let store = MemoryHealthStore::default();
    store.fail_next(CALYX_ASTER_CF_UNAVAILABLE);
    let mut registry = memory_registry(store.clone());
    let mut ledger = memory_ledger();
    let ann = ComponentKind::ann_index(SlotId::new(3));

    let error = registry
        .set_health(
            ann.clone(),
            ComponentHealth::degraded(TEST_TS, "persist outage"),
            &mut ledger,
        )
        .unwrap_err();

    assert_eq!(error.code, CALYX_ASTER_CF_UNAVAILABLE);
    assert_eq!(
        registry.health(&ann),
        &ComponentHealth::degraded(TEST_TS, "persist outage")
    );
    assert!(store.rows().is_empty());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn active_lenses_never_returns_failing_or_parked(ops in prop::collection::vec((0u8..4, 0u8..4), 1..64)) {
        let store = MemoryHealthStore::default();
        let mut registry = memory_registry(store);
        let mut ledger = memory_ledger();
        let all = [lens(1), lens(2), lens(3), lens(4)];

        for (lens_index, state) in ops {
            let lens_id = all[lens_index as usize % all.len()];
            let kind = ComponentKind::lens_endpoint(lens_id);
            let health = match state {
                0 => ComponentHealth::Ok,
                1 => ComponentHealth::failing(TEST_TS, "prop failing"),
                2 => ComponentHealth::parked(TEST_TS, "prop parked"),
                _ => ComponentHealth::degraded(TEST_TS, "prop degraded"),
            };
            let _ = registry.set_health(kind, health, &mut ledger);
            for active in registry.active_lenses(&all) {
                let active_kind = ComponentKind::lens_endpoint(active);
                prop_assert!(!registry.health(&active_kind).excludes_lens());
            }
        }
    }
}

#[ignore = "manual FSV for #400 anneal_health CF and status readback"]
#[test]
fn ph44_degrade_registry_manual_fsv() {
    let root = fsv_root();
    let vault_dir = reset_dir(&root.join(format!("vault-{}", std::process::id())));
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue400-health-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let mut registry = DegradeRegistry::open(clock(), AsterHealthStore::new(&vault)).unwrap();
    let mut ledger = aster_ledger(&vault);
    let ann = ComponentKind::ann_index(SlotId::new(0));
    let l1 = lens(10);
    let l2 = lens(20);
    let before = registry.degraded_components();
    let unknown_health = registry
        .health(&ComponentKind::KernelIndex {
            scope: ScopeId::new("fsv-unknown-scope"),
        })
        .clone();

    registry
        .set_health(
            ann.clone(),
            ComponentHealth::degraded(TEST_TS, "fsv checksum mismatch"),
            &mut ledger,
        )
        .unwrap();
    let single_degraded = registry.degraded_components();
    let direct_ok_error = registry
        .set_health(ann.clone(), ComponentHealth::Ok, &mut ledger)
        .unwrap_err();
    registry
        .set_health(
            ComponentKind::lens_endpoint(l1),
            ComponentHealth::failing(TEST_TS + 1, "fsv endpoint down"),
            &mut ledger,
        )
        .unwrap();
    let active_after_one_failing = registry.active_lenses(&[l1, l2]);
    registry
        .set_health(
            ComponentKind::lens_endpoint(l2),
            ComponentHealth::failing(TEST_TS + 2, "fsv second endpoint down"),
            &mut ledger,
        )
        .unwrap();
    let active_after_all_failing = registry.active_lenses(&[l1, l2]);
    vault.flush().unwrap();

    let rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealHealth)
        .unwrap();
    let decoded = rows
        .iter()
        .map(|(_, value)| decode_health_value(value).unwrap())
        .collect::<Vec<_>>();
    let status_lines = decoded
        .iter()
        .map(|row| format!("{}: {}", row.kind, row.health))
        .collect::<Vec<_>>();
    let persist_edge = persist_failure_edge();

    assert!(before.is_empty());
    assert_eq!(single_degraded.len(), 1);
    assert_eq!(
        direct_ok_error.code,
        CALYX_ANNEAL_HEAL_CONFIRMATION_REQUIRED
    );
    assert_eq!(active_after_one_failing, vec![l2]);
    assert!(active_after_all_failing.is_empty());
    assert!(
        status_lines
            .iter()
            .any(|line| line.contains("AnnIndex(slot_0): Degraded"))
    );

    let fsv_dir = root.join("fsv");
    fs::create_dir_all(&fsv_dir).unwrap();
    let readback = json!({
        "source_of_truth": "Aster anneal_health CF plus DegradeRegistry::degraded_components",
        "vault": vault_dir,
        "before_degraded_components": before,
        "unknown_component_health": unknown_health,
        "single_degraded_component": single_degraded,
        "direct_degraded_to_ok_error": direct_ok_error.code,
        "cf_row_count": rows.len(),
        "decoded_rows": decoded,
        "status_lines": status_lines,
        "active_after_one_failing": active_after_one_failing.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "active_after_all_failing": active_after_all_failing.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "persist_failure_edge": persist_edge,
        "degraded_components_after": registry.degraded_components(),
    });
    let path = fsv_dir.join("ph44-degrade-health-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("PH44_DEGRADE_FSV {}", path.display());
}

fn persist_failure_edge() -> serde_json::Value {
    let store = MemoryHealthStore::default();
    store.fail_next(CALYX_ASTER_CF_UNAVAILABLE);
    let mut registry = memory_registry(store.clone());
    let mut ledger = memory_ledger();
    let ann = ComponentKind::ann_index(SlotId::new(88));
    let before = registry.degraded_components();
    let error = registry
        .set_health(
            ann.clone(),
            ComponentHealth::degraded(TEST_TS, "fsv injected CF outage"),
            &mut ledger,
        )
        .unwrap_err();
    json!({
        "before_degraded_components": before,
        "error_code": error.code,
        "memory_health_after": registry.health(&ann),
        "stored_row_count_after": store.rows().len(),
    })
}

#[derive(Clone, Default)]
struct MemoryHealthStore {
    inner: Arc<Mutex<MemoryHealthInner>>,
}

#[derive(Default)]
struct MemoryHealthInner {
    rows: BTreeMap<Vec<u8>, Vec<u8>>,
    fail_next: Option<CalyxError>,
}

impl MemoryHealthStore {
    fn fail_next(&self, code: &'static str) {
        self.inner.lock().unwrap().fail_next = Some(CalyxError {
            code,
            message: "injected anneal_health CF outage".to_string(),
            remediation: "restore health CF",
        });
    }

    fn rows(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.inner
            .lock()
            .unwrap()
            .rows
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

impl HealthStorage for MemoryHealthStore {
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<Seq> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(error) = inner.fail_next.take() {
            return Err(error);
        }
        inner.rows.insert(key, value);
        Ok(inner.rows.len() as Seq)
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self.rows())
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
        ActorId::Service("calyx-anneal-degrade-test".to_string()),
    )
    .unwrap()
}

fn aster_ledger<C: Clock>(
    vault: &AsterVault<C>,
) -> calyx_anneal::AnnealLedger<AsterAnnealLedgerStore<'_, C>, FixedClock> {
    let store = AsterAnnealLedgerStore::new(vault);
    let appender = LedgerAppender::open(store, FixedClock::new(TEST_TS)).unwrap();
    calyx_anneal::AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-degrade-fsv".to_string()),
    )
    .unwrap()
}

fn clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock::new(TEST_TS))
}

fn lens(byte: u8) -> LensId {
    LensId::from_bytes([byte; 16])
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-issue400-degrade-fsv"))
}

fn reset_dir(path: &Path) -> PathBuf {
    if path.exists() {
        fs::remove_dir_all(path).unwrap();
    }
    fs::create_dir_all(path).unwrap();
    path.to_path_buf()
}
