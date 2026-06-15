# PH43 - T04 - Background budget enforcer (CPU/VRAM yield)

| Field | Value |
|---|---|
| Phase | PH43 - Tripwires + Shadow-First + Reversible/Rollback |
| Stage | S10 - Anneal + Intelligence Objective J |
| Crate | `calyx-anneal` |
| Files | `crates/calyx-anneal/src/budget.rs` (<=500) |
| Depends on | None inside PH43; future Anneal workers acquire handles before compute |
| Axioms | A14, A26 |
| PRD | `dbprdplans/12 section 6`, `dbprdplans/27 section 4` |

## Goal

Implement `BudgetEnforcer` so Anneal background work cannot starve the serving
path or resident TEI services on aiwonder (:8088/:8089/:8090). Each background
task acquires a `BudgetHandle` before compute. `acquire` is deliberately
non-blocking: if the requested CPU or VRAM would exceed configured headroom, it
returns `CALYX_ANNEAL_BUDGET_EXHAUSTED` immediately and the caller schedules a
retry.

## Implementation

- [x] `BudgetConfig { cpu_fraction, vram_bytes, tick_interval_ms }` defaults to
  `0.15`, `512MiB`, and `100`; `BudgetConfig::load_from_vault` persists and
  loads `<vault>/.anneal/budget.toml`.
- [x] `BudgetEnforcer` tracks CPU from `/proc/stat` on Linux and VRAM through a
  conservative static-pool counter when NVML is unavailable.
- [x] `BudgetEnforcer::acquire(cpu_weight, vram_bytes)` returns a RAII
  `BudgetHandle`; dropping the handle releases reserved CPU/VRAM back to the
  pool.
- [x] `acquire` returns `CALYX_ANNEAL_BUDGET_EXHAUSTED` if either CPU or VRAM
  headroom is below the request. It never blocks the serving path.
- [x] `tick` uses an injected `&dyn Clock`; the module never calls
  `SystemTime::now()`.
- [x] `status()` returns `BudgetStatus { cpu_used_fraction, vram_used_bytes,
  handles_active, last_tick_at, low_priority_nice, warning_code }`.
- [x] Low-priority background intent is exposed as `BACKGROUND_NICE = 10` and
  `BudgetStatus.low_priority_nice`. The future scheduler must use that value
  when it launches long-running Anneal worker threads/processes.
- [x] First `/proc/stat` sample is conservative while the rolling sample is
  primed, so unknown CPU load fails closed rather than admitting work as zero.

## Tests

- [x] Unit: `cpu_fraction=0.10`, sampled CPU `0.05`, request `0.04` succeeds;
  sampled CPU `0.12` returns `CALYX_ANNEAL_BUDGET_EXHAUSTED`.
- [x] Unit: acquiring and dropping a `BudgetHandle` replenishes the pool; the
  next acquire succeeds.
- [x] Proptest: across acquire/drop sequences, `status().vram_used_bytes` never
  exceeds `BudgetConfig::vram_bytes`.
- [x] Edge: zero CPU capacity and zero VRAM capacity fail closed.
- [x] Edge: dropping a handle after the enforcer is dropped does not panic.
- [x] Fail-closed: NVML unavailable sets
  `CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE`, uses the static pool, and does not
  panic.
- [x] CLI readback: `calyx readback config budget --vault <dir>` prints the
  byte-backed budget config hash and parsed values.

## FSV

Source of truth:

- Vault config bytes: `<vault>/.anneal/budget.toml`
- In-process budget state: `BudgetStatus` while a synthetic background task
  holds a real RAII handle
- FSV root: `/home/croyse/calyx/data/fsv-issue397-budget-<timestamp>`

Readback paths:

- `calyx readback config budget --vault <vault>`
- `budget-status-sequence.json`
- `budget-edge-readback.json`
- `background-priority.txt`
- `BLAKE3SUMS.txt`

Required proof:

1. Load budget config from the vault and read back the persisted TOML bytes.
2. Tick the enforcer with CPU `0.05` and VRAM `128MiB`.
3. Acquire a background task handle with CPU weight `0.04` and VRAM `128MiB`.
4. Read `BudgetStatus` while the handle is active and prove CPU `0.09 <= 0.15`,
   VRAM `256MiB <= 512MiB`, and `handles_active == 1`.
5. Drop the handle and read `BudgetStatus` again to prove `handles_active == 0`.
6. Exercise at least three edges with before/after status readback: CPU
   exhausted, VRAM exhausted, zero config, and handle drop after enforcer drop.

When PH43 T06 wires a daemon-level Anneal scheduler, add a live endpoint or
`calyx anneal status` command for runtime status. T04's current SoT is the
enforcer state plus persisted vault config bytes.

Evidence root:

`/home/croyse/calyx/data/fsv-issue397-budget-20260610-2341`

## Done when

- [x] `cargo check`, `cargo clippy -D warnings`, and focused tests pass on
  aiwonder.
- [x] All touched `.rs` files are <=500 lines.
- [x] aiwonder FSV reads the budget config bytes, running `BudgetStatus`, edge
  status artifacts, and BLAKE3 manifest.
- [x] Evidence is attached to GitHub issue #397 before close.
