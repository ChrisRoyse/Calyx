# PH45 · T06 — Integration FSV: contradiction → update → no recurrence, frozen unchanged

| Field | Value |
|---|---|
| **Phase** | PH45 — Mistake-Closure + Online Heads + Replay Buffer |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/tests/fsv_mistake_closure.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05 |
| **Axioms** | A4, A14, A15 |
| **PRD** | `dbprdplans/12 §3`, `dbprdplans/03 §6` |

## Goal

Prove the full mistake-closure loop end-to-end: (1) feed a contradicting outcome
into the system; (2) confirm `MistakeLog` and `ReplayBuffer` are updated; (3)
run the "sleep pass" (background head update); (4) confirm the same mistake does
not recur on replay; (5) confirm frozen lens hashes are unchanged. This is the
PH45 FSV gate as a deterministic runnable test.

## Build (checklist of concrete, code-level steps)

- [ ] Test scenario `mistake_closure_loop`: (a) create synthetic vault with 1 frozen lens `L1` and 1 constellation `CX1`; (b) `FrozenLensGuard::initialize()`; record `L1_hash_before`; (c) call `MistakeLog::append(cx_id=CX1, predicted=0.9, observed=0.1)`; assert `surprise=0.8`; (d) assert `ReplayBuffer` contains `CX1` at top priority; (e) run head update: `OnlineHeadState::update(batch=[CX1])` — assert `ChangeOutcome::Promoted`; (f) run `RegressionAssert::assert_no_regression` → assert `passed=true` (old_surprise=0.8, new_surprise<0.8); (g) `FrozenLensGuard::check()` → assert `violations=[]`; (h) `L1_hash_after == L1_hash_before` (byte comparison); (i) Ledger has `HeadUpdate` entry with `regression_rate=0.0`.
- [ ] Test scenario `no_frozen_mutation_under_load`: apply 100 mistake-closure cycles with varied (predicted, observed) pairs; after all cycles, `FrozenLensGuard::check()` still returns `violations=[]`.
- [ ] Both scenarios fully deterministic: seeded RNG (`seed=0xCAFE`), injected clock, synthetic data, no live TEI call.
- [ ] Expose `fn run_sleep_pass(heads: &mut OnlineHeadState, buffer: &ReplayBuffer, log: &MistakeLog, assert: &RegressionAssert) -> SleepPassOutcome` — the compositing function that drives the full background loop; used in both test scenarios and in production Anneal.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] `mistake_closure_loop`: all 9 assertions (a–i) must pass.
- [ ] `no_frozen_mutation_under_load`: after 100 cycles, `violations=[]`.
- [ ] edge: contradicting outcome where `|predicted − observed| = 0.0` (zero surprise) → appended to log, NOT added to `ReplayBuffer` (surprise below min threshold `0.01`); heads unchanged.
- [ ] edge: `DegradeRegistry` shows a component `Degraded` → `run_sleep_pass` defers the update until components are `Ok` (writes a Ledger `SleepPassDeferred` entry instead of updating).
- [ ] fail-closed: head update reverted by substrate → `SleepPassOutcome::Reverted`; `MistakeLog` NOT modified; mistake stays in `ReplayBuffer` for next cycle.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_heads` CF (version + params), Ledger `HeadUpdate` entry, `FrozenLensGuard` report.
- **Readback:** `calyx readback ledger --kind Anneal --action HeadUpdate --last 1` (shows `regression_rate`); `calyx anneal frozen-guard-report` (shows all stable); `calyx anneal head-status --kind Predictor` (shows version incremented).
- **Prove:** run `cargo test fsv_mistake_closure` on aiwonder; all assertions green; `readback ledger` shows `HeadUpdate` with `regression_rate=0.0`; `frozen-guard-report` shows all lenses `stable=true`; `head-status` shows `version=1` after one cycle. Attach stdout to the PH45 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH45 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
