# PH52 · T04 — Bayesian posteriors: Gamma-Poisson + Beta-Bernoulli

| Field | Value |
|---|---|
| **Phase** | PH52 — Advanced math |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/bayesian.rs` (≤500) |
| **Depends on** | PH42 (grounded recurrence streams — the data to maintain posteriors over), PH49 T02 (oracle self-consistency — Beta-Bernoulli for pass-rate uncertainty) |
| **Axioms** | A16, A2, A20 |
| **PRD** | `dbprdplans/26 §6` |

## Goal

Implement conjugate Bayesian posteriors for the two quantities where Calyx currently uses
raw counts (`26 §6`):
1. **Gamma-Poisson rate posterior** for recurrence event rate — credible interval on rate +
   next-occurrence prediction, graceful at `n = 2, 3`.
2. **Beta-Bernoulli consistency posterior** for oracle self-consistency (pass-rate /
   flakiness) — uncertainty on flakiness/validity ("is this oracle *reliably* consistent
   or just so-far?").

Both are **online** (one update per occurrence), cheap, and exactly the small-sample regime
where Calyx operates. Every recurrence/consistency number carries honest uncertainty (A16).

## Build (checklist of concrete, code-level steps)

- [ ] `struct GammaPoisson { alpha: f64, beta: f64 }` — conjugate prior/posterior for Poisson rate λ; prior `Gamma(α, β)` → posterior after observing `k` events in interval `t`: `Gamma(α + k, β + t)`
- [ ] `impl GammaPoisson { fn new(prior_alpha: f64, prior_beta: f64) -> Self; fn update(&mut self, events: u64, interval: f64); fn mean_rate(&self) -> f64; fn credible_interval_95(&self) -> (f64, f64); fn next_occurrence_expected(&self) -> f64 }` — all values in consistent time units
- [ ] Default prior: `alpha = 1.0, beta = 1.0` (weakly informative; one event in one unit as prior mean)
- [ ] `fn next_occurrence_expected` = `1.0 / mean_rate` (expected inter-arrival time)
- [ ] `struct BetaBernoulli { alpha: f64, beta: f64 }` — conjugate prior/posterior for Bernoulli success probability p; prior `Beta(α, β)` → posterior after k successes, n-k failures: `Beta(α+k, β+n-k)`
- [ ] `impl BetaBernoulli { fn new(prior_alpha: f64, prior_beta: f64) -> Self; fn update(&mut self, successes: u64, failures: u64); fn mean_consistency(&self) -> f64; fn credible_interval_95(&self) -> (f64, f64); fn is_reliable(&self, threshold: f64, confidence: f64) -> bool }` — `is_reliable` = posterior probability that `p ≥ threshold ≥ confidence` (tail integral)
- [ ] Default prior for oracle consistency: `alpha = 1.0, beta = 1.0`; can be strengthened per domain
- [ ] Both posteriors `#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]` for persistence; state stored in Aster vault keyed by `(domain, anchor_kind)`
- [ ] CI computed via incomplete beta/gamma functions (use a small inline `f64` implementation of the regularized incomplete functions; ≤50 lines; no external crate)
- [ ] `pub fn gamma_poisson_for_domain(vault: &Vault, domain: DomainId) -> GammaPoisson` — loads from vault or initializes with default prior

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `GammaPoisson::default()` updated with 10 events in 5 time units → `mean_rate ≈ 11/6 ≈ 1.83 ± 0.1`; CI contains `true_rate = 2.0` (planted)
- [ ] unit: `GammaPoisson` updated with 0 events → `mean_rate = 1.0` (prior mean); `next_occurrence_expected = 1.0`
- [ ] unit: `BetaBernoulli::default()` updated with 9 successes + 1 failure → `mean_consistency ≈ 10/12 ≈ 0.83 ± 0.05`; CI `(0.6, 0.97)` (approximate)
- [ ] unit: `BetaBernoulli.is_reliable(threshold=0.7, confidence=0.9)` → `true` after 9/10 successes
- [ ] unit: `BetaBernoulli.is_reliable(threshold=0.7, confidence=0.9)` → `false` after 1/10 successes
- [ ] proptest: `credible_interval_95.0 ≤ mean ≤ credible_interval_95.1` always holds for both posteriors; CI width decreases monotonically with `n` observations
- [ ] edge (≥3): `GammaPoisson` with `n = 1` event → CI is wide but finite (no infinity); `BetaBernoulli` with 0 observations → mean = 0.5 (prior mean); both serde round-trip byte-identical
- [ ] fail-closed: `interval = 0` in `GammaPoisson::update` → `Err` with `CALYX_BAYES_INVALID_INTERVAL`; negative event count → error

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `GammaPoisson` and `BetaBernoulli` JSON from `calyx readback bayesian_posterior --domain <domain>`; test output for planted rate
- **Readback:**
  ```
  cargo test -p calyx-assay -- bayesian --nocapture 2>&1 | tee /tmp/ph52_bayes.log
  grep "mean_rate\|credible_interval\|mean_consistency" /tmp/ph52_bayes.log
  # GammaPoisson: CI contains true rate 2.0
  # BetaBernoulli: CI for 9/10 successes in [0.6, 0.97]
  ```
- **Prove:** `GammaPoisson` CI contains true planted rate (2.0) after 10 events in 5 units; `BetaBernoulli` CI for 9/10 successes is `≈ (0.6, 0.97)` ± 0.05; `is_reliable(0.7, 0.9) = true` after 9/10; `is_reliable(0.7, 0.9) = false` after 1/10

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH52 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
