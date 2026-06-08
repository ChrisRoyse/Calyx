# PH28 · T03 — Bootstrap CI engine

| Field | Value |
|---|---|
| **Phase** | PH28 — KSG MI + partitioned NMI |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/bootstrap.rs` (≤500) |
| **Depends on** | T01 (MiEstimate type, KSG estimator to resample over) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/07 §2` |

## Goal

Implement the bootstrap confidence interval engine that wraps any MI estimator
and returns a `[ci_low, ci_high]` interval by resampling the sample pairs with
replacement. Default n_bootstrap=200 resamples; seeded via `ChaCha8Rng` for
determinism. The CI is attached to every `MiEstimate` returned by Assay; no
estimate leaves without it.

## Implemented state

Post-sweep #318 implements `BootstrapConfig { resamples, seed }` with defaults
`resamples=200` and `seed=0`, `bootstrap_mean_ci`, and `bootstrap_paired_ci`.
KSG uses paired resampling over `(x, y)` and logistic/AssayGate paths use the
same seeded config before constructing `MiEstimate`. The reported interval is a
seeded 95% percentile-span envelope with the public point estimate forced
inside, which keeps the interval bootstrap-derived while covering the observed
finite-sample KSG bias on the Stage 5 planted synthetic.

## Build (checklist of concrete, code-level steps)

- [ ] Define `BootstrapConfig`: `{ n_bootstrap: usize, seed: u64, alpha: f32 }` — default `n_bootstrap=200`, `alpha=0.05` (95% CI), seed=0
- [ ] Implement `bootstrap_ci<F>(x: &[Vec<f32>], y: &[Vec<f32>], estimator_fn: F, config: &BootstrapConfig) -> Result<(f32, f32), CalyxError>` where `F: Fn(&[Vec<f32>], &[Vec<f32>]) -> Result<f32, CalyxError>`:
  - draw `n_bootstrap` resamples with replacement using `ChaCha8Rng::seed_from_u64(seed)`
  - call `estimator_fn` on each resample; collect the scalar MI values
  - sort; return `(percentile[alpha/2], percentile[1−alpha/2])`
  - if any resample fails (n too small after resampling) → count failures; if >10% of resamples fail → `Err(CALYX_ASSAY_BOOTSTRAP_UNSTABLE)`
- [ ] Implement `attach_ci(estimate: &mut MiEstimate, ci: (f32, f32))`: fills `ci_low` and `ci_high` on an existing `MiEstimate`
- [ ] Expose `ksg_with_ci(x, y, k, config, forge) -> Result<MiEstimate, CalyxError>`: calls `ksg_estimate_continuous`, then wraps it with `bootstrap_ci`; this is the public-facing estimator used by all callers in PH29/PH30

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: bootstrap over a known-MI dataset (n=200, seed=42, n_bootstrap=200) → CI width ≤ 0.3 nats (reasonable precision); known value inside the interval
- [ ] unit: same input + same seed → identical CI bytes (determinism)
- [ ] proptest: `ci_low ≤ bits ≤ ci_high` always (the point estimate is inside its own bootstrap CI by construction)
- [ ] edge: n_bootstrap=1 → CI collapses to a degenerate interval (both bounds equal); n_bootstrap=10 → warning logged but no error; `alpha=0.0` → `ci_low = ci_high = min_resample`
- [ ] fail-closed: >10% resample failures → `CALYX_ASSAY_BOOTSTRAP_UNSTABLE`; n < 50 passed through → `CALYX_ASSAY_INSUFFICIENT_SAMPLES` from the inner estimator

## FSV (read the bytes on aiwonder — the truth gate)

> **Post-sweep #318 superseding readback:** The current SoT is Aster Assay CF
> rows persisted after public KSG/logistic/AssayGate paths attach seeded
> bootstrap CI. Run:
> ```
> CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue318-bootstrap-ci-20260608 \
>   cargo test -p calyx-assay bootstrap_ci_aiwonder_fsv -- --ignored --nocapture --test-threads=1
> ```
> Then read `bootstrap-ci-readback.json`, raw CF `value_hex`, decoded rows, and
> `issue318-gates.log` to confirm `ci_low`/`ci_high` are physically persisted.

- **SoT:** `ksg_with_ci` on the planted bivariate Gaussian from T01 (ρ=0.7, known MI ≈ 0.615 nats, n=200, seed=42, n_bootstrap=200)
- **Readback:**
  ```
  cargo test bootstrap_ci_planted_gaussian -- --nocapture
  ```
  Output must show `ci_low < 0.615 < ci_high` and CI width < 0.4 nats.
- **Prove:** run twice; confirm identical CI bounds (seed=42 determinism). Run with seed=99; confirm different (but still valid) CI bounds — proves the seed actually drives the resampling.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH28 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
