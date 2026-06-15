# PH16 — Autotune Config Cache

**Stage:** S2 — Forge Math Runtime  ·  **Crate:** `calyx-forge`  ·
**PRD roadmap:** `12 §4`  ·  **Axioms:** A14

## Objective

Implement the per-shape best-config cache keyed `(op, shape, dtype, device,
recall_tgt)` → `BestConfig`, refreshed by a low-rate **ε-greedy / Thompson
explorer**. The cache is persisted to disk (JSON), promotions are recorded in a
local append-only JSONL audit stub and reversible. This is the seam that Anneal
(PH43–PH48) later drives for end-to-end autotune loops — PH16 delivers the
microbench infrastructure and the cache read/write/explore API that Anneal will
call. The `autotune(op, shape, dtype, device) -> BestConfig` function is the
Forge-facing surface.

## Dependencies

- **Phases:** PH15 (MXFP4 + grouped GEMM must be DONE — autotune covers all Forge
  ops including grouped GEMM configs and fp4 vs bf16 choices)
- **Provides for:** PH43 (Anneal tripwires call `autotune` to pick the current best
  config before triggering), PH46 (autotune loops drive `autotune` at scale), PH57
  (VRAM budgeter checks `BestConfig.vram_estimate`)

## Current state (build off what exists)

`calyx-forge` has the full CPU SIMD (PH12), CUDA sm_120 (PH13), TurboQuant (PH14),
MXFP4 + grouped GEMM (PH15), and the PH16 autotune cache/microbench/explorer
surfaces in-tree. Promotions are logged to the reversible PH16 JSONL stub file;
after PH35 this remains a local Forge audit artifact because `calyx-forge` does
not own the cross-engine Ledger append path. Real Ledger-backed promotion
provenance belongs to later Anneal/provenance integration.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/autotune.rs` | `AutotuneCache`: key type, cache CRUD, persist/load, `autotune()` API |
| `src/autotune/microbench.rs` | `microbench(op, config, shape, ctx)` → `BenchResult`; wall-clock timing + GFLOP/s |
| `src/autotune/explorer.rs` | ε-greedy and Thompson-sampling explorer; A/B-on-live hook |
| `tests/autotune_tests.rs` | Cache convergence + promotion logging + reversibility tests |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `AutotuneCache` key type + CRUD + persist/load | PH12 T01 (BestConfig) |
| T02 | `microbench` harness (wall-clock, GFLOP/s) | PH13 T03 |
| T03 | ε-greedy / Thompson explorer + promotion gate | T01, T02 |
| T04 | A/B-on-live hook + promotion audit stub + reversibility | T03 |
| T05 | FSV: two shapes converge to two cached configs | T01, T02, T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run on aiwonder:

```bash
source $CALYX_HOME/repo/env.sh
cargo test -p calyx-forge --features cuda autotune -- --nocapture 2>&1 | tee /tmp/ph16_fsv.txt

grep -E "converged|promotion|reversed|two_shapes|PASSED|FAILED" /tmp/ph16_fsv.txt

# Read the cache file to confirm two distinct entries:
cat $CALYX_HOME/repo/crates/calyx-forge/tests/autotune_cache_fsv.json | python3 -m json.tool | head -30
```

Proof: `autotune_two_shapes_converge` PASSED — each of the two shapes has a distinct
`BestConfig` in the cache (different tile sizes or different backend variants);
`autotune_promotion_logged` PASSED — the promotion event log file contains a
timestamped entry for each promotion; `autotune_promotion_reversible` PASSED —
the `rollback_promotion(key)` API restores the previous config and the log entry
shows the rollback.

## Risks / landmines

- **Explorer convergence speed:** ε-greedy with ε=0.1 needs O(1/ε²) trials per
  shape to reliably identify the winner; on aiwonder with fast GPU, 20 trials
  per shape is feasible in < 5 seconds. Set `MAX_EXPLORE_ITERS=20` as a constant.
- **Persist race condition:** cache writes use a write-then-rename pattern (write to
  `.tmp` then rename atomically) to avoid torn writes; document this in `persist()`.
- **Promotion audit stub:** PH16 writes to a plain `promotion_log.jsonl` append
  file, not the real Ledger chain. This remains acceptable after PH35 because
  Forge is a math-runtime crate with no direct storage/Ledger dependency; real
  Ledger-backed promotion provenance is future Anneal/provenance wiring.
- **Clock injection:** the microbench must use `std::time::Instant` (wall clock for
  GPU timing after synchronization) — this is acceptable since microbench is not
  determinism-critical; however, the `Clock` trait from `calyx-core` must be used
  for any timestamp written to the promotion log (not `SystemTime::now()` in logic).
