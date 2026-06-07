# PH57 · T05 — Anneal yield + 600 W cap enforcement

| Field | Value |
|---|---|
| **Phase** | PH57 — VRAM budgeter + admission control |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/vram/yield_policy.rs` (≤500) |
| **Depends on** | T04 (OOM guard + admission), T01 (budgeter) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §2`, `12 §6` |

## Goal

Implement the VRAM/SM yield policy: Anneal background math (autotuning, kernel rebuild,
lens proposal) runs at a lower CUDA stream priority than serving (search/embed) and TEI
containers; a separate Anneal VRAM sub-budget cap (`CALYX_ANNEAL_VRAM_BUDGET`) limits
background allocations so they cannot crowd out serving. Also implement a 600 W soft cap:
query `nvidia-smi dmon` power draw; if sustained > 560 W, back off Anneal SM occupancy by
throttling dispatch frequency. Defends hazard 20 (Anneal thrash/oscillation) and system
power stability.

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct YieldPolicy { anneal_vram_cap_bytes: usize, serving_stream_priority: i32, anneal_stream_priority: i32, power_backoff_threshold_w: u32 }` in `yield_policy.rs`
- [ ] Implement `YieldPolicy::from_env() -> Self` — reads `CALYX_ANNEAL_VRAM_BUDGET` (default 2 GiB); stream priorities: serving = 0 (highest), Anneal = -1 (lower); `power_backoff_threshold_w` default 560
- [ ] Implement `YieldPolicy::anneal_budget_check(&self, budgeter: &VramBudgeter) -> Result<(), CalyxError>` — checks `budgeter.allocated_bytes_for(Category::Anneal) <= anneal_vram_cap_bytes`; returns `CALYX_FORGE_VRAM_BUDGET` if exceeded; Anneal callers must check before allocating
- [ ] Implement `YieldPolicy::query_power_draw_w() -> Result<u32, CalyxError>` — reads `/sys/bus/pci/drivers/nvidia/*/power_state` or calls `nvmlDeviceGetPowerUsage` via NVML FFI; returns watts; on error returns `CALYX_GPU_ERROR` (non-fatal to caller)
- [ ] Implement `YieldPolicy::should_throttle_anneal(&self) -> bool` — calls `query_power_draw_w()`; returns true if power > `power_backoff_threshold_w`; callers insert a `std::thread::sleep(Duration::from_millis(50))` before next Anneal dispatch
- [ ] Implement `YieldPolicy::create_anneal_stream() -> Result<CudaStream, CalyxError>` — creates a CUDA stream with `anneal_stream_priority` (lower priority than serving streams); serving streams created with `serving_stream_priority`
- [ ] Add `YieldStats { anneal_throttle_events: u64, anneal_vram_rejections: u64 }` to `VramStats`
- [ ] Add category field to `VramGuard` so `budgeter.allocated_bytes_for(Category)` can split accounting between `Serving` and `Anneal`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `from_env()` with `CALYX_ANNEAL_VRAM_BUDGET=2147483648` (2 GiB) → `anneal_vram_cap_bytes == 2147483648`
- [ ] unit: mock `query_power_draw_w()` returning 580 W → `should_throttle_anneal()` returns true; 550 W → false; exactly 560 W → false (threshold is strict >)
- [ ] unit: reserve 2 GiB for `Anneal` category; `anneal_budget_check` succeeds; reserve 1 more byte → `CALYX_FORGE_VRAM_BUDGET`; serving reservation of 100 MiB still succeeds (separate budget)
- [ ] unit: CUDA stream priorities — serving stream priority > Anneal stream priority (`cudaStreamGetPriority` check on aiwonder); aiwonder sm_120 supports priorities
- [ ] unit: `query_power_draw_w()` on aiwonder returns a plausible value (> 0, < 700 W) when GPU is idle
- [ ] edge: NVML not available (driver error) → `query_power_draw_w()` returns `CALYX_GPU_ERROR`; `should_throttle_anneal()` returns false (unknown power → don't throttle, just log)
- [ ] edge: `anneal_vram_cap_bytes == 0` → every Anneal alloc returns `CALYX_FORGE_VRAM_BUDGET`
- [ ] fail-closed: Anneal budget exhausted; Anneal dispatch returns `CALYX_FORGE_VRAM_BUDGET`; serving dispatch with same bytes succeeds (budgets are independent)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `YieldStats::anneal_throttle_events` and `nvidia-smi dmon -s p -d 1` (1-second power samples) on aiwonder during concurrent TEI + Forge + Anneal load
- **Readback:** `calyx readback --metric forge_anneal_throttle_events_total`; `nvidia-smi dmon -s p -d 10 -c 60` (60 samples) → power column; `calyx readback --metric forge_anneal_vram_rejections_total`
- **Prove:** under combined load (3 TEI + serving + Anneal), power stays ≤ 600 W in the nvidia-smi dmon output (or Anneal throttle events increment when it approaches threshold); `anneal_vram_rejections_total > 0` if Anneal cap is set conservatively. Attach power chart as FSV evidence.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH57 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
