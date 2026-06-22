use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_anneal::{
    ActionMetricSnapshot, AnnealLedger, AnnealSubstrate, AsterAnnealLedgerStore, AsterHealthStore,
    AsterRollbackStorage, BudgetConfig, BudgetEnforcer, BudgetHandle, BudgetProbe,
    BudgetProbeSample, CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET, CALYX_ANNEAL_UNPARK_THRESHOLD_NOT_MET,
    CALYX_WARD_RECALIBRATE_FAILED, ComponentHealth, ComponentKind, DegradeRegistry,
    FileWardTauStore, HealthStorage, HeldOutReplay, LensParkOutcome, NewTau, RecalibrationOutcome,
    ReplayAnchor, ReplayQuery, RollbackStorage, RollbackStore, TauDriftEvent, TripwireMetric,
    TripwireRegistry, WardRecalibrate, WardTauStore, decode_health_value, park_decayed_lens,
    trigger_tau_recalibration, unpark_lens,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, CxId, FixedClock, LensId, Result, SlotId, SystemClock};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use serde_json::{Value, json};

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use fsv_support::{reset_dir, vault_id, write_json};

const TEST_TS: u64 = 1_785_500_404;

#[test]
#[ignore = "requires CALYX_ISSUE404_FSV_ROOT in a manual verification run"]
fn fsv_tau_recalibration_and_lens_park_manual() {
    let root = PathBuf::from(env::var("CALYX_ISSUE404_FSV_ROOT").unwrap());
    reset_dir(&root);
    let (vault_dir, vault) = open_durable_vault(&root);
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = durable_substrate(&clock, &vault, &vault_dir);
    let mut registry =
        DegradeRegistry::open(Arc::new(clock), AsterHealthStore::new(&vault)).unwrap();
    let slot_id = SlotId::new(0);
    let lens_id = lens(1);
    let other_lens = lens(2);
    let alerts_path = vault_dir.join("alerts.jsonl");
    let mut tau_store = FileWardTauStore::open(&vault_dir).unwrap();
    tau_store
        .upsert_current(slot_id, 0.70, TEST_TS - 1)
        .unwrap();
    registry
        .set_health(
            ComponentKind::GuardProfile { slot_id },
            ComponentHealth::degraded(TEST_TS - 1, "synthetic FAR creep"),
            &mut substrate.ledger,
        )
        .unwrap();

    let before = json!({
        "ledger_rows": read_ledger_rows(&vault),
        "health_rows": read_health_rows(&vault),
        "ward_tau_file": read_text(tau_store.path()),
        "active_lenses": active_lenses(&registry, &[lens_id, other_lens]),
    });
    let outcome = trigger_tau_recalibration(
        &StaticWard::ok(new_tau(slot_id, 0.82, 0.001, 0.002, metrics(0.96, 0.001))),
        &mut tau_store,
        &mut registry,
        slot_id,
        &drift_event(slot_id, 0.70, metrics(0.91, 0.002)),
        &mut substrate,
    )
    .unwrap();
    let park_outcome = park_decayed_lens(
        lens_id,
        0.02,
        &mut registry,
        &mut substrate.ledger,
        &clock,
        &alerts_path,
    )
    .unwrap();
    vault.flush().unwrap();

    assert!(matches!(outcome, RecalibrationOutcome::Promoted { .. }));
    assert_eq!(tau_store.current_tau(slot_id).unwrap(), Some(0.82));
    assert_eq!(park_outcome, LensParkOutcome::Parked { lens_id });
    assert_eq!(
        registry.active_lenses(&[lens_id, other_lens]),
        vec![other_lens]
    );

    let park_edge = park_threshold_edge(
        other_lens,
        &mut registry,
        &mut substrate,
        &clock,
        &alerts_path,
        &vault,
    );
    let unpark_edge =
        unpark_threshold_edge(lens_id, &mut registry, &mut substrate, &clock, &alerts_path);
    let ward_edge = ward_failure_edge(
        slot_id,
        &mut tau_store,
        &mut registry,
        &mut substrate,
        &vault,
    );
    vault.flush().unwrap();

    let after = json!({
        "ledger_rows": read_ledger_rows(&vault),
        "health_rows": read_health_rows(&vault),
        "ward_tau_file": read_text(tau_store.path()),
        "alerts_jsonl": read_text(&alerts_path),
        "active_lenses": active_lenses(&registry, &[lens_id, other_lens]),
    });
    assert!(ledger_has_action(&after["ledger_rows"], "tau_recalibrated"));
    assert!(ledger_has_action(&after["ledger_rows"], "lens_park"));
    assert_eq!(after["active_lenses"], json!([other_lens.to_string()]));

    let readback = json!({
        "surface": "PH44.T05.tau_recalibration_lens_park",
        "source_of_truth": [
            "Aster ledger CF",
            "Aster anneal_health CF",
            "vault/.anneal/ward_tau.json",
            "vault/alerts.jsonl",
            "DegradeRegistry::active_lenses"
        ],
        "vault": vault_dir.display().to_string(),
        "before": before,
        "trigger": {
            "tau": "slot 0 current tau 0.70 with observed FAR creep",
            "lens": "lens L1 bits_per_anchor = 0.02"
        },
        "happy_after": after,
        "edges": [park_edge, unpark_edge, ward_edge],
        "expected": {
            "tau": 0.82,
            "ledger_actions": ["tau_recalibrated", "lens_park"],
            "parked_lens_removed_from_active_lenses": true,
            "edge_error_codes": [
                CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET,
                CALYX_ANNEAL_UNPARK_THRESHOLD_NOT_MET,
                CALYX_WARD_RECALIBRATE_FAILED
            ]
        }
    });
    write_json(&root.join("issue404-readback.json"), &readback);
    println!(
        "ISSUE404_FSV_READBACK {}",
        root.join("issue404-readback.json").display()
    );
}

