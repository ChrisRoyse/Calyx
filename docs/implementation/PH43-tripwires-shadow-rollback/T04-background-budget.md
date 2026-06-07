# PH43 · T04 — Background budget enforcer (CPU/VRAM yield)

| Field | Value |
|---|---|
| **Phase** | PH43 — Tripwires + Shadow-First + Reversible/Rollback |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/budget.rs` (≤500) |
| **Depends on** | — (no intra-phase dep; used by T02, T04 wires into all future Anneal tasks) |
| **Axioms** | A14, A26 |
| **PRD** | `dbprdplans/12 §6`, `dbprdplans/27 §4` |

## Goal

Implement `BudgetEnforcer` that caps Anneal background work to a configured
fraction of CPU and VRAM so it never starves the serving path or the resident
TEI services on aiwonder (:8088/:8089/:8090). The enforcer provides
`BudgetHandle` tokens — each background task acquires a handle before doing
compute; the handle blocks/yields if the budget is exhausted; tasks that exceed
their budget slot are preempted cooperatively.

## Build (checklist of concrete, code-level steps)

- [ ] `struct BudgetConfig { cpu_fraction: f64, vram_bytes: u64, tick_interval_ms: u64 }` — default `cpu_fraction=0.15`, `vram_bytes=512MiB`, `tick_interval_ms=100`; loaded from vault config.
- [ ] `struct BudgetEnforcer` tracks rolling CPU usage (via `/proc/stat` on Linux or equivalent) and VRAM (query from CUDA device via `nvml` or pre-allocated pool counter); exposes `fn acquire(cpu_weight: f64, vram_bytes: u64) -> Result<BudgetHandle, CalyxError>`.
- [ ] `BudgetHandle` is a RAII guard: `Drop` releases budget back to the pool; carries the tick-count limit.
- [ ] `fn acquire` returns `CALYX_ANNEAL_BUDGET_EXHAUSTED` immediately (non-blocking) if both CPU and VRAM headroom are below request; the caller schedules a retry rather than blocking the serving path.
- [ ] `fn tick(&mut self)` — called by the background scheduler every `tick_interval_ms`; updates rolling usage and replenishes the token pool; clock-injected (`&dyn Clock`), never `SystemTime::now()`.
- [ ] Expose `fn status() -> BudgetStatus { cpu_used_fraction, vram_used_bytes, handles_active }` for observability.
- [ ] Background tasks run at `nice +10` (Linux) or equivalent low-priority to yield to serving.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: configure `cpu_fraction=0.10`; simulate usage at `0.05` → `acquire` succeeds; simulate usage at `0.12` → `CALYX_ANNEAL_BUDGET_EXHAUSTED`.
- [ ] unit: acquire then drop `BudgetHandle` → pool replenished; next acquire succeeds.
- [ ] proptest: for any sequence of `(acquire, drop)` operations, the total `vram_used_bytes` reported by `status()` never exceeds `BudgetConfig::vram_bytes`.
- [ ] edge: zero `cpu_fraction` config → every `acquire` returns `CALYX_ANNEAL_BUDGET_EXHAUSTED`; `vram_bytes=0` → same; `BudgetHandle` drop after enforcer is dropped → no panic (graceful shutdown).
- [ ] fail-closed: NVML unavailable → fall back to a conservative static pool; log `CALYX_ANNEAL_BUDGET_NVML_UNAVAILABLE`; never panic.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `BudgetStatus` reported while Anneal background task is running.
- **Readback:** `calyx anneal status` (or read the in-process metrics endpoint) — prints `cpu_used_fraction`, `vram_used_bytes`, `handles_active`.
- **Prove:** start a background Anneal task (shadow run); while it is running, `status()` reports `cpu_used_fraction ≤ 0.15` and `vram_used_bytes ≤ 512MiB`; serving-path p99 does not regress during the background run (read search p99 metric series).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH43 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
