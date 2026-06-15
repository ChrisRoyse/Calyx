# PH18 · T03 — Dim/dtype mismatch guard

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

Implement the `CALYX_LENS_DIM_MISMATCH` guard: after every `measure` call,
check that the returned `SlotVector`'s actual dimension matches the dimension
declared in `LensSpec.output` (`SlotShape`). A runtime that lies about its
output shape must be caught immediately.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn check_dim(actual: &SlotVector, expected: SlotShape) -> Result<()>`:
  - match on `(actual, expected)`:
    - `(SlotVector::Dense { dim: a, .. }, SlotShape::Dense(e))`:
      if `a != e` → `Err(frozen_dim_mismatch(…))`.
    - `(SlotVector::Sparse { ambient_dim: a, .. }, SlotShape::Sparse(e))`:
      if `a != e` → error.
    - shape variant mismatch (e.g. Dense returned but Sparse declared) → error.
  - `CalyxError::frozen_dim_mismatch` constructor: code
    `"CALYX_LENS_DIM_MISMATCH"`, remediation `"re-register the lens with the
    correct SlotShape or fix the runtime to emit the declared dimension"`.
- [x] `pub fn check_shape_variant(actual: &SlotVector, expected: SlotShape) -> Result<()>`:
  separate check for Dense vs Sparse vs Multi variant mismatch (same error
  code, different remediation suffix).
- [x] Hook `check_dim` into `Registry::measure` and `Registry::measure_batch`
  as a post-call guard (wraps every runtime call uniformly).
- [x] Stub check for `SlotVector::Multi { token_dim }` vs
  `SlotShape::Multi { token_dim }` for completeness even though no multi-vector
  runtime ships yet.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `check_dim(Dense { dim: 768 }, SlotShape::Dense(768))` → `Ok(())`.
- [x] unit: `check_dim(Dense { dim: 128 }, SlotShape::Dense(768))` → `Err`
  with code `"CALYX_LENS_DIM_MISMATCH"`.
- [x] unit: `check_dim(Sparse { ambient_dim: 30522 }, SlotShape::Dense(768))`
  → `Err` with code `"CALYX_LENS_DIM_MISMATCH"` (variant mismatch).
- [x] integration: mock runtime declaring `Dense(4)` but returning `Dense(8)`
  → `Registry::measure` returns `CALYX_LENS_DIM_MISMATCH`; no vector returned.
- [x] edge (≥3): (1) dim 0 declared, dim 0 returned → `Ok(())` (degenerate
  but not a mismatch); (2) `Multi { token_dim: 64 }` matches → `Ok(())`; (3)
  `Multi { token_dim: 64 }` vs `Multi { token_dim: 32 }` → mismatch.
- [x] fail-closed: any mismatch → exact code `"CALYX_LENS_DIM_MISMATCH"` with
  non-empty remediation; no partial or coerced vector returned.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-registry dim_mismatch` on aiwonder
- **Readback:** `cargo test -p calyx-registry -- --nocapture 2>&1 | grep DIM`
- **Prove:** test output shows `CALYX_LENS_DIM_MISMATCH` on the wrong-dim mock
  and `Ok` on the correct-dim mock; attached to PH18 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH18 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