fn park_threshold_edge<R, L, C, P>(
    lens_id: LensId,
    registry: &mut DegradeRegistry<AsterHealthStore<'_, SystemClock>>,
    substrate: &mut AnnealSubstrate<'_, R, L, C, P>,
    clock: &FixedClock,
    alerts_path: &Path,
    vault: &AsterVault,
) -> Value
where
    R: RollbackStorage,
    L: calyx_ledger::LedgerCfStore,
    C: calyx_core::Clock,
    P: BudgetProbe,
{
    let before = read_health_rows(vault);
    let error = park_decayed_lens(
        lens_id,
        0.06,
        registry,
        &mut substrate.ledger,
        clock,
        alerts_path,
    )
    .unwrap_err();
    let after = read_health_rows(vault);
    assert_eq!(error.code, CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET);
    assert_eq!(before, after);
    json!({
        "edge": "park bits 0.06 is above decay threshold",
        "before": before,
        "error_code": error.code,
        "after": after
    })
}

fn unpark_threshold_edge<R, L, C, P>(
    lens_id: LensId,
    registry: &mut DegradeRegistry<AsterHealthStore<'_, SystemClock>>,
    substrate: &mut AnnealSubstrate<'_, R, L, C, P>,
    clock: &FixedClock,
    alerts_path: &Path,
) -> Value
where
    R: RollbackStorage,
    L: calyx_ledger::LedgerCfStore,
    C: calyx_core::Clock,
    P: BudgetProbe,
{
    let before = active_lenses(registry, &[lens_id]);
    let error = unpark_lens(
        lens_id,
        0.03,
        registry,
        &mut substrate.ledger,
        clock,
        alerts_path,
    )
    .unwrap_err();
    let after = active_lenses(registry, &[lens_id]);
    assert_eq!(error.code, CALYX_ANNEAL_UNPARK_THRESHOLD_NOT_MET);
    assert_eq!(before, after);
    json!({
        "edge": "unpark bits 0.03 is below restore threshold",
        "before_active_lenses": before,
        "error_code": error.code,
        "after_active_lenses": after
    })
}

fn ward_failure_edge<R, L, C, P>(
    slot_id: SlotId,
    tau_store: &mut FileWardTauStore,
    registry: &mut DegradeRegistry<AsterHealthStore<'_, SystemClock>>,
    substrate: &mut AnnealSubstrate<'_, R, L, C, P>,
    vault: &AsterVault,
) -> Value
where
    R: RollbackStorage,
    L: calyx_ledger::LedgerCfStore,
    C: calyx_core::Clock,
    P: BudgetProbe,
{
    let before_tau = tau_store.current_tau(slot_id).unwrap();
    let before_ledger = read_ledger_rows(vault);
    let error = trigger_tau_recalibration(
        &StaticWard::err("CALYX_SYNTHETIC_WARD_DOWN"),
        tau_store,
        registry,
        slot_id,
        &drift_event(slot_id, 0.82, metrics(0.90, 0.010)),
        substrate,
    )
    .unwrap_err();
    assert_eq!(error.code, CALYX_WARD_RECALIBRATE_FAILED);
    assert_eq!(tau_store.current_tau(slot_id).unwrap(), before_tau);
    json!({
        "edge": "WardRecalibrate error fails closed",
        "before_tau": before_tau,
        "before_ledger_len": before_ledger.len(),
        "error_code": error.code,
        "after_tau": tau_store.current_tau(slot_id).unwrap(),
        "after_ledger_rows": read_ledger_rows(vault)
    })
}

#[derive(Clone)]
struct StaticWard {
    response: Result<NewTau>,
}

impl StaticWard {
    fn ok(tau: NewTau) -> Self {
        Self { response: Ok(tau) }
    }

    fn err(code: &'static str) -> Self {
        Self {
            response: Err(CalyxError {
                code,
                message: "synthetic Ward recalibration outage".to_string(),
                remediation: "test fixture",
            }),
        }
    }
}

