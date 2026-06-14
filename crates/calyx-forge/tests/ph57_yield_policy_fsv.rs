use std::fs;
use std::path::PathBuf;
#[cfg(feature = "cuda")]
use std::sync::Mutex;

use calyx_forge::{
    ForgeError, PowerProbe, Result, VramBudgeter, VramProbe, VramStats, YieldPolicy,
};
use serde::Serialize;

const GIB: usize = 1024 * 1024 * 1024;
const MIB: usize = 1024 * 1024;
const BUDGET_CODE: &str = "CALYX_FORGE_VRAM_BUDGET";
const GPU_CODE: &str = "CALYX_GPU_ERROR";

struct StaticProbe;

impl VramProbe for StaticProbe {
    fn free_device_vram(&self) -> Result<usize> {
        Ok(64 * GIB)
    }
}

struct Power(u32);

impl PowerProbe for Power {
    fn power_draw_w(&self) -> Result<u32> {
        Ok(self.0)
    }
}

struct FailingPower;

impl PowerProbe for FailingPower {
    fn power_draw_w(&self) -> Result<u32> {
        Err(ForgeError::GpuError {
            detail: "simulated NVML unavailable".into(),
            remediation: "test".into(),
        })
    }
}

#[derive(Serialize)]
struct YieldReadback {
    source_of_truth: &'static str,
    happy_expected_anneal_bytes: usize,
    happy_before: VramStats,
    happy_at_exact_cap: VramStats,
    over_cap_error_code: String,
    after_over_cap: VramStats,
    serving_after_anneal_cap_full: VramStats,
    throttle_580w: bool,
    throttle_560w: bool,
    throttle_550w: bool,
    after_throttle: VramStats,
    power_failure_error_code: String,
    power_failure_should_throttle: bool,
    zero_cap_error_code: String,
    zero_cap_after_anneal_reject: VramStats,
    zero_cap_after_serving_ok: VramStats,
    after_drop: VramStats,
}

#[cfg(feature = "cuda")]
#[derive(Serialize)]
struct CudaReadback {
    power_w: u32,
    priority_range_least: i32,
    priority_range_greatest: i32,
    serving_raw_priority: i32,
    anneal_raw_priority: i32,
    priority_order_proved: bool,
    streams_created: bool,
}

#[test]
fn ph57_yield_policy_writes_readback_artifacts() -> Result<()> {
    let policy = YieldPolicy {
        anneal_vram_cap_bytes: 2 * GIB,
        ..YieldPolicy::default()
    };
    let budgeter = VramBudgeter::with_soft_cap(4 * GIB, StaticProbe);
    let happy_before = budgeter.stats();

    let anneal = policy.reserve_anneal(&budgeter, 2 * GIB)?;
    let happy_at_exact_cap = budgeter.stats();
    policy.anneal_budget_check(&budgeter)?;

    let over_cap_error = match policy.reserve_anneal(&budgeter, 1) {
        Ok(_) => panic!("Anneal +1 byte must reject"),
        Err(err) => err,
    };
    let after_over_cap = budgeter.stats();

    let serving = budgeter.reserve(100 * MIB)?;
    let serving_after_anneal_cap_full = budgeter.stats();

    let throttle_560w = policy.should_throttle_with(&Power(560));
    let throttle_550w = policy.should_throttle_with(&Power(550));
    let throttle_580w = policy.throttle_anneal_if_needed_with(&budgeter, &Power(580), None);
    let after_throttle = budgeter.stats();

    let power_failure = FailingPower.power_draw_w().expect_err("synthetic failure");
    let power_failure_should_throttle = policy.should_throttle_with(&FailingPower);

    drop(serving);
    drop(anneal);
    let after_drop = budgeter.stats();

    let zero_policy = YieldPolicy {
        anneal_vram_cap_bytes: 0,
        ..YieldPolicy::default()
    };
    let zero_budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe);
    let zero_cap_error = match zero_policy.reserve_anneal(&zero_budgeter, 1) {
        Ok(_) => panic!("zero Anneal cap must reject nonzero"),
        Err(err) => err,
    };
    let zero_cap_after_anneal_reject = zero_budgeter.stats();
    let zero_serving = zero_budgeter.reserve(100 * MIB)?;
    let zero_cap_after_serving_ok = zero_budgeter.stats();
    drop(zero_serving);

    assert_eq!(happy_before.anneal_allocated_bytes, 0);
    assert_eq!(happy_at_exact_cap.anneal_allocated_bytes, 2 * GIB);
    assert_eq!(over_cap_error.code(), BUDGET_CODE);
    assert_eq!(after_over_cap.anneal_allocated_bytes, 2 * GIB);
    assert_eq!(
        serving_after_anneal_cap_full.serving_allocated_bytes,
        100 * MIB
    );
    assert!(throttle_580w);
    assert!(!throttle_560w);
    assert!(!throttle_550w);
    assert_eq!(power_failure.code(), GPU_CODE);
    assert!(!power_failure_should_throttle);
    assert_eq!(zero_cap_error.code(), BUDGET_CODE);
    assert_eq!(zero_cap_after_anneal_reject.anneal_allocated_bytes, 0);
    assert_eq!(zero_cap_after_serving_ok.serving_allocated_bytes, 100 * MIB);
    assert_eq!(after_drop.allocated_bytes, 0);
    assert_eq!(after_throttle.yield_stats.anneal_throttle_events, 1);
    assert_eq!(after_throttle.yield_stats.anneal_vram_rejections, 1);

    let readback = YieldReadback {
        source_of_truth: "VramStats category bytes + YieldStats counters + Prometheus text",
        happy_expected_anneal_bytes: 2 * GIB,
        happy_before,
        happy_at_exact_cap,
        over_cap_error_code: over_cap_error.code().into(),
        after_over_cap,
        serving_after_anneal_cap_full,
        throttle_580w,
        throttle_560w,
        throttle_550w,
        after_throttle,
        power_failure_error_code: power_failure.code().into(),
        power_failure_should_throttle,
        zero_cap_error_code: zero_cap_error.code().into(),
        zero_cap_after_anneal_reject,
        zero_cap_after_serving_ok,
        after_drop,
    };

    let root = fsv_root();
    fs::create_dir_all(&root).map_err(io_error)?;
    let json_path = root.join("ph57-yield-policy-readback.json");
    let prom_path = root.join("ph57-yield-policy.prom");
    let json = serde_json::to_string_pretty(&readback).map_err(|err| ForgeError::CacheError {
        op: "serialize yield policy readback".into(),
        path: json_path.display().to_string(),
        detail: err.to_string(),
        remediation: "fix FSV serialization".into(),
    })?;
    let metrics = budgeter.stats().admission_metrics_text();
    assert!(metrics.contains("forge_anneal_throttle_events_total 1"));
    assert!(metrics.contains("forge_anneal_vram_rejections_total 1"));
    fs::write(&json_path, json).map_err(io_error)?;
    fs::write(&prom_path, metrics).map_err(io_error)?;

    println!("PH57_YIELD_POLICY_JSON {}", json_path.display());
    println!("PH57_YIELD_POLICY_PROM {}", prom_path.display());
    Ok(())
}

