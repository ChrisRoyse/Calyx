# PH17 · T06 — Error catalog + fail-closed paths

| Field | Value |
|---|---|
| **Phase** | PH17 — Lens trait + algorithmic + tei-http runtimes |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/error.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/05 §4`, DOCTRINE §9 (fail-closed) |

## Goal

Define and test every `CALYX_REGISTRY_*` error code used in PH17, ensuring
every error path returns a structured `CalyxError` with a `code` string and
non-empty `remediation` string. No silent fallbacks or zero-fills anywhere in
the registry.

## Build (checklist of concrete, code-level steps)

- [x] In `error.rs`, define constructor fns on `CalyxError` (or a
  `RegistryError` newtype that maps to `CalyxError`):
  - `lens_not_found(lens_id: LensId) -> CalyxError` → code
    `"CALYX_REGISTRY_LENS_NOT_FOUND"`, remediation `"register the lens with
    Registry::register before calling measure"`.
  - `runtime_unavailable(detail: &str) -> CalyxError` → code
    `"CALYX_REGISTRY_RUNTIME_UNAVAILABLE"`, remediation includes `detail`.
  - `duplicate_lens(lens_id: LensId) -> CalyxError` → code
    `"CALYX_REGISTRY_DUPLICATE"`, remediation `"retire or park the existing
    lens before re-registering under the same LensId"`.
  - `numerical_invariant(detail: &str) -> CalyxError` → code
    `"CALYX_LENS_NUMERICAL_INVARIANT"`, remediation includes `detail`.
    (This code is shared with PH18; define it here so PH18 can reuse it.)
- [x] Ensure `CalyxError` carries `code: String` and `remediation: String`
  fields (or extend `calyx-core` error type if not already present; coordinate
  with PH03 error catalog).
- [x] All error constructors are `#[inline]` and produce `const`-friendly
  string slices where possible.
- [x] Audit every `?` or `unwrap` in `runtime/algorithmic.rs` and
  `runtime/tei_http.rs` (T02/T03) to confirm they propagate one of these codes
  rather than a generic `anyhow` or `Box<dyn Error>`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `lens_not_found` → `error.code == "CALYX_REGISTRY_LENS_NOT_FOUND"`
  and `!error.remediation.is_empty()`.
- [x] unit: `runtime_unavailable("connection refused")` → code
  `"CALYX_REGISTRY_RUNTIME_UNAVAILABLE"` and remediation contains
  `"connection refused"`.
- [x] unit: `numerical_invariant("determinism probe")` → code
  `"CALYX_LENS_NUMERICAL_INVARIANT"` and remediation non-empty.
- [x] edge (≥3): (1) `duplicate_lens` error serde round-trips correctly;
  (2) all four error constructors produce distinct `code` strings; (3) a
  `runtime_unavailable` with an empty `detail` still has a non-empty
  remediation from the template.
- [x] fail-closed: calling `Registry::measure` on unregistered id → error code
  is exactly `"CALYX_REGISTRY_LENS_NOT_FOUND"` (no fallback, no zero vector).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-registry error` test output on aiwonder
- **Readback:** `cargo test -p calyx-registry -- --nocapture 2>&1 | grep CALYX_`
- **Prove:** output shows each of the four `CALYX_REGISTRY_*` / `CALYX_LENS_*`
  codes printed at least once by the test suite; no test uses `unwrap()` on an
  expected-error path; screenshot attached to PH17 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH17 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
