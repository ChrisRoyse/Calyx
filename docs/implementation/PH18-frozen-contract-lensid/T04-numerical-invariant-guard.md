# PH18 · T04 — Finite + unit-norm numerical invariant guard

| Field | Value |
|---|---|
| **Phase** | PH18 — Frozen contract + content-addressed LensId |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/frozen.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A4, A16 |
| **PRD** | `dbprdplans/05 §4` |

## Goal

Implement `CALYX_LENS_NUMERICAL_INVARIANT`: every `measure` result must be
finite (no NaN or Inf) and, if `NormPolicy::L2` is declared, unit-norm within
tolerance (1.0 ± 1e-4). Catch degenerate or training-damaged vectors before
they enter the vault.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn check_finite(vec: &SlotVector) -> Result<()>`:
  - iterate all `f32` values in `SlotVector::Dense.data` (and Sparse entries).
  - if any `f32::is_nan(v)` or `f32::is_infinite(v)` → `Err(numerical_invariant(…))`.
  - `CalyxError::numerical_invariant` constructor: code
    `"CALYX_LENS_NUMERICAL_INVARIANT"`, remediation `"the lens runtime produced
    a non-finite value; check for training damage, overflow, or misconfigured
    normalization"`.
- [x] `pub fn check_unit_norm(vec: &SlotVector, norm_policy: NormPolicy, tol: f32) -> Result<()>`:
  - if `norm_policy != NormPolicy::L2` → `Ok(())` (skip check).
  - compute `norm = sqrt(sum(v^2))` over Dense data.
  - if `(norm - 1.0).abs() > tol` → `Err(numerical_invariant("L2 norm out of
    tolerance"))`.
  - recommended `tol = 1e-4`.
- [x] `pub fn check_not_unreachable(vec: &SlotVector) -> Result<()>`:
  - if `SlotVector::Dense.data` is empty → `Err(lens_unreachable(…))`.
  - `CalyxError::lens_unreachable`: code `"CALYX_LENS_UNREACHABLE"`, remediation
    `"the lens runtime returned an empty vector; confirm the runtime is loaded
    and the input modality is supported"`.
- [x] Compose into `pub fn check_output(vec: &SlotVector, spec: &LensSpec) -> Result<()>`:
  calls `check_not_unreachable`, `check_dim`, `check_finite`, `check_unit_norm`
  in that order; returns the first error encountered.
- [x] Hook `check_output` into `Registry::measure` after every runtime call.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `check_finite` on `Dense { data: vec![0.1, 0.2, 0.3] }` → `Ok(())`.
- [x] unit: `check_finite` on `Dense { data: vec![f32::NAN] }` → `Err` with
  `"CALYX_LENS_NUMERICAL_INVARIANT"`.
- [x] unit: `check_unit_norm` on a 3-vector `[1/sqrt(3); 3]` with `NormPolicy::L2`
  and `tol=1e-4` → `Ok(())`.
- [x] unit: `check_unit_norm` on `[0.0, 0.0, 0.5]` with L2 → `Err` (norm ≈
  0.5, out of tolerance).
- [x] unit: `check_not_unreachable` on empty Dense → `CALYX_LENS_UNREACHABLE`.
- [x] edge (≥3): (1) `f32::INFINITY` → `CALYX_LENS_NUMERICAL_INVARIANT`;
  (2) `f32::NEG_INFINITY` → same; (3) norm exactly 1.0 → `Ok(())`.
- [x] fail-closed: NaN input → exact `"CALYX_LENS_NUMERICAL_INVARIANT"`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-registry numerical_invariant` on aiwonder
- **Readback:** `cargo test -p calyx-registry -- --nocapture 2>&1 | grep -E 'NUMERICAL|UNREACHABLE'`
- **Prove:** NaN injection test prints `CALYX_LENS_NUMERICAL_INVARIANT`;
  unit-norm test on valid vector prints `Ok`; attached to PH18 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH18 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
