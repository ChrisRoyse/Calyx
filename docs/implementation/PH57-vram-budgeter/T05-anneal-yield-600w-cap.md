# PH57 - T05 - Anneal yield + 600 W cap enforcement

| Field | Value |
|---|---|
| **Phase** | PH57 - VRAM budgeter + admission control |
| **Stage** | S13 - Resource, GC & Reliability Hardening |
| **Crate** | `calyx-forge` |
| **Primary files** | `crates/calyx-forge/src/vram/yield_policy.rs`, `budget.rs`, `mod.rs` |
| **Depends on** | T04 (OOM guard + admission), T01 (budgeter) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §2`, `12 §6` |

## Goal

Implement the VRAM/SM yield policy: Anneal background math (autotuning, kernel rebuild,
lens proposal) runs at a lower CUDA stream priority than serving (search/embed) and TEI
containers; a separate Anneal VRAM sub-budget cap (`CALYX_ANNEAL_VRAM_BUDGET`) limits
background allocations so they cannot crowd out serving. Also implement a 600 W soft cap:
query GPU power draw; if sustained above 560 W, back off Anneal dispatch by sleeping before
the next Anneal dispatch. This defends hazard 20 (Anneal thrash/oscillation) and system
power stability.

## Implementation

- [x] `YieldPolicy { anneal_vram_cap_bytes, serving_stream_priority, anneal_stream_priority, power_backoff_threshold_w }`
- [x] `YieldPolicy::from_env()` reads `CALYX_ANNEAL_VRAM_BUDGET`, defaults to 2 GiB, and fails closed to cap `0` on invalid input.
- [x] `YieldPolicy::anneal_budget_check()` checks `allocated_bytes_for(Category::Anneal) <= anneal_vram_cap_bytes` and returns `CALYX_FORGE_VRAM_BUDGET` on overage.
- [x] `YieldPolicy::reserve_anneal()` enforces the Anneal sub-budget atomically before admitting the allocation.
- [x] `YieldPolicy::query_power_draw_w()` reads NVML first and falls back to `nvidia-smi --query-gpu=power.draw`; all-source failure returns `CALYX_GPU_ERROR`.
- [x] `YieldPolicy::should_throttle_anneal()` is strict `power > threshold`; unknown power logs and returns false.
- [x] `YieldPolicy::throttle_anneal_if_needed()` records `anneal_throttle_events` and sleeps 50 ms when throttling.
- [x] CUDA stream helpers create serving and Anneal streams with serving higher priority than Anneal.
- [x] `YieldStats { anneal_throttle_events, anneal_vram_rejections }` is included in `VramStats` and Prometheus text.
- [x] `VramGuard` tracks `Category::{Serving, Anneal}` so `allocated_bytes_for(Category)` splits accounting.

## CUDA priority note

The card's logical defaults are preserved: serving priority `0`, Anneal priority `-1`.
CUDA's raw priority polarity is opposite the wording in the issue: lower numeric raw values
mean higher runtime priority. The implementation maps logical serving to the highest raw CUDA
priority and Anneal to the lowest raw CUDA priority. aiwonder FSV readback proved
`priority_range=(0, -5)`, `serving_raw_priority=-5`, `anneal_raw_priority=0`, and
`priority_order_proved=true`.

## Tests

- [x] `from_env()` with `CALYX_ANNEAL_VRAM_BUDGET=2147483648` yields 2 GiB.
- [x] Power threshold logic: 580 W throttles; 560 W and 550 W do not.
- [x] Anneal exact-cap reservation succeeds; +1 byte returns `CALYX_FORGE_VRAM_BUDGET`.
- [x] Serving 100 MiB reservation succeeds while Anneal is full because budgets are split.
- [x] CUDA stream priority test creates both streams and proves serving outranks Anneal.
- [x] aiwonder real power query returns plausible watts.
- [x] Unknown power returns/logs `CALYX_GPU_ERROR` at the probe boundary and throttle decision returns false.
- [x] Zero Anneal cap rejects nonzero Anneal allocation while serving still succeeds.
- [x] `nvidia-smi` decimal power parsing rounds up and invalid output fails closed.

## FSV

Evidence root:

```text
/home/croyse/calyx/data/fsv-issue479-yield-policy-20260614T211902Z
```

Source-of-truth files read directly on aiwonder:

- `ph57-yield-policy-readback.json`
- `ph57-yield-policy-cuda-readback.json`
- `ph57-yield-policy.prom`
- `nvidia-smi-dmon-power.txt`
- `cuda-load-loop.log`
- `sha256sum.txt`

Manual readback values:

- Happy path: `happy_before.anneal_allocated_bytes=0`; exact cap `anneal_allocated_bytes=2147483648`.
- Edge max+1: over-cap error code `CALYX_FORGE_VRAM_BUDGET`; Anneal bytes stayed `2147483648`.
- Serving independence: `serving_allocated_bytes=104857600` while Anneal remained at 2 GiB.
- Throttle boundary: `throttle_580w=true`, `throttle_560w=false`, `throttle_550w=false`.
- Counters: `anneal_throttle_events=1`, `anneal_vram_rejections=1`.
- Prometheus bytes: `forge_anneal_throttle_events_total 1`, `forge_anneal_vram_rejections_total 1`.
- Unknown power edge: `power_failure_error_code=CALYX_GPU_ERROR`, `power_failure_should_throttle=false`.
- Zero cap edge: `zero_cap_error_code=CALYX_FORGE_VRAM_BUDGET`, Anneal bytes stayed `0`, serving reached `104857600`.
- CUDA readback: `power_w=22`, raw priorities `serving=-5`, `anneal=0`, streams created.
- Power samples: 60 `nvidia-smi dmon -s p -d 1` samples during 249 Calyx CUDA `perf_vs_cublas` passes; max observed power `76 W`, below the 600 W cap.

## Done

- [x] `cargo fmt --all -- --check` green on aiwonder.
- [x] `cargo check -p calyx-forge` green on aiwonder.
- [x] `cargo clippy -p calyx-forge --all-targets -- -D warnings` green on aiwonder.
- [x] `cargo test -p calyx-forge -- --nocapture` green on aiwonder.
- [x] `cargo check -p calyx-forge --features cuda` green on aiwonder.
- [x] `cargo clippy -p calyx-forge --features cuda --all-targets -- -D warnings` green on aiwonder.
- [x] `cargo test -p calyx-forge --features cuda --test ph57_yield_policy_fsv -- --nocapture` green on aiwonder.
- [x] `cargo test -p calyx-forge --features cuda --test cuda_parity -- --nocapture` green on aiwonder; golden parity max relative error stayed below `1e-3`.
- [x] `.rs` line-count gate: `RS_OVER_500_COUNT 0`.
- [x] FSV evidence captured under the root above and attached in issue/PR closeout.
