# PH57 — VRAM budgeter + admission control

**Stage:** S13 — Resource, GC & Reliability Hardening  ·  **Crate:** `calyx-forge`  ·
**PRD roadmap:** RESOURCE  ·  **Axioms:** A26

## Objective

`calyx-forge` coexists with the 3 resident TEI containers on the single RTX 5090 (32 GB VRAM).
This phase installs a VRAM budgeter with a soft configurable cap (`CALYX_FORGE_VRAM_BUDGET`),
a pre-dispatch free-VRAM query, LRU eviction of cached GPU-resident blocks, admission control
(split large batches / queue medium ones / fail large ones closed), an OOM guard (reduce batch
then retry then fail closed), and the discipline that VRAM holds only the current batch + ANN
frontier — never the corpus. Anneal background work yields to serving and TEI; the 600 W TDP
cap is honored. Cross-cutting hardening from Stage 2, finalized here. Single NVMe no
redundancy — but this phase is GPU-focused; disk interactions route through PH56/PH58.

## Dependencies

- **Phases:** PH13 (CUDA sm_120 backend + bit-parity — the VRAM allocation primitives being
  bounded here)
- **Provides for:** PH58 (GPU staging slab pools freed by GC), PH59 (hazards 7, 20 FSV)

## Current state (build off what exists)

`calyx-forge` has a CUDA sm_120 backend from PH13 with raw `cudaMalloc`/`cudaFree` calls and
no budget enforcement. Autotune config cache exists (PH16) but is unbounded in VRAM footprint.
TEI containers run resident on :8088/:8089/:8090; no Forge VRAM coordination with them exists.
Greenfield for the budgeter, admission control, and yield logic.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-forge/src/vram/budget.rs` | VRAM budgeter: soft cap config, free-VRAM query, usage accounting |
| `crates/calyx-forge/src/vram/lru_evict.rs` | LRU eviction of GPU-resident blocks under pressure |
| `crates/calyx-forge/src/vram/admission.rs` | Admission control: split/queue/fail logic; `CALYX_FORGE_VRAM_BUDGET` |
| `crates/calyx-forge/src/vram/oom_guard.rs` | OOM guard: reduce-batch → retry → fail closed; CUDA OOM intercept |
| `crates/calyx-forge/src/vram/yield_policy.rs` | Anneal sub-budget, power backoff, and CUDA stream priority yield policy |
| `crates/calyx-forge/src/vram/mod.rs` | Re-exports + `VramStats` |
| `crates/calyx-registry/src/ingest_microbatch.rs` | Bounded ingest microbatch admission, timeout breaker, `CALYX_BACKPRESSURE` |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | VRAM budgeter — soft cap config, free-VRAM query, usage accounting | — |
| T02 | LRU eviction of GPU-resident blocks | T01 |
| T03 | Admission control — split/queue/fail, `CALYX_FORGE_VRAM_BUDGET` | T01, T02 |
| T04 | OOM guard — reduce-batch → retry → fail closed | T03 |
| T05 | Anneal yield + 600 W cap enforcement | T04 |
| T06 | Concurrent TEI FSV soak — dispatch over budget → split/queue/fail, p99 holds | T01, T02, T03, T04, T05 |
| T07 | Bounded ingest microbatch admission — cap bytes, backpressure, per-lens breaker | PRD 24 §6 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Under concurrent TEI load on aiwonder (all three TEI containers at :8088/:8089/:8090 running):

```
nvidia-smi --query-gpu=memory.used,memory.free --format=csv,noheader,nounits
calyx readback --metric forge_vram_budget_exceeded_total
calyx readback --metric forge_dispatch_p99_ms
```

- A dispatch over budget → split/queue/`CALYX_FORGE_VRAM_BUDGET` (no silent OOM); verified by
  reading `forge_vram_budget_exceeded_total > 0`
- Search p99 SLO holds (read `nvidia-smi` + latency series; no OOM kill in `dmesg`)
- Evidence (nvidia-smi screenshot + latency series + `forge_vram_budget_exceeded_total`) attached
  to the PH57 GitHub issue

## Current implementation evidence

- T05 Anneal yield policy is implemented in `crates/calyx-forge/src/vram/yield_policy.rs`.
- `VramStats` now includes split serving/Anneal byte gauges and `YieldStats`.
- aiwonder FSV root: `/home/croyse/calyx/data/fsv-issue479-yield-policy-20260614T211902Z`.
- T05 readback proved `forge_anneal_throttle_events_total 1`,
  `forge_anneal_vram_rejections_total 1`, 60 `nvidia-smi dmon` samples with max
  power `76 W`, and CUDA priority ordering `serving_raw=-5` over `anneal_raw=0`.
- T06 concurrent TEI soak is implemented in `crates/calyx-forge/tests/soak_ph57.rs`
  with Criterion coverage in `crates/calyx-forge/benches/bench_admission_overhead.rs`.
- aiwonder FSV root: `/home/croyse/calyx/data/fsv-issue480-ph57-soak-20260614T214449Z`.
- T06 readback proved TEI :8088/:8089/:8090 health `200`, baseline p99
  `25.539 ms`, loaded p99 `19.943 ms`, one-TEI edge success `12/12`,
  `calyx_forge_vram_budget_exceeded_total 38`, splits `75`, queued `4`,
  max VRAM `7628 MiB`, max power `192 W`, admission overhead `88.415 ns`,
  no OOM matches in `dmesg`/`journalctl`, and line gate max `.rs` length `500`.

## Risks / landmines

- **nvidia-smi free VRAM is stale:** query `cudaMemGetInfo` inside the process, not `nvidia-smi`,
  for accurate free VRAM before each dispatch
- **TEI containers hide their allocation:** Forge cannot know TEI's VRAM use ahead of time;
  budget `CALYX_FORGE_VRAM_BUDGET` must be set conservatively (e.g., 12 GB of the 32 GB) by
  the operator, leaving 20 GB headroom for 3 TEI containers
- **Split-then-OOM race:** another process may claim VRAM between the free-VRAM query and the
  dispatch; the OOM guard (T04) handles this with reduce-batch + retry
- **600 W cap:** RTX 5090 TDP is 575 W; sustained high-compute + Anneal background can exceed
  system budget; yield logic must cap Anneal SM occupancy (use CUDA stream priorities)
- **CUDA driver OOM vs graceful error:** `cudaMalloc` returns `cudaErrorMemoryAllocation`, not a
  panic; the OOM guard must intercept this return code specifically
