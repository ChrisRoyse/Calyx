# PH43 · T02 — Shadow executor (held-out replay + beat-incumbent check)

| Field | Value |
|---|---|
| **Phase** | PH43 — Tripwires + Shadow-First + Reversible/Rollback |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/shadow.rs` (≤500) |
| **Depends on** | T01 (TripwireRegistry — shadow uses it to gate promotion) |
| **Axioms** | A14, A15 |
| **PRD** | `dbprdplans/12 §6`, `dbprdplans/27 §4` |

## Goal

Implement `ShadowExecutor`: given a `candidate` (new config/artifact) and an
`incumbent` (current live artifact), run both against a seeded, deterministic
held-out query replay set, compare their metric vectors through
`TripwireRegistry::check`, and return `ShadowVerdict::Promote` only if the
candidate beats the incumbent on every tripwire metric with no regression.
Any failure returns `ShadowVerdict::Revert { reason }` — the candidate never
touches the live path.

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct HeldOutReplay { queries: Vec<ReplayQuery>, seed: u64 }` — seeded at construction, order deterministic; `ReplayQuery` carries the query vector + expected top-k anchor IDs + anchor similarity scores.
- [ ] `fn build_replay(vault, n: usize, seed: u64) -> HeldOutReplay` — samples `n` queries from stored anchors; never from live-traffic order.
- [ ] `struct ShadowExecutor { registry: TripwireRegistry, replay: HeldOutReplay, budget: BudgetHandle }` — takes a `BudgetHandle` from T04 to stay within background CPU/VRAM ceiling.
- [ ] `fn run_shadow<A: AnnealAction>(&mut self, candidate: &A, incumbent: &A) -> ShadowVerdict` — runs both against `self.replay`; collects metric pairs; calls `registry.check` for each; returns `Promote` only if candidate metrics all pass and each is ≥ incumbent (no regression on any).
- [ ] `enum ShadowVerdict { Promote { metrics: MetricSnapshot }, Revert { reason: ShadowRevertReason, metrics: MetricSnapshot } }` — `MetricSnapshot` captures `(metric, candidate_value, incumbent_value)` for every tripwire metric.
- [ ] Trait `AnnealAction: Send + Sync` with `fn apply_shadow(&self, query: &ReplayQuery) -> MetricSnapshot`.
- [ ] Shadow run must complete within `budget` ticks; if budget exhausted before replay finishes → `Revert { reason: BudgetExhausted }`.
- [ ] Clock-injected: `shadow.rs` never calls `SystemTime::now()`; receives `&dyn Clock`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: incumbent = perfect recall `1.0`; candidate = recall `0.85` → `Revert { reason: TripwireCrossed(RecallAtK) }`; verify `MetricSnapshot` carries both values.
- [ ] unit: candidate beats incumbent on all metrics → `Promote`; `MetricSnapshot` values match manually computed expectations.
- [ ] proptest: for any two metric snapshots where candidate dominates incumbent on every metric, `run_shadow` returns `Promote`.
- [ ] edge: empty replay set → `Revert { reason: InsufficientReplay }`; single-query replay → runs without panic; candidate == incumbent on all metrics → `Promote` (no regression, passes).
- [ ] fail-closed: `BudgetHandle` set to 0 ticks → `Revert { reason: BudgetExhausted }` immediately without running any queries.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ShadowVerdict` returned from `run_shadow`, plus the `MetricSnapshot` logged to Ledger (T05).
- **Readback:** `calyx anneal shadow-log --last 5` (or read the Anneal Ledger CF) — prints the last 5 shadow verdicts with candidate vs incumbent metrics.
- **Prove:** craft a candidate with recall `0.80` vs incumbent `0.95`; run shadow; confirm `Revert` entry in Ledger with `RecallAtK` as the failing metric; confirm the live config pointer is unchanged (incumbent still active).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH43 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