#[cfg(feature = "cuda")]
static CUDA_LOCK: Mutex<()> = Mutex::new(());

#[cfg(feature = "cuda")]
#[test]
fn ph57_yield_policy_cuda_power_and_stream_readback() -> Result<()> {
    let _lock = CUDA_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let policy = YieldPolicy::default();
    let ctx = calyx_forge::init_cuda(0, false)?;
    let (least, greatest) = YieldPolicy::stream_priority_range_for_context(&ctx)?;
    let serving_raw = policy.resolved_serving_cuda_priority(&ctx)?;
    let anneal_raw = policy.resolved_anneal_cuda_priority(&ctx)?;
    let _serving_stream = policy.create_serving_stream_for_context(&ctx)?;
    let _anneal_stream = policy.create_anneal_stream_for_context(&ctx)?;
    let power_w = YieldPolicy::query_power_draw_w()?;

    let priority_order_proved = least == greatest || serving_raw < anneal_raw;
    assert!(priority_order_proved);
    assert!(power_w > 0);
    assert!(power_w < 700);

    let readback = CudaReadback {
        power_w,
        priority_range_least: least,
        priority_range_greatest: greatest,
        serving_raw_priority: serving_raw,
        anneal_raw_priority: anneal_raw,
        priority_order_proved,
        streams_created: true,
    };

    let root = fsv_root();
    fs::create_dir_all(&root).map_err(io_error)?;
    let path = root.join("ph57-yield-policy-cuda-readback.json");
    let json = serde_json::to_string_pretty(&readback).map_err(|err| ForgeError::CacheError {
        op: "serialize yield cuda readback".into(),
        path: path.display().to_string(),
        detail: err.to_string(),
        remediation: "fix FSV serialization".into(),
    })?;
    fs::write(&path, json).map_err(io_error)?;

    println!(
        "PH57_YIELD_POLICY_CUDA {} power_w={} priority_range=({}, {}) serving_raw={} anneal_raw={}",
        path.display(),
        power_w,
        least,
        greatest,
        serving_raw,
        anneal_raw
    );
    Ok(())
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-ph57-yield-policy-fsv"))
}

fn io_error(err: std::io::Error) -> ForgeError {
    ForgeError::CacheError {
        op: "yield policy FSV file IO".into(),
        path: fsv_root().display().to_string(),
        detail: err.to_string(),
        remediation: "ensure CALYX_FSV_ROOT exists and is writable".into(),
    }
}
