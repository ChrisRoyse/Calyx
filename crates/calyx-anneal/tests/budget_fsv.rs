use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use calyx_anneal::{
    BACKGROUND_NICE, BudgetConfig, BudgetEnforcer, BudgetProbe, BudgetProbeSample, BudgetStatus,
    CALYX_ANNEAL_BUDGET_EXHAUSTED, CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE, budget_config_path,
    read_budget_config_from_vault,
};
use calyx_core::FixedClock;
use fsv_support::{write_json, write_manifest};
use serde_json::json;

const FSV_TS: u64 = 1_785_500_397;
const MIB: u64 = 1024 * 1024;

#[test]
#[ignore = "requires CALYX_ISSUE397_FSV_ROOT in a manual verification run"]
fn issue397_budget_enforcer_fsv() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE397_FSV_ROOT").expect("set CALYX_ISSUE397_FSV_ROOT"));
    fs::create_dir_all(&root).expect("create FSV root");
    let vault = root.join("vault");
    fs::create_dir_all(&vault).expect("create FSV vault");

    let config = BudgetConfig::load_from_vault(&vault).expect("load budget config");
    assert_eq!(config, BudgetConfig::default());
    let config_readback = read_budget_config_from_vault(&vault).expect("read back config bytes");
    let config_path = budget_config_path(&vault);
    let config_text = fs::read_to_string(&config_path).expect("read budget config bytes");

    let clock = FixedClock::new(FSV_TS);
    let probe = ScriptedProbe::new(0.05, 128 * MIB, false);
    let enforcer =
        BudgetEnforcer::with_probe(config, &clock, probe).expect("budget enforcer from config");

    let before = enforcer.tick().expect("tick before acquire");
    let handle = enforcer
        .acquire(0.04, 128 * MIB)
        .expect("acquire within budget");
    let running = enforcer.status().expect("running status");
    assert!(running.cpu_used_fraction <= config.cpu_fraction);
    assert!(running.vram_used_bytes <= config.vram_bytes);
    assert_eq!(running.handles_active, 1);
    assert_eq!(
        running.warning_code.as_deref(),
        Some(CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE)
    );

    drop(handle);
    let after_drop = enforcer.status().expect("status after drop");
    assert_eq!(after_drop.handles_active, 0);
    assert_eq!(after_drop.vram_used_bytes, 128 * MIB);

    let status_path = root.join("budget-status-sequence.json");
    write_json(
        &status_path,
        &json!({
            "surface": "anneal.budget_enforcer",
            "source_of_truth": "BudgetStatus read from BudgetEnforcer state plus persisted vault .anneal/budget.toml bytes",
            "vault": vault.display().to_string(),
            "config_path": config_path.display().to_string(),
            "config_text": config_text,
            "config_readback": config_readback,
            "limits": {
                "cpu_fraction_max": config.cpu_fraction,
                "vram_bytes_max": config.vram_bytes,
                "tick_interval_ms": config.tick_interval_ms,
                "background_nice": BACKGROUND_NICE
            },
            "before_acquire": before,
            "running_background_task": running,
            "after_handle_drop": after_drop,
            "proof": "running_background_task.cpu_used_fraction <= 0.15 and vram_used_bytes <= 512MiB with one active handle"
        }),
    );

    let edges = json!([
        edge_cpu_exhausted(&clock),
        edge_vram_exhausted(&clock),
        edge_zero_config(&clock),
        edge_drop_after_enforcer_drop(&clock)
    ]);
    let edge_path = root.join("budget-edge-readback.json");
    write_json(&edge_path, &edges);

    let priority_path = root.join("background-priority.txt");
    fs::write(
        &priority_path,
        format!("background_nice={BACKGROUND_NICE}\n"),
    )
    .expect("write priority readback");

    write_manifest(&root, &[config_path, status_path, edge_path, priority_path]);
}

fn edge_cpu_exhausted(clock: &FixedClock) -> serde_json::Value {
    let enforcer = BudgetEnforcer::with_probe(
        BudgetConfig::default(),
        clock,
        ScriptedProbe::new(0.16, 0, false),
    )
    .expect("cpu edge enforcer");
    let before = enforcer.tick().expect("cpu edge before");
    let result_code = acquire_code(&enforcer, 0.01, 0);
    let after = enforcer.status().expect("cpu edge after");
    assert_eq!(result_code, CALYX_ANNEAL_BUDGET_EXHAUSTED);
    edge_json("cpu_exhausted", before, result_code, after)
}

fn edge_vram_exhausted(clock: &FixedClock) -> serde_json::Value {
    let enforcer = BudgetEnforcer::with_probe(
        BudgetConfig::default(),
        clock,
        ScriptedProbe::new(0.05, 513 * MIB, false),
    )
    .expect("vram edge enforcer");
    let before = enforcer.tick().expect("vram edge before");
    let result_code = acquire_code(&enforcer, 0.0, 0);
    let after = enforcer.status().expect("vram edge after");
    assert_eq!(result_code, CALYX_ANNEAL_BUDGET_EXHAUSTED);
    edge_json("vram_exhausted", before, result_code, after)
}

fn edge_zero_config(clock: &FixedClock) -> serde_json::Value {
    let enforcer = BudgetEnforcer::with_probe(
        BudgetConfig {
            cpu_fraction: 0.0,
            vram_bytes: 512 * MIB,
            tick_interval_ms: 100,
        },
        clock,
        ScriptedProbe::new(0.0, 0, false),
    )
    .expect("zero edge enforcer");
    let before = enforcer.tick().expect("zero edge before");
    let result_code = acquire_code(&enforcer, 0.0, 1);
    let after = enforcer.status().expect("zero edge after");
    assert_eq!(result_code, CALYX_ANNEAL_BUDGET_EXHAUSTED);
    edge_json("zero_cpu_fraction", before, result_code, after)
}

fn edge_drop_after_enforcer_drop(clock: &FixedClock) -> serde_json::Value {
    let enforcer = BudgetEnforcer::with_probe(
        BudgetConfig::default(),
        clock,
        ScriptedProbe::new(0.02, 0, false),
    )
    .expect("shutdown edge enforcer");
    let before = enforcer.tick().expect("shutdown edge before");
    let handle = enforcer.acquire(0.01, MIB).expect("shutdown edge acquire");
    let active = enforcer.status().expect("shutdown edge active");
    drop(enforcer);
    drop(handle);
    json!({
        "case": "drop_after_enforcer_drop",
        "expected": "handle Drop completes after enforcer shutdown without panic",
        "before": before,
        "during": active,
        "result_code": "ok",
        "after": {"enforcer_dropped": true, "handle_drop_completed": true}
    })
}

fn edge_json(
    case: &str,
    before: BudgetStatus,
    result_code: String,
    after: BudgetStatus,
) -> serde_json::Value {
    json!({
        "case": case,
        "expected": CALYX_ANNEAL_BUDGET_EXHAUSTED,
        "before": before,
        "result_code": result_code,
        "after": after
    })
}

fn acquire_code<P>(enforcer: &BudgetEnforcer<'_, P>, cpu_weight: f64, vram_bytes: u64) -> String
where
    P: BudgetProbe,
{
    match enforcer.acquire(cpu_weight, vram_bytes) {
        Ok(handle) => {
            drop(handle);
            "ok".to_string()
        }
        Err(error) => error.code.to_string(),
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
}

impl BudgetProbe for ScriptedProbe {
    fn sample(&self) -> BudgetProbeSample {
        self.sample.lock().unwrap().clone()
    }
}