impl WardRecalibrate for StaticWard {
    fn recalibrate(&self, slot_id: SlotId, snapshot: u64, _budget: BudgetHandle) -> Result<NewTau> {
        assert_eq!(snapshot, 7);
        let response = self.response.clone()?;
        assert_eq!(response.slot_id, slot_id);
        Ok(response)
    }
}

#[derive(Clone)]
struct ScriptedProbe {
    sample: BudgetProbeSample,
}

impl BudgetProbe for ScriptedProbe {
    fn sample(&self) -> BudgetProbeSample {
        self.sample.clone()
    }
}

fn durable_substrate<'a>(
    clock: &'a FixedClock,
    vault: &'a AsterVault,
    vault_dir: &Path,
) -> AnnealSubstrate<
    'a,
    AsterRollbackStorage<'a, SystemClock>,
    AsterAnnealLedgerStore<'a, SystemClock>,
    FixedClock,
    ScriptedProbe,
> {
    let rollback = RollbackStore::open(clock, 9, AsterRollbackStorage::new(vault)).unwrap();
    let appender = LedgerAppender::open(AsterAnnealLedgerStore::new(vault), *clock).unwrap();
    let ledger =
        AnnealLedger::new(appender, ActorId::Service("calyx-anneal-fsv".to_string())).unwrap();
    let budget = BudgetEnforcer::with_probe(budget_config(), clock, scripted_probe()).unwrap();
    AnnealSubstrate::new(
        TripwireRegistry::load_from_vault(vault_dir).unwrap(),
        replay(),
        rollback,
        ledger,
        budget,
        clock,
    )
}

fn budget_config() -> BudgetConfig {
    BudgetConfig {
        cpu_fraction: 1.0,
        vram_bytes: 1024,
        tick_interval_ms: 100,
    }
}

fn replay() -> HeldOutReplay {
    HeldOutReplay {
        queries: vec![ReplayQuery {
            query_id: 404,
            query_vector: vec![1.0, 0.0],
            expected_top_k: vec![ReplayAnchor {
                cx_id: CxId::from_bytes([4; 16]),
                similarity: 1.0,
            }],
        }],
        seed: 404,
    }
}

fn scripted_probe() -> ScriptedProbe {
    ScriptedProbe {
        sample: BudgetProbeSample {
            cpu_used_fraction: 0.0,
            vram_used_bytes: 0,
            nvml_available: true,
        },
    }
}

fn new_tau(slot_id: SlotId, tau: f32, far: f64, frr: f64, metrics: ActionMetricSnapshot) -> NewTau {
    NewTau::new(slot_id, tau, far, frr, metrics).unwrap()
}

fn drift_event(
    slot_id: SlotId,
    current_tau: f32,
    incumbent_metrics: ActionMetricSnapshot,
) -> TauDriftEvent {
    TauDriftEvent::new(
        slot_id,
        current_tau,
        0.020,
        0.005,
        TEST_TS,
        7,
        incumbent_metrics,
    )
    .unwrap()
}

fn metrics(recall: f64, far: f64) -> ActionMetricSnapshot {
    ActionMetricSnapshot::from_values([
        (TripwireMetric::RecallAtK, recall),
        (TripwireMetric::GuardFAR, far),
        (TripwireMetric::GuardFRR, 0.001),
        (TripwireMetric::SearchP99, 50.0),
        (TripwireMetric::IngestP95, 80.0),
    ])
}

fn active_lenses<S: HealthStorage>(
    registry: &DegradeRegistry<S>,
    lenses: &[LensId],
) -> Vec<String> {
    registry
        .active_lenses(lenses)
        .into_iter()
        .map(|lens| lens.to_string())
        .collect()
}

fn read_ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .map(|(key, bytes)| {
            let entry = decode_ledger(&bytes).unwrap();
            assert_eq!(entry.kind, EntryKind::Anneal);
            assert_eq!(key, ledger_key(entry.seq));
            json!({
                "seq": entry.seq,
                "key_hex": hex(&key),
                "entry_hash": hex(&entry.entry_hash),
                "payload_json": serde_json::from_slice::<Value>(&entry.payload).unwrap()
            })
        })
        .collect()
}

fn read_health_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealHealth)
        .unwrap()
        .into_iter()
        .map(|(key, bytes)| {
            let row = decode_health_value(&bytes).unwrap();
            json!({
                "key_hex": hex(&key),
                "kind": row.kind.to_string(),
                "health": row.health.to_string(),
                "updated_at": row.updated_at
            })
        })
        .collect()
}

fn ledger_has_action(rows: &Value, action: &str) -> bool {
    rows.as_array().unwrap().iter().any(|row| {
        row["payload_json"]["action"]
            .as_str()
            .is_some_and(|value| value == action)
    })
}

fn open_durable_vault(root: &Path) -> (PathBuf, AsterVault) {
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).unwrap();
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue404-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    (vault_dir, vault)
}

fn lens(seed: u8) -> LensId {
    LensId::from_parts(
        &format!("issue404-lens-{seed}"),
        &[seed; 32],
        &[seed.wrapping_add(1); 32],
        b"2",
    )
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
