# PH18 · T02 — weights_sha256 frozen-violation guard

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

Implement the weights-hash check that enforces the frozen instrument contract:
if the hash of the weights a lens runtime actually uses does not match the
`weights_sha256` stored in `LensSpec`, fail immediately with
`CALYX_LENS_FROZEN_VIOLATION`. This is the primary guard against silent
weight drift.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn check_weights_sha256(actual: &[u8; 32], spec: &LensSpec) -> Result<()>`:
  - compare `actual` to `spec.weights_sha256` byte-for-byte.
  - if mismatch → `Err(CalyxError::frozen_violation(spec.lens_id, "weights_sha256 mismatch"))`.
  - `CalyxError::frozen_violation` constructor: code `"CALYX_LENS_FROZEN_VIOLATION"`,
    remediation `"re-register with the correct weights or restore the original
    model weights; do not mutate a registered frozen lens"`.
- [x] For `TeiHttpLens`: the resident TEI service manages weights; we cannot
  hash model bytes over HTTP. Instead, the hash is declared at registration
  time and treated as a trust anchor (operator's responsibility). Document
  this limitation with a comment: `// TEI weights hash is operator-declared;
  not re-verified per call (network model)`.
- [x] For `CandleLocal` / `OnnxRuntime` (stubs at this phase, implemented in
  PH19): `check_weights_sha256` will be called with the computed sha256 of the
  loaded weight file. Stub the call site now.
- [x] For `AlgorithmicLens`: `weights_sha256` is the hash of the serialized
  encoder config (deterministic); compute it at construction time and compare.
- [x] `Registry::register` calls `check_weights_sha256` before storing the
  lens; if it fails, registration is rejected.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `check_weights_sha256` with matching 32-byte arrays → `Ok(())`.
- [x] unit: `check_weights_sha256` with one differing byte → `Err` with code
  exactly `"CALYX_LENS_FROZEN_VIOLATION"`.
- [x] unit: register an `AlgorithmicLens` with correct config hash → succeeds;
  mutate the spec's `weights_sha256` before re-registering → fails with
  `CALYX_LENS_FROZEN_VIOLATION`.
- [x] edge (≥3): (1) all-zero `weights_sha256` matches all-zero stored →
  `Ok(())`; (2) all-zero stored vs all-ones actual → violation; (3) two
  registrations with same spec → second returns `CALYX_REGISTRY_DUPLICATE`
  (not a violation — that's a different error path).
- [x] fail-closed: any hash mismatch → exact code `"CALYX_LENS_FROZEN_VIOLATION"`,
  remediation string non-empty, no vector returned.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-registry frozen_violation` on aiwonder
- **Readback:** `cargo test -p calyx-registry -- --nocapture 2>&1 | grep FROZEN`
- **Prove:** test output shows `CALYX_LENS_FROZEN_VIOLATION` when hash is
  tampered; `Ok(())` when hash matches; attached to PH18 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH18 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
