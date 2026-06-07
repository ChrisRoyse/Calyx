# PH45 · T05 — Regression re-assert (replay + no-recurrence check)

| Field | Value |
|---|---|
| **Phase** | PH45 — Mistake-Closure + Online Heads + Replay Buffer |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/learn/regression_assert.rs` (≤500) |
| **Depends on** | T01 (MistakeLog), T02 (ReplayBuffer), T03 (OnlineHeadState::predict) |
| **Axioms** | A4, A14 |
| **PRD** | `dbprdplans/12 §3` |

## Goal

Implement `RegressionAssert`: after `OnlineHeadState` is updated, replay each
mistake from the batch through the updated predictor head and assert the mistake
does not recur. "Does not recur" means the updated prediction is on the correct
side of the decision boundary (|new_prediction − observed| < |old_prediction −
observed|). If regression is detected (a mistake recurs after update), the
update must be flagged and potentially reverted. This is the "wrong only once"
invariant made concrete and machine-checkable.

## Build (checklist of concrete, code-level steps)

- [ ] `struct RegressionResult { cx_id: CxId, old_surprise: f64, new_surprise: f64, recurred: bool }` — `recurred = new_surprise >= old_surprise`.
- [ ] `fn assert_no_regression(heads: &OnlineHeadState, batch: &[ReplayEntry], log: &MistakeLog) -> RegressionReport` — for each entry in `batch`, calls `heads.predict(cx)` to get `new_prediction`; retrieves `observed` from `log`; computes `new_surprise`; sets `recurred` if `new_surprise >= old_surprise`; returns `RegressionReport { results, regression_count, passed: regression_count == 0 }`.
- [ ] `fn regression_rate(report: &RegressionReport) -> f64` — `regression_count / batch.len()`.
- [ ] Integration with `OnlineHeadState::update`: after a promoted update, `assert_no_regression` is run; if `regression_rate > max_regression_rate` (configurable, default `0.05`), the update is reverted via `AnnealSubstrate::rollback_explicit`; Ledger gets `action=HeadUpdateReverted` with `regression_rate`.
- [ ] `fn record_regression(result: &RegressionResult, log: &mut MistakeLog)` — a recurred mistake is re-appended to `MistakeLog` with increased `surprise` (to ensure it re-enters the `ReplayBuffer` at higher priority next cycle).
- [ ] Clock-injected; no `SystemTime::now()`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: old_surprise=`0.8`, new_prediction correctly crosses boundary (new_surprise=`0.1`) → `recurred=false`.
- [ ] unit: old_surprise=`0.8`, new_prediction still wrong (new_surprise=`0.9`) → `recurred=true`; `regression_rate` = 1.0 for a single-entry batch.
- [ ] proptest: if all new_surprises < all old_surprises, `regression_rate = 0.0`.
- [ ] edge: empty batch → `regression_count=0`, `passed=true`; all entries recur → `regression_rate=1.0`; `max_regression_rate=0.0` (strict mode) → any regression triggers revert.
- [ ] fail-closed: `heads.predict` returns `NaN` → treat as `new_surprise = f64::MAX` (worst case; always counts as regression); error logged.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `RegressionReport` and Ledger entries after a head-update cycle.
- **Readback:** `calyx anneal regression-report --last 1` (or read Ledger `HeadUpdate` / `HeadUpdateReverted` entries).
- **Prove:** (combined with T06 FSV) feed a mistake, update heads, run `assert_no_regression`; confirm `passed=true` (no recurrence); the Ledger `HeadUpdate` entry has `regression_rate=0.0`. Separately test a deliberate no-op head (all-zero gradient): regression fires; `HeadUpdateReverted` appears in Ledger.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH45 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
