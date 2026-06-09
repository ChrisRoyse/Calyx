# PH19 · T04 — Dim guard + unit-norm for local runtimes

| Field | Value |
|---|---|
| **Phase** | PH19 — candle-local + onnx runtimes |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/runtime/candle.rs` (≤500), `crates/calyx-registry/src/runtime/onnx.rs` (≤500) |
| **Depends on** | T02, T03 (this phase), PH18 T03, PH18 T04 |
| **Axioms** | A4, A16 |
| **PRD** | `dbprdplans/05 §4`, `13_STAGE3_REGISTRY.md §PH19 FSV gate` |

## Goal

Confirm and exercise the dim guard and unit-norm checks specifically for
`CandleLocalLens` and `OnnxLens` — the PH19 FSV gate requires that `dim guard
fires on mismatch` and that both runtimes produce `finite, unit-norm vectors`.
This card is the focused test that proves those two properties independently
of the full contract chain.

## Build (checklist of concrete, code-level steps)

- [x] In `candle.rs`: after mean-pooling, compute `actual_dim = data.len()`;
  if `actual_dim != self.dim as usize` → return `CALYX_LENS_DIM_MISMATCH`
  before L2 normalization (catches model config errors early).
- [x] In `onnx.rs`: same check after extracting the flat `Vec<f32>`.
- [x] Shared helper `runtime_normalize_checked(data: Vec<f32>, expected_dim: u32, norm_policy: NormPolicy) -> Result<SlotVector>`:
  - check len == expected_dim → `CALYX_LENS_DIM_MISMATCH` if not.
  - L2-normalize in place if `norm_policy == NormPolicy::L2`.
  - call `check_finite` → `CALYX_LENS_NUMERICAL_INVARIANT` if not.
  - call `check_unit_norm(…, tol=1e-4)` → `CALYX_LENS_NUMERICAL_INVARIANT` if not.
  - return `SlotVector::Dense { dim: expected_dim, data }`.
- [x] Both `CandleLocalLens::measure` and `OnnxLens::measure` call
  `runtime_normalize_checked` as their final step.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `runtime_normalize_checked` on a 3-vector `[3.0, 4.0, 0.0]`
  with `expected_dim=3`, `L2` → norm ≈ 1.0 (`[0.6, 0.8, 0.0]`), no error.
- [x] unit: `runtime_normalize_checked` on 4 values but `expected_dim=3` →
  `CALYX_LENS_DIM_MISMATCH`.
- [x] unit: `runtime_normalize_checked` on `[f32::NAN, 0.0, 0.0]` →
  `CALYX_LENS_NUMERICAL_INVARIANT` (post-normalize NaN).
- [x] integration (`#[ignore]`): `CandleLocalLens` on aiwonder returns dim=768,
  norm ∈ [0.9999, 1.0001].
- [x] integration (`#[ignore]`): `OnnxLens` on aiwonder returns dim=768,
  norm ∈ [0.9999, 1.0001].
- [x] edge (≥3): (1) all-zero vector → after L2-normalize with zero norm →
  `CALYX_LENS_NUMERICAL_INVARIANT` (zero norm is non-finite after division);
  (2) dim=1 vector → normalizes to ±1.0; (3) very large values (1e30) →
  normalize produces unit-norm finite vector.
- [x] fail-closed: any dim mismatch or non-finite value → named CALYX_* error.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** integration test output on aiwonder for both candle and ONNX
- **Readback:**
  `cargo test -p calyx-registry dim_guard -- --include-ignored --nocapture 2>&1`
- **Prove:** output shows for each runtime: `dim=768 norm=1.0000` and the
  forced-mismatch test showing `CALYX_LENS_DIM_MISMATCH`; attached to PH19
  GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH19 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
