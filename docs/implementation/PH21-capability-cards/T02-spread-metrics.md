# PH21 · T02 — Spread metrics (participation-ratio + stable-rank)

| Field | Value |
|---|---|
| **Phase** | PH21 — Capability cards / profile |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/profile/spread.rs` (≤500) |
| **Depends on** | T01 (this phase), PH12 (Forge CPU — SVD/eigenvalue ops) |
| **Axioms** | A6 |
| **PRD** | `dbprdplans/05 §5` |

## Goal

Compute `participation_ratio` and `stable_rank` from the N×D matrix of probe
embeddings. These two numbers together answer "is this lens rich (distributed
variance across many dimensions) or collapsed (all variance in one direction)?"

## Build (checklist of concrete, code-level steps)

- [x] `pub fn participation_ratio(embeddings: &[Vec<f32>]) -> f32`:
  - build N×D matrix `X`.
  - compute covariance `C = X^T X / N` (or centre first for unit-normed vecs:
    since unit-normed, `X^T X / N` ≈ correlation matrix).
  - compute eigenvalues `λ_i` of `C` (use Forge LAPACK binding or iterative
    power method for D≤1024; for larger D, use randomized SVD).
  - `participation_ratio = (sum(λ))^2 / sum(λ^2)` — normalised effective dim.
  - scale to `[0, 1]` by dividing by D: `pr = participation_ratio / D`.
  - return the scaled value.
- [x] `pub fn stable_rank(embeddings: &[Vec<f32>]) -> f32`:
  - compute singular values `σ_i` of X (via SVD or `σ_i = sqrt(λ_i)`).
  - `stable_rank = (sum(σ))^2 / sum(σ^2) / D` (same normalization).
- [x] `pub fn spread_metrics(embeddings: &[Vec<f32>]) -> Result<SpreadMetrics>`:
  - if `embeddings.len() < 2` → `Err(CalyxError::…)` with remediation
    "need at least 2 probe embeddings for spread computation".
  - call both functions; return `SpreadMetrics { participation_ratio, stable_rank }`.
- [x] For D > 1024: use a randomized PCA (top-k singular values, k=64) to
  avoid O(D^2) cost; document approximation in comment.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: N=10 embeddings that are all equal (rank-1 matrix) →
  `participation_ratio ≈ 0.0` (well below `COLLAPSE_THRESHOLD = 0.05`).
- [x] unit: N=100 isotropic unit Gaussian embeddings (seeded RNG) in D=64 →
  `participation_ratio` is close to 1.0 (within 0.2 of 1.0).
- [x] unit: `stable_rank >= participation_ratio` always (since stable rank ≥ PR
  by definition of these normalizations).
- [x] proptest: for any N≥2, D≥1 matrix, both metrics ∈ [0.0, 1.0].
- [x] edge (≥3): (1) N=2, D=768 → no panic; (2) all-zero matrix → PR≈0 (no
  signal); (3) D=1 → PR=1.0 (only one dimension, fully occupied).
- [x] fail-closed: N<2 → named error with non-empty remediation.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** unit test output on aiwonder
- **Readback:** `cargo test -p calyx-registry spread -- --nocapture 2>&1`
- **Prove:** test output shows `rank-1 matrix → PR≈0.00`, `isotropic →
  PR≈0.90±0.10` (seeded); both values within [0,1]; screenshot attached to
  PH21 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH21 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
