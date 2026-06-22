//! PH57 · T01 — CUDA-path FSV for the VRAM budgeter (GPU truth gate).
//!
//! This proves the *live* `cudaMemGetInfo` reading on a real device and the
//! end-to-end budgeter wired to it. It is gated on the `cuda` feature and
//! marked `#[ignore]` so it only runs on a GPU host (manual, CUDA GPU):
//!
//! ```text
//! cargo test -p calyx-forge --features cuda --test ph57_vram_cuda_fsv \
//!     -- --ignored --nocapture
//! ```
//!
//! SoT: `CudaVramProbe::free_device_vram()` (== `cudaMemGetInfo`) cross-checked
//! against `nvidia-smi --query-gpu=memory.free`, and the budgeter's
//! `allocated_bytes` / `VramStats` after a real reservation.
#![cfg(feature = "cuda")]

use std::sync::Arc;

use calyx_forge::{CudaVramProbe, VramBudgeter, VramProbe, init_cuda};

const GIB: usize = 1024 * 1024 * 1024;
/// 32 GiB device + generous tolerance for driver-reported totals.
const VRAM_UPPER_BOUND: usize = 34_359_738_368;

#[test]
#[ignore = "requires a CUDA GPU (run on a CUDA host with --features cuda --ignored)"]
fn fsv_live_free_vram_query() {
    let ctx = init_cuda(0, false).expect("init_cuda on device 0");
    let probe = CudaVramProbe::new(Arc::new(ctx));

    let free = probe.free_device_vram().expect("live cudaMemGetInfo query");
    println!(
        "[CUDA-FSV] free_device_vram() = {free} bytes ({} MiB)",
        free / (1024 * 1024)
    );

    // The issue's exact bound: must be > 0 on an idle/loaded GPU and within
    // the physical device size (+ tolerance). Never the 0 of a silent failure.
    assert!(free > 0, "free VRAM must be > 0 on a live GPU");
    assert!(
        free <= VRAM_UPPER_BOUND,
        "free VRAM {free} exceeds physical bound {VRAM_UPPER_BOUND}"
    );
}

#[test]
#[ignore = "requires a CUDA GPU (run on a CUDA host with --features cuda --ignored)"]
fn fsv_budgeter_reserve_on_real_device() {
    let ctx = init_cuda(0, false).expect("init_cuda on device 0");
    let probe = CudaVramProbe::new(Arc::new(ctx));

    // 4 GiB soft cap, well under what the CUDA GPU has free alongside TEI.
    let budgeter = VramBudgeter::with_soft_cap(4 * GIB, probe);

    let before = budgeter.stats();
    println!(
        "[CUDA-FSV] BEFORE: allocated={} device_free={}",
        before.allocated_bytes, before.device_free_bytes
    );
    assert_eq!(before.allocated_bytes, 0);
    assert!(
        before.device_free_bytes > 0,
        "device_free must be live, not 0"
    );

    // Trigger X: reserve 1 GiB. Outcome Y: allocated rises by exactly 1 GiB.
    let guard = budgeter.reserve(GIB).expect("reserve 1 GiB on real device");
    let during = budgeter.stats();
    println!(
        "[CUDA-FSV] DURING: allocated={} device_free={}",
        during.allocated_bytes, during.device_free_bytes
    );
    assert_eq!(during.allocated_bytes, GIB, "accounting must reflect 1 GiB");

    drop(guard);
    let after = budgeter.stats();
    println!(
        "[CUDA-FSV] AFTER:  allocated={} device_free={}",
        after.allocated_bytes, after.device_free_bytes
    );
    assert_eq!(
        after.allocated_bytes, 0,
        "guard drop must release accounting"
    );
}
