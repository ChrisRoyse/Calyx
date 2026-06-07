# PH46 · T05 — A/B runner on live traffic

| Field | Value |
|---|---|
| **Phase** | PH46 — Autotune Loops |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/tune/ab_runner.rs` (≤500) |
| **Depends on** | T01 (ConfigBandit), T02, T03, T04 (all scope tuners call this) |
| **Axioms** | A14, A15 |
| **PRD** | `dbprdplans/12 §4` |

## Goal

Implement `ABRunner`: the shared execution harness that all scope tuners use for
live-traffic A/B comparisons. The incumbent config serves the real request on
the hot path; the candidate runs the same request in shadow (background budget,
parallel goroutine) and emits a `ABResult` with latency and quality metrics. The
runner accumulates results across `min_samples` queries before declaring a winner
and feeds into the bandit's `record_result`. Every promotion and revert is
Ledger-logged.

## Build (checklist of concrete, code-level steps)

- [ ] `struct ABResult { arm_idx: usize, latency_ns: u64, recall_k: f64, bits_per_anchor: f64, ts: LogicalTime }`.
- [ ] `struct ABTrial { key: ShapeKey, incumbent_idx: usize, candidate_idx: usize, results: Vec<ABResult>, min_samples: usize }` — `min_samples` default `100`.
- [ ] `struct ABRunner { substrate: Arc<AnnealSubstrate>, budget: Arc<BudgetEnforcer>, active_trials: HashMap<ShapeKey, ABTrial>, clock: Arc<dyn Clock> }`.
- [ ] `fn start_trial(&mut self, key: ShapeKey, candidate_arm: usize, incumbent_arm: usize) -> Result<(), CalyxError>` — creates an `ABTrial`; errors if a trial is already active for this key.
- [ ] `fn record_query(&mut self, key: ShapeKey, incumbent_result: ABResult, candidate_result: ABResult)` — appends both results to the trial; if `trial.results.len() >= min_samples`: calls `declare_winner`.
- [ ] `fn declare_winner(trial: &ABTrial, bandit: &mut ConfigBandit, substrate: &AnnealSubstrate) -> ABVerdict` — computes mean latency and recall for each arm; calls `TripwireRegistry::check` for recall/p99; if candidate wins (lower p99 AND no tripwire crossed): calls `substrate.propose_change` for the promotion; writes Ledger entry; returns `ABVerdict::Promoted` or `ABVerdict::Kept`.
- [ ] Candidate shadow run must NOT affect the serving-path response: candidate result is captured asynchronously; the caller receives the incumbent's result.
- [ ] Shadow cancellation: if budget is exhausted mid-trial, the trial is abandoned with `ABVerdict::Abandoned`; Ledger records `action=AutotuneAbandoned`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: fill `min_samples=10` results where candidate consistently beats incumbent on latency + recall; `declare_winner` returns `ABVerdict::Promoted`.
- [ ] unit: candidate beats on latency but recall crosses tripwire → `ABVerdict::Kept`; incumbent unchanged.
- [ ] unit: `record_query` after `declare_winner` → trial complete, no double-promotion (idempotent).
- [ ] proptest: for any `ABResult` sequence, `declare_winner` returns `Promoted` iff mean candidate latency < mean incumbent latency AND no tripwire crossed.
- [ ] edge: `min_samples=1` → verdict after first query pair; start two trials for the same key → `CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE`; budget exhausted on first shadow query → `ABVerdict::Abandoned`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Ledger A/B entries + `AutotuneCache` CF.
- **Readback:** `calyx anneal ab-log --last 5` (reads Ledger `AutotunePromote` + `AutotuneAbandoned` entries).
- **Prove:** run a synthetic A/B trial (100 samples, candidate 20% faster, same recall); `ab-log` shows `Promoted` entry with `latency_before_ns`, `latency_after_ns`, `latency_after_ns < 0.80 × latency_before_ns`; incumbent config updated in `AutotuneCache`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH46 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
