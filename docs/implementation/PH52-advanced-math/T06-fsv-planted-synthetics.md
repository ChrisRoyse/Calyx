# PH52 · T06 — FSV: all five new numbers proven against planted synthetics

| Field | Value |
|---|---|
| **Phase** | PH52 — Advanced math |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-assay` (test file spans `calyx-assay`, `calyx-mincut`, `calyx-lodestar`) |
| **Files** | `crates/calyx-assay/tests/advanced_math_fsv.rs` (≤500) |
| **Depends on** | T01 (spectral), T02 (transfer entropy), T03 (TC/n_eff), T04 (Bayesian), T05 (label propagation) |
| **Axioms** | A30, A2, A16 |
| **PRD** | `dbprdplans/26 §2–§6`, `dbprdplans/26 §11.2` |

## Goal

Prove the PH52 FSV exit gate on aiwonder: each new number is proven against a **planted
synthetic** by reading the computed value, not a harness (`26 §10`):
1. Planted period via **Lomb-Scargle** recovers within ±5%.
2. Planted causal A→B via **transfer entropy**: `T(A→B) > T(B→A)`.
3. Planted rare-class carrier via stratified bits + **label propagation**: label propagates
   to the rare-class node.
4. Planted community via **spectral**: second Laplacian eigenvector bisects the community.
5. All five numbers carry CI; fail-closed below quorum.

## Build (checklist of concrete, code-level steps)

- [ ] **FSV test 1 — planted period (Lomb-Scargle):** generate a synthetic recurrence stream with planted period `T_true = 7.0` time units (100 events, Gaussian jitter, seeded); run Lomb-Scargle periodogram over the recurrence timestamps; assert dominant period `T_detected ∈ [6.65, 7.35]` (±5%); write `{T_true, T_detected}` to `/tmp/ph52_period.json`
  - Lomb-Scargle implementation: 2-phase: (1) compute the test frequencies `ω` on a grid from `1/T_max` to `1/T_min_inter_event`; (2) compute the Lomb-Scargle power `P(ω) = (Σ cos²(ω(t-τ)))^{-1}·(Σ cos(ω(t-τ)))² + (...)` using numerically stable forms; peak frequency → dominant period. ≤50 lines, inline in `transfer_entropy.rs` or a small helper.
- [ ] **FSV test 2 — planted causal (transfer entropy):** 100-event stream where A always fires 2 steps before B (planted lag=2, seeded); `transfer_entropy_sweep([1,2,4,8])`; assert `t_a_to_b > t_b_to_a + 0.1` at lag=2; `dominant_direction = A_to_B`; CI does not overlap zero for `t_a_to_b`; write to `/tmp/ph52_te_fsv.json`
- [ ] **FSV test 3 — planted rare-class (label propagation):** synthetic association graph with 20 nodes; 2 nodes are rare-class carriers (the only nodes receiving a specific anchor label); both are in the MFVS kernel; run `propagate_labels`; assert the two nearest-neighbor non-kernel nodes receive `confidence > 0.3` with `provisional = true`; write to `/tmp/ph52_label_prop.json`
- [ ] **FSV test 4 — planted community (spectral):** synthetic 10-node graph = two 5-cliques joined by one bridge edge; `laplacian_eigenmaps(k=2)`; second eigenvector (Fiedler vector) must have positive values on one clique and negative on the other; `spectral_gap` detects the bottleneck (small Fiedler value < 0.1); write to `/tmp/ph52_spectral_fsv.json`
- [ ] **FSV test 5 — Bayesian CI coverage:** 10 replications, each with `true_rate = 2.0` and 10 events observed in 5 time units (seeded across replications); assert that ≥9/10 of the `GammaPoisson` CIs contain `true_rate = 2.0` (≥90% nominal coverage); write coverage rate to `/tmp/ph52_bayes_fsv.json`
- [ ] All tests: seeded RNG (seed `42` for reproducibility); deterministic; print JSON output for readback
- [ ] Use `calyx-testkit` `MockClock`; all randomness seeded

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] FSV test 1: `T_detected ∈ [6.65, 7.35]` for planted period 7.0
- [ ] FSV test 2: `t_a_to_b > t_b_to_a + 0.1`; `dominant_direction = "A_to_B"` in JSON
- [ ] FSV test 3: nearest neighbors of rare-class kernel nodes have `confidence > 0.3`; `provisional = true`
- [ ] FSV test 4: Fiedler vector sign-bisects the two cliques; `spectral_gap < 0.1`
- [ ] FSV test 5: ≥9/10 Bayesian CIs contain true rate
- [ ] Cross-check: all five results have `provisional = false` for grounded nodes and `provisional = true` for inferred/near-insufficient nodes (tag discipline scan)
- [ ] fail-closed: each test with too-short data → `provisional = true` or appropriate `CALYX_*` error; never a confident fabrication

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/tmp/ph52_period.json`, `/tmp/ph52_te_fsv.json`, `/tmp/ph52_label_prop.json`, `/tmp/ph52_spectral_fsv.json`, `/tmp/ph52_bayes_fsv.json`
- **Readback:**
  ```
  cargo test -p calyx-assay -- advanced_math_fsv --nocapture 2>&1 | tee /tmp/ph52_fsv.log
  cat /tmp/ph52_period.json       | jq '{T_true, T_detected, within_5pct: ((.T_detected - .T_true) | fabs | . <= .T_true * 0.05)}'
  cat /tmp/ph52_te_fsv.json       | jq '{t_a_to_b, t_b_to_a, dominant_direction}'
  cat /tmp/ph52_label_prop.json   | jq '.labels[] | select(.hop_distance == 1) | {node_id, confidence, provisional}'
  cat /tmp/ph52_spectral_fsv.json | jq '{spectral_gap, fiedler_sign_count_positive, fiedler_sign_count_negative}'
  cat /tmp/ph52_bayes_fsv.json    | jq '.coverage_rate'                          # must be >= 0.9
  ```
- **Prove:** (1) period within ±5% of planted 7.0; (2) `t_a_to_b > t_b_to_a` in JSON; (3) rare-class neighbors have `confidence > 0.3`, `provisional = true`; (4) Fiedler vector splits 5+5 (5 positive, 5 negative signs); (5) Bayesian coverage ≥ 0.9

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence — all 5 JSON files screenshots attached to the PH52 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
