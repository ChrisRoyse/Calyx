# PH12 · T06 — NaN/Inf guards + fail-closed paths

| Field | Value |
|---|---|
| **Phase** | PH12 — CPU SIMD Backend |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cpu/guard.rs` (≤500) — shared guard helpers used by all CPU kernels |
| **Depends on** | T01, T02, T03, T04 (this phase) |
| **Axioms** | A13, A16 |
| **PRD** | `dbprdplans/13 §6`, `dbprdplans/13 §7` |

## Goal

Extract all NaN/Inf guard logic into a shared `cpu/guard.rs` module to ensure
every kernel boundary enforces the `CALYX_FORGE_NUMERICAL_INVARIANT` contract
uniformly. A16 requires that every error path returns a structured `CALYX_*`
code with remediation — no silent fallback, no zero-fill, no `unwrap`. This
card audits T02/T03/T04 and ensures the guard logic is consistent, tested, and
callable from future CUDA wrappers (PH13) and TurboQuant (PH14).

## Build (checklist of concrete, code-level steps)

- [x] `src/cpu/guard.rs`: `pub fn check_finite(slice: &[f32], op: &str) -> Result<(), ForgeError>`
  — linear scan using `f32::is_finite`; on first non-finite value returns
  `ForgeError::NumericalInvariant { op: op.to_string(), detail: format!("non-finite f32 at index {i}: {v}"), remediation: "Ensure all input vectors are normalized finite f32; check upstream embedding model output".to_string() }`
- [x] `pub fn check_norm_positive(norm: f32, op: &str, row: usize) -> Result<(), ForgeError>`
  — used by normalize and cosine; `norm > 0.0 && norm.is_finite()` or returns
  `ForgeError::NumericalInvariant { op, detail: format!("zero or non-finite norm at row {row}"), remediation: "..." }`
- [x] `pub fn check_shape_2d(slice: &[f32], rows: usize, cols: usize, name: &str) -> Result<(), ForgeError>`
  — `slice.len() == rows * cols` or `ForgeError::ShapeMismatch { expected: vec![rows, cols], got: vec![slice.len()] }`
- [x] Audit T02 (gemm.rs), T03 (distance.rs), T04 (normalize.rs/topk.rs): replace any
  inline guard logic with calls to these helpers; no duplication of the guard pattern
- [x] Add `remediation: String` field to `ForgeError::NumericalInvariant` and
  `ForgeError::DeviceUnavailable` (update T01's error.rs); `Display` appends
  `" Remediation: {remediation}"` on a new line so it's machine-parseable
- [x] The `CALYX_FORGE_NUMERICAL_INVARIANT` code must appear verbatim (as the
  first token) in every `ForgeError::NumericalInvariant` `Display` output — add
  a debug-assertion in `Display` impl to enforce this in test builds

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `check_finite(&[1.0, f32::NAN, 2.0], "test")` → `Err(ForgeError::NumericalInvariant)`;
  error `Display` contains `"CALYX_FORGE_NUMERICAL_INVARIANT"` at position 0
- [x] unit: `check_finite(&[1.0, f32::INFINITY, 2.0], "test")` → `Err`; index 1 in detail
- [x] unit: `check_norm_positive(0.0, "normalize", 7)` → `Err`; detail contains `"row 7"`
- [x] proptest: any `ForgeError` produced by any CPU kernel with a NaN-containing
  input → `Display` starts with `"CALYX_FORGE_"` and contains `"Remediation:"`
- [x] edge (≥3): (1) all-finite slice → `Ok(())`; (2) empty slice → `Ok(())`;
  (3) `f32::NEG_INFINITY` detected same as `INFINITY`
- [x] fail-closed: `check_shape_2d` with mismatched sizes → `ForgeError::ShapeMismatch`;
  `Display` contains the word `"expected"` and the word `"got"` with actual numbers

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `tests/cpu_kernels.rs::numerical_invariant_*` tests on aiwonder
- **Readback:**
  ```bash
  cargo test -p calyx-forge guard numerical_invariant -- --nocapture 2>&1 \
    | grep -E "CALYX_FORGE_NUMERICAL_INVARIANT|PASSED|FAILED"
  ```
- **Prove:** the grep prints `CALYX_FORGE_NUMERICAL_INVARIANT` at least once (from
  the nocapture output of the failing-input tests); all guard tests PASSED; absent:
  any `panic`, `unwrap`, or `expect` unwind in test output

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (guards do not alter numeric path)
- [x] FSV evidence (readback output showing `CALYX_FORGE_NUMERICAL_INVARIANT`) attached to PH12 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
