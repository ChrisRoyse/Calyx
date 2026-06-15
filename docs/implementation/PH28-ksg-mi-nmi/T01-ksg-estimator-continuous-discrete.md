# PH28 · T01 — KSG estimator: k-NN MI, bias correction, continuous↔discrete

| Field | Value |
|---|---|
| **Phase** | PH28 — KSG MI + partitioned NMI |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/ksg.rs` (≤500) |
| **Depends on** | PH13 (Forge ANN k-NN graph for neighbor queries) · PH09 (Aster anchor reads) |
| **Axioms** | A2, A16 |
| **PRD** | `dbprdplans/07 §2` |

## Goal

Implement the KSG (Kraskov–Stögbauer–Grassberger) mutual information estimator
as the production default for vector↔outcome MI. Supports continuous↔continuous
(KSG Algorithm 1) and continuous↔discrete modes. Bias-corrected; k-NN distances
from the existing Forge ANN graph. Every estimate returns
`MiEstimate { bits: f32, ci_low: f32, ci_high: f32, n_samples: usize,
estimator: EstimatorKind::Ksg }`. Fails closed when n < 50.

## Build (checklist of concrete, code-level steps)

- [x] Define `MiEstimate`: `{ bits: f32, ci_low: f32, ci_high: f32, n_samples: usize, estimator: EstimatorKind, trust: Trusted | Provisional, anchor: AnchorKind }`
- [x] Define `EstimatorKind` enum: `Ksg { k: usize }`, `PartitionedHistogramNmi`, `LinearCorr`, `LogisticProbe`
- [x] Implement `ksg_estimate_continuous(x: &[Vec<f32>], y: &[Vec<f32>], k: usize, forge: &ForgeHandle) -> Result<MiEstimate, CalyxError>`:
  - for each sample `i`: find `k` nearest neighbors in the joint `(x_i, y_i)` space using Forge ANN; record `eps_x[i]` and `eps_y[i]` (max-norm radius to the `k`-th neighbor, projected onto x and y marginals)
  - count `n_x[i]` = samples with `‖x_j − x_i‖ < eps_x[i]`, `n_y[i]` similarly
  - KSG Algorithm 1: `MI = psi(k) − <psi(n_x+1)+psi(n_y+1)> + psi(N)` where `psi` is the digamma function
  - bias correction: subtract the small-k bias term `1/k` (standard)
  - if n < 50: return `Err(CalyxError::AssayInsufficientSamples { n, required: 50 })`
- [x] Implement `ksg_estimate_discrete_y(x: &[Vec<f32>], y: &[u32], k: usize, forge: &ForgeHandle) -> Result<MiEstimate, CalyxError>`:
  - for continuous↔discrete: separate by class; compute within-class k-NN distances; use the mixed KSG formula with correction for tied distances in the discrete dimension
  - if n < 50: fail closed
- [x] `digamma(x: f64) -> f64`: Lanczos approximation, accurate to 1e-10; tested against tabulated values
- [x] Tag result `trust: Trusted` only when the anchor passed in is `AnchorKind::Grounded`; else `Provisional`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: two independent Gaussian samples (n=200, seed=42) → MI ≈ 0.0 ± 0.05 nats; confirmed within CI
- [x] unit: two perfectly correlated Gaussians `y = x + ε` (ε very small, n=200, seed=42) → MI ≈ `0.5·ln(1+SNR)` nats; within CI of known value
- [x] unit: discrete-y case: `y ∈ {0,1}` with `p(y=1|x>0) = 0.9` (n=200, seed=42) → MI ≈ known KL-divergence value; within CI
- [x] edge: n=30 (below quorum) → `CALYX_ASSAY_INSUFFICIENT_SAMPLES`; n=50 exactly → does not fail closed; k=1 → does not panic (edge of KSG validity range, CI will be wide)
- [x] fail-closed: x and y with different lengths → `CALYX_ASSAY_MISMATCHED_SAMPLES`; empty input → `CALYX_ASSAY_INSUFFICIENT_SAMPLES`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** planted bivariate Gaussian with known MI = `−0.5·ln(1−ρ²)` for ρ=0.7 ≈ 0.615 nats (seeded, n=500)
- **Readback:**
  ```
  cargo test ksg_planted_gaussian_mi -- --nocapture
  ```
  Printed output must show `bits = 0.615 ± δ` where the known 0.615 nats is inside `[ci_low, ci_high]`.
- **Prove:** run on aiwonder; capture the printed `MiEstimate`; confirm `ci_low < 0.615 < ci_high`. Also run with n=30 and confirm `CALYX_ASSAY_INSUFFICIENT_SAMPLES` is printed.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH28 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
