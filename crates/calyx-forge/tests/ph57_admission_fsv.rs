//! PH57 T03 manual FSV for Forge VRAM admission control.
//!
//! SoT: `VramStats` admission counters and the Prometheus text bytes returned
//! by `VramStats::admission_metrics_text()`. This test writes those bytes under
//! `CALYX_FSV_ROOT`, re-reads them from disk, and prints before/after state for
//! a deterministic happy path plus edge cases.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use calyx_forge::{
    AdmissionController, AdmitDecision, BlockDeallocator, ForgeError, GpuBlockRegistry,
    VramBudgeter, VramProbe,
};
use serde_json::json;

const MIB: usize = 1024 * 1024;
const GIB: usize = 1024 * 1024 * 1024;
const CODE: &str = "CALYX_FORGE_VRAM_BUDGET";

struct StaticProbe {
    free: usize,
}

impl VramProbe for StaticProbe {
    fn free_device_vram(&self) -> Result<usize, ForgeError> {
        Ok(self.free)
    }
}

#[derive(Clone, Default)]
struct NoopDealloc;

impl BlockDeallocator for NoopDealloc {
    fn free(&self, _ptr: calyx_forge::DevicePtr, _size: usize) -> Result<(), ForgeError> {
        Ok(())
    }
}

fn controller<'b>(
    budgeter: &'b VramBudgeter<StaticProbe>,
    queue_cap: usize,
    split_min_batch: usize,
) -> AdmissionController<'b, StaticProbe, NoopDealloc> {
    let registry = GpuBlockRegistry::new(budgeter, NoopDealloc, 16);
    AdmissionController::new(
        budgeter,
        Arc::new(Mutex::new(registry)),
        queue_cap,
        split_min_batch,
    )
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/ph57-admission-fsv"))
}

#[test]
fn ph57_admission_fsv_writes_counter_and_metric_readbacks() {
    let root = fsv_root();
    fs::create_dir_all(&root).expect("create fsv root");
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let ctl = controller(&budgeter, 1, 2);
    let deadline = Instant::now() + Duration::from_secs(1);
    let past = Instant::now() - Duration::from_secs(1);

    let before = budgeter.stats();
    println!("BEFORE stats: {before:?}");

    let happy = ctl.decide(512 * MIB, 8, deadline);
    let split_output = ctl
        .run_with_admission(4 * GIB, 8, deadline, |offset, len| {
            Ok((offset..offset + len).collect::<Vec<_>>())
        })
        .expect("split run");
    let queued = ctl.decide(2 * GIB, 2, deadline);
    let fail = ctl.decide(2 * GIB, 2, deadline);
    let past_deadline = ctl.decide(512 * MIB, 8, past);
    let zero = ctl.decide(0, 8, past);
    let sync_err = ctl
        .run_with_admission(
            2 * GIB,
            2,
            deadline,
            |_offset, _len| Ok(Vec::<usize>::new()),
        )
        .expect_err("full queue fails closed");

    let after = budgeter.stats();
    println!("AFTER stats: {after:?}");
    println!("split_output={split_output:?}");
    println!("sync_error_code={}", sync_err.code());

    assert_eq!(happy, AdmitDecision::Split { sub_batch_size: 8 });
    assert_eq!(split_output, (0..8).collect::<Vec<_>>());
    assert!(matches!(queued, AdmitDecision::Queue { .. }));
    assert_eq!(fail, AdmitDecision::Fail);
    assert_eq!(past_deadline, AdmitDecision::Fail);
    assert_eq!(zero, AdmitDecision::Split { sub_batch_size: 8 });
    assert_eq!(sync_err.code(), CODE);
    assert_eq!(ctl.queue_len(), 1);
    assert!(after.splits_total >= 9);
    assert_eq!(after.queued_total, 1);
    assert!(after.failed_total >= 3);

    let metrics = after.admission_metrics_text();
    assert!(metrics.contains("calyx_forge_vram_budget_exceeded_total"));
    assert!(metrics.contains(&format!(
        "calyx_forge_vram_budget_exceeded_total {}",
        after.failed_total
    )));

    let readback = json!({
        "source_of_truth": "VramStats admission counters + Prometheus text bytes",
        "before": before,
        "after": after,
        "known_input": {"bytes": 4 * GIB, "batch": 8, "expected": [0,1,2,3,4,5,6,7]},
        "actual_split_output": split_output,
        "decisions": {
            "happy": format!("{happy:?}"),
            "queued": format!("{queued:?}"),
            "fail": format!("{fail:?}"),
            "past_deadline": format!("{past_deadline:?}"),
            "zero": format!("{zero:?}"),
            "sync_error_code": sync_err.code()
        },
        "queue_len": ctl.queue_len(),
        "metrics": metrics,
    });
    let readback_path = root.join("ph57-admission-readback.json");
    let metrics_path = root.join("ph57-admission.prom");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    fs::write(&metrics_path, metrics.as_bytes()).unwrap();

    let readback_bytes = fs::read(&readback_path).expect("read readback bytes");
    let metrics_bytes = fs::read_to_string(&metrics_path).expect("read metrics bytes");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&readback_bytes).unwrap()["after"]["failed_total"],
        json!(after.failed_total)
    );
    assert!(metrics_bytes.contains(&format!(
        "calyx_forge_vram_budget_exceeded_total {}",
        after.failed_total
    )));
    println!("PH57_ADMISSION_FSV_ROOT={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
}
