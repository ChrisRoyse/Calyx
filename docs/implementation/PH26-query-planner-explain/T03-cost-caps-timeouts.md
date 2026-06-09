# PH26 · T03 — Cost caps + timeout enforcement (reject unbounded plans)

| Field | Value |
|---|---|
| **Phase** | PH26 — Query planner + intent + explain |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/planner.rs` (≤500) |
| **Depends on** | T02 (this phase) |
| **Axioms** | A16, A17 |
| **PRD** | `dbprdplans/10 §7`, `dbprdplans/10 §8`, `dbprdplans/17 §7.3` |

## Goal

The planner must reject queries whose estimated cost exceeds the configured cap,
and enforce per-query execution timeouts. An unbounded plan (e.g. `k=u32::MAX`,
`ef=u32::MAX`, or `num_slots > MAX_SLOTS`) must fail with
`CALYX_SEXTANT_PLAN_UNBOUNDED` before any index is touched. This is the
"bounded plans" requirement from `17 §7.3`.

## Build (checklist of concrete, code-level steps)

- [x] `PlannerConfig` struct:
  ```rust
  pub struct PlannerConfig {
      pub max_k: usize,              // default 1000
      pub max_ef: usize,             // default 2000
      pub max_slots: usize,          // default 16
      pub max_estimated_ms: f32,     // default 120.0 (reject plans > 120ms estimate)
      pub query_timeout_ms: u64,     // default 5000 (5s hard wall)
      pub pipeline_timeout_ms: u64,  // default 60000 (60s for Pipeline with rerank)
  }
  ```
  with a `Default` impl using the values above
- [x] `fn check_bounds(query: &Query, config: &PlannerConfig) -> Result<(), CalyxError>`:
      - `query.k > max_k` → `CALYX_SEXTANT_PLAN_UNBOUNDED` with `remediation: "reduce k"`
      - `ef > max_ef` (computed from query or default) → `CALYX_SEXTANT_PLAN_UNBOUNDED`
      - participating slot count > `max_slots` → `CALYX_SEXTANT_PLAN_UNBOUNDED`
      - all checks run before any index operation; fail-closed at first violation
- [x] `fn check_cost(cost: &CostEstimate, config: &PlannerConfig) -> Result<(), CalyxError>`:
      - `cost.estimated_ms > config.max_estimated_ms` → `CALYX_SEXTANT_PLAN_COST_EXCEEDED`
        with `remediation: "reduce k, ef, or num_slots"`
- [x] Wire both checks into `plan()` (from T02) before returning `PlannerOutput`
- [x] `PlannerOutput` gains a `timeout_budget_ms: u64` field (the applicable
      timeout from config, passed down to the executor)
- [x] `CALYX_SEXTANT_PLAN_UNBOUNDED` and `CALYX_SEXTANT_PLAN_COST_EXCEEDED`
      added to the error catalog in `calyx-core`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `k=1001` with `max_k=1000` → `CALYX_SEXTANT_PLAN_UNBOUNDED`
- [x] unit: `k=10, ef=2001` with `max_ef=2000` → `CALYX_SEXTANT_PLAN_UNBOUNDED`
- [x] unit: 17 slots with `max_slots=16` → `CALYX_SEXTANT_PLAN_UNBOUNDED`
- [x] unit: `estimated_ms=130.0` with `max_estimated_ms=120.0` →
      `CALYX_SEXTANT_PLAN_COST_EXCEEDED`
- [x] unit: valid query (k=10, ef=100, 2 slots, estimated 9ms) → `Ok(PlannerOutput)`
      with `timeout_budget_ms=5000`
- [x] proptest: `check_bounds` never panics for any `Query` input
- [x] edge: `k=0` → `CALYX_SEXTANT_PLAN_UNBOUNDED` (zero-k is meaningless, fail-closed)
- [x] fail-closed: remediation string is non-empty in both error variants (A16)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant cost_caps -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant cost_caps -- --nocapture 2>&1`
- **Prove:** test prints:
  `k_too_large=CALYX_SEXTANT_PLAN_UNBOUNDED ef_too_large=CALYX_SEXTANT_PLAN_UNBOUNDED cost_exceeded=CALYX_SEXTANT_PLAN_COST_EXCEEDED valid_ok=true`

## Post-sweep hardening

- [x] #282: `k=0` and `ef=0` fail closed with
      `CALYX_SEXTANT_PLAN_UNBOUNDED`.
- [x] #282: cost caps now fail with `CALYX_SEXTANT_PLAN_COST_EXCEEDED`, distinct
      from structural unbounded-plan errors.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH26 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
