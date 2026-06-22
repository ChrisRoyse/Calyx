use super::*;
use std::sync::Mutex;

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

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn from_env_reads_exact_two_gib_cap() {
    let _lock = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var(ANNEAL_VRAM_BUDGET_ENV, "2147483648") };
    let policy = YieldPolicy::from_env();
    unsafe { std::env::remove_var(ANNEAL_VRAM_BUDGET_ENV) };

    assert_eq!(policy.anneal_vram_cap_bytes, 2 * GIB);
    assert_eq!(policy.serving_stream_priority, 0);
    assert_eq!(policy.anneal_stream_priority, -1);
    assert_eq!(policy.power_backoff_threshold_w, 560);
}

#[test]
fn power_threshold_is_strict_greater_than() {
    let policy = YieldPolicy::default();

    assert!(policy.should_throttle_with(&Power(580)));
    assert!(!policy.should_throttle_with(&Power(550)));
    assert!(!policy.should_throttle_with(&Power(560)));
}

#[test]
fn power_query_failure_is_non_fatal_to_throttle_decision() {
    let policy = YieldPolicy::default();
    let err = FailingPower.power_draw_w().expect_err("mock failure");

    assert_eq!(err.code(), GPU_CODE);
    assert!(!policy.should_throttle_with(&FailingPower));
}

#[test]
fn reserve_anneal_cap_is_separate_from_serving() {
    let budgeter = VramBudgeter::with_soft_cap(4 * GIB, StaticProbe);
    let policy = YieldPolicy {
        anneal_vram_cap_bytes: 2 * GIB,
        ..YieldPolicy::default()
    };

    let anneal = policy
        .reserve_anneal(&budgeter, 2 * GIB)
        .expect("exact Anneal cap fits");
    assert_eq!(budgeter.allocated_bytes_for(Category::Anneal), 2 * GIB);
    assert!(policy.anneal_budget_check(&budgeter).is_ok());

    let err = match policy.reserve_anneal(&budgeter, 1) {
        Ok(_) => panic!("Anneal +1 byte must reject"),
        Err(err) => err,
    };
    assert_eq!(err.code(), BUDGET_CODE);
    assert_eq!(budgeter.stats().yield_stats.anneal_vram_rejections, 1);

    let serving = budgeter
        .reserve(100 * MIB)
        .expect("serving remains independent of Anneal cap");
    assert_eq!(serving.category(), Category::Serving);
    assert_eq!(budgeter.allocated_bytes_for(Category::Serving), 100 * MIB);

    drop(serving);
    drop(anneal);
    assert_eq!(budgeter.allocated_bytes(), 0);
}

#[test]
fn zero_anneal_cap_rejects_nonzero_but_serving_succeeds() {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe);
    let policy = YieldPolicy {
        anneal_vram_cap_bytes: 0,
        ..YieldPolicy::default()
    };

    let err = match policy.reserve_anneal(&budgeter, 1) {
        Ok(_) => panic!("zero Anneal cap must reject nonzero"),
        Err(err) => err,
    };
    assert_eq!(err.code(), BUDGET_CODE);
    assert_eq!(budgeter.allocated_bytes_for(Category::Anneal), 0);

    let serving = budgeter.reserve(100 * MIB).expect("serving fits");
    assert_eq!(budgeter.allocated_bytes_for(Category::Serving), 100 * MIB);
    drop(serving);
}

#[test]
fn anneal_budget_check_rejects_existing_overage() {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe);
    let policy = YieldPolicy {
        anneal_vram_cap_bytes: MIB,
        ..YieldPolicy::default()
    };
    let _guard = budgeter
        .reserve_category(2 * MIB, Category::Anneal)
        .expect("raw category reservation for overage test");

    let err = policy
        .anneal_budget_check(&budgeter)
        .expect_err("existing Anneal overage must reject");

    assert_eq!(err.code(), BUDGET_CODE);
    assert_eq!(budgeter.stats().yield_stats.anneal_vram_rejections, 1);
}

#[test]
fn throttle_records_metric_without_sleep_when_requested() {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe);
    let policy = YieldPolicy::default();

    let throttled = policy.throttle_anneal_if_needed_with(&budgeter, &Power(580), None);

    assert!(throttled);
    assert_eq!(budgeter.stats().yield_stats.anneal_throttle_events, 1);
    assert!(
        budgeter
            .stats()
            .admission_metrics_text()
            .contains("forge_anneal_throttle_events_total 1")
    );
}

#[test]
fn nvidia_smi_power_parser_rounds_up_decimal_watts() -> Result<()> {
    let watts = parse_nvidia_smi_power_stdout(b"15.56\n")?;

    assert_eq!(watts, 16);
    Ok(())
}

#[test]
fn nvidia_smi_power_parser_fails_closed_on_invalid_output() {
    let err = parse_nvidia_smi_power_stdout(b"N/A\n").expect_err("invalid power output");

    assert_eq!(err.code(), GPU_CODE);
}

#[cfg(feature = "cuda")]
#[test]
fn cuda_stream_priorities_put_serving_above_anneal() -> Result<()> {
    let _lock = crate::cuda::test_lock();
    let policy = YieldPolicy::default();
    let ctx = crate::cuda::init_cuda(0, false)?;
    let (least, greatest) = YieldPolicy::stream_priority_range_for_context(&ctx)?;
    let serving_raw = policy.resolved_serving_cuda_priority(&ctx)?;
    let anneal_raw = policy.resolved_anneal_cuda_priority(&ctx)?;
    let _serving = policy.create_serving_stream_for_context(&ctx)?;
    let _anneal = policy.create_anneal_stream_for_context(&ctx)?;

    println!(
        "CUDA_PRIORITY_RANGE least={least} greatest={greatest} serving_raw={serving_raw} anneal_raw={anneal_raw}"
    );

    if least != greatest {
        assert!(serving_raw < anneal_raw);
    }
    Ok(())
}

#[cfg(feature = "cuda")]
#[test]
fn power_query_returns_plausible_watts_on_manual() -> Result<()> {
    let power_w = YieldPolicy::query_power_draw_w()?;

    println!("GPU_POWER_W {power_w}");

    assert!(power_w > 0);
    assert!(power_w < 700);
    Ok(())
}
