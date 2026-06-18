use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    BudgetConfig, BudgetEnforcer, BudgetHandle, BudgetProbe, BudgetProbeSample,
    CALYX_ANNEAL_BUDGET_EXHAUSTED, CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE, ProcStatBudgetProbe,
    budget_config_path, read_budget_config_from_vault,
};
use calyx_core::FixedClock;
use proptest::prelude::*;

const TEST_TS: u64 = 1_785_500_397;
const MIB: u64 = 1024 * 1024;

#[test]
fn cpu_headroom_succeeds_then_exhausts() {
    let clock = FixedClock::new(TEST_TS);
    let probe = ScriptedProbe::new(0.05, 0, true);
    let enforcer =
        BudgetEnforcer::with_probe(config(0.10, MIB, 100), &clock, probe.clone()).unwrap();

    let handle = enforcer.acquire(0.04, 128).expect("budget headroom");
    let status = enforcer.status().expect("status");
    assert_eq!(status.handles_active, 1);
    assert!((status.cpu_used_fraction - 0.09).abs() < 1e-12);
    drop(handle);

    probe.set(0.12, 0, true);
    expect_exhausted(enforcer.acquire(0.01, 0));
}

#[test]
fn drop_releases_reserved_pool() {
    let clock = FixedClock::new(TEST_TS);
    let enforcer = BudgetEnforcer::with_probe(
        config(0.50, 64, 100),
        &clock,
        ScriptedProbe::new(0.0, 0, true),
    )
    .unwrap();

    let handle = enforcer.acquire(0.10, 64).expect("first acquire");
    let active = enforcer.status().expect("active status");
    assert_eq!(active.handles_active, 1);
    assert_eq!(active.vram_used_bytes, 64);
    expect_exhausted(enforcer.acquire(0.0, 1));

    drop(handle);
    let released = enforcer.status().expect("released status");
    assert_eq!(released.handles_active, 0);
    assert_eq!(released.vram_used_bytes, 0);
    let next = enforcer.acquire(0.10, 64).expect("released capacity");
    drop(next);
}

#[test]
fn zero_capacity_configs_exhaust_without_blocking() {
    let clock = FixedClock::new(TEST_TS);
    let cpu_zero = BudgetEnforcer::with_probe(
        config(0.0, MIB, 100),
        &clock,
        ScriptedProbe::new(0.0, 0, true),
    )
    .unwrap();
    expect_exhausted(cpu_zero.acquire(0.0, 1));

    let vram_zero = BudgetEnforcer::with_probe(
        config(0.50, 0, 100),
        &clock,
        ScriptedProbe::new(0.0, 0, true),
    )
    .unwrap();
    expect_exhausted(vram_zero.acquire(0.01, 0));
}

#[test]
fn drop_after_enforcer_drop_does_not_panic() {
    let clock = FixedClock::new(TEST_TS);
    let enforcer = BudgetEnforcer::with_probe(
        config(0.50, MIB, 100),
        &clock,
        ScriptedProbe::new(0.0, 0, true),
    )
    .unwrap();

    let handle = enforcer
        .acquire(0.10, 512)
        .expect("acquire before shutdown");
    drop(enforcer);
    drop(handle);
}

#[test]
fn nvml_unavailable_uses_static_pool_warning() {
    let clock = FixedClock::new(TEST_TS);
    let enforcer = BudgetEnforcer::with_probe(
        config(0.20, MIB, 100),
        &clock,
        ScriptedProbe::new(0.03, 256, false),
    )
    .unwrap();

    let status = enforcer.tick().expect("tick");
    assert_eq!(
        status.warning_code.as_deref(),
        Some(CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE)
    );
    assert_eq!(status.vram_used_bytes, 256);
    let handle = enforcer.acquire(0.01, 256).expect("static pool capacity");
    assert_eq!(enforcer.status().unwrap().vram_used_bytes, 512);
    drop(handle);
}

#[test]
fn proc_stat_probe_first_sample_establishes_cpu_baseline() {
    let clock = FixedClock::new(TEST_TS);
    let enforcer = BudgetEnforcer::with_probe(
        config(0.01, MIB, 100),
        &clock,
        ProcStatBudgetProbe::default(),
    )
    .unwrap();

    let handle = enforcer
        .acquire(0.01, 0)
        .expect("first proc sample baseline");
    assert_eq!(enforcer.status().unwrap().handles_active, 1);
    drop(handle);
}

#[test]
fn load_from_vault_persists_default_config_bytes() {
    let root = TestRoot::new("config");

    let config = BudgetConfig::load_from_vault(root.path()).expect("load default config");
    assert_eq!(config, BudgetConfig::default());
    let path = budget_config_path(root.path());
    let bytes = fs::read_to_string(&path).expect("read persisted config bytes");
    assert!(bytes.contains("cpu_fraction = 0.15"));
    assert!(bytes.contains("vram_bytes = 536870912"));
    assert!(bytes.contains("tick_interval_ms = 100"));

    let readback = read_budget_config_from_vault(root.path()).expect("config readback");
    assert_eq!(readback.config_path, path);
    assert_eq!(readback.config, BudgetConfig::default());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn vram_status_never_exceeds_cap(
        ops in prop::collection::vec((any::<bool>(), 0_u64..40), 1..64)
    ) {
        let clock = FixedClock::new(TEST_TS);
        let enforcer = BudgetEnforcer::with_probe(
            config(1.0, 32, 100),
            &clock,
            ScriptedProbe::new(0.0, 0, true),
        )
        .unwrap();
        let mut handles = Vec::new();

        for (do_acquire, bytes) in ops {
            if do_acquire {
                if let Ok(handle) = enforcer.acquire(0.0, bytes) {
                    handles.push(handle);
                }
            } else {
                handles.pop();
            }
            prop_assert!(enforcer.status().unwrap().vram_used_bytes <= enforcer.config().vram_bytes);
        }
    }
}

#[derive(Clone)]
struct ScriptedProbe {
    sample: Arc<Mutex<BudgetProbeSample>>,
}

impl ScriptedProbe {
    fn new(cpu_used_fraction: f64, vram_used_bytes: u64, nvml_available: bool) -> Self {
        Self {
            sample: Arc::new(Mutex::new(BudgetProbeSample {
                cpu_used_fraction,
                vram_used_bytes,
                nvml_available,
            })),
        }
    }

    fn set(&self, cpu_used_fraction: f64, vram_used_bytes: u64, nvml_available: bool) {
        *self.sample.lock().unwrap() = BudgetProbeSample {
            cpu_used_fraction,
            vram_used_bytes,
            nvml_available,
        };
    }
}

impl BudgetProbe for ScriptedProbe {
    fn sample(&self) -> BudgetProbeSample {
        self.sample.lock().unwrap().clone()
    }
}

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "calyx-budget-{label}-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

fn config(cpu_fraction: f64, vram_bytes: u64, tick_interval_ms: u64) -> BudgetConfig {
    BudgetConfig {
        cpu_fraction,
        vram_bytes,
        tick_interval_ms,
    }
}

fn expect_exhausted(result: calyx_core::Result<BudgetHandle>) {
    match result {
        Ok(_) => panic!("expected budget exhaustion"),
        Err(error) => assert_eq!(error.code, CALYX_ANNEAL_BUDGET_EXHAUSTED),
    }
}
