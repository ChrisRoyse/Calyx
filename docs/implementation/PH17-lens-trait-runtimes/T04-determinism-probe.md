# PH17 · T04 — Determinism probe (embed twice → identical)

| Field | Value |
|---|---|
| **Phase** | PH17 — Lens trait + algorithmic + tei-http runtimes |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/runtime/tei_http.rs` (≤500 — adds probe fn), `crates/calyx-registry/tests/determinism.rs` (≤500) |
| **Depends on** | T03 (this phase) |
| **Axioms** | A4 |
| **PRD** | `dbprdplans/05 §4`, `13_STAGE3_REGISTRY.md §PH17 FSV gate` |

## Goal

Implement and run the determinism probe required by the PH17 FSV gate and the
frozen contract (`05 §4`): embed the same input via `TeiHttpLens` twice and
assert the vectors are element-for-element identical (bitwise equal `f32`).
Also confirm `AlgorithmicLens` is bit-identical across runs.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn determinism_probe(lens: &dyn Lens, input: &Input) -> Result<()>`:
  - call `lens.measure(input)` → `v1`.
  - call `lens.measure(input)` → `v2`.
  - compare `v1 == v2` element-for-element (f32 bitwise equality via
    `f32::to_bits`); if any element differs → return
    `CALYX_LENS_NUMERICAL_INVARIANT` with remediation
    "lens failed determinism probe: two measurements of identical input differ".
  - on success → `Ok(())`.
- [x] Expose `determinism_probe` from `calyx_registry::frozen` (stub module for
  now; PH18 will fill it out fully).
- [x] Integration test `determinism_tei_8088` (`#[ignore]`): construct
  `TeiHttpLens` at `127.0.0.1:8088`; call `determinism_probe` with
  `Input::new(Modality::Text, b"the quick brown fox")` → assert `Ok(())`.
- [x] Unit test `determinism_algorithmic`: `AlgorithmicLens::AstStyle` on
  `b"fn main"` → probe returns `Ok(())`; assert in non-ignored test.
- [x] `Registry` calls `determinism_probe` on every lens at `register()` time
  (one probe pair per registration); if probe fails, registration fails with
  `CALYX_LENS_NUMERICAL_INVARIANT`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: mock lens that returns different vectors on second call →
  `determinism_probe` returns `Err` with exact code
  `CALYX_LENS_NUMERICAL_INVARIANT`.
- [x] unit: `AlgorithmicLens::Scalar` determinism probe → `Ok(())` on seeded
  known input.
- [x] edge (≥3): (1) probe on zero-length vector → OK (trivially equal);
  (2) probe on single-element vector with value `0.0` → OK; (3) mock lens
  that returns `NaN` → probe returns `CALYX_LENS_NUMERICAL_INVARIANT` because
  `NaN != NaN` bitwise (caught by the bit-equality check).
- [x] fail-closed: any difference in bit pattern → exact
  `CALYX_LENS_NUMERICAL_INVARIANT` with non-empty remediation string.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** integration test `determinism_tei_8088` output on aiwonder
- **Readback:**
  `cargo test -p calyx-registry determinism -- --include-ignored --nocapture 2>&1`
- **Prove:** output contains `determinism_probe OK` (or equivalent pass line);
  if flipped to a mock non-deterministic lens the test prints
  `CALYX_LENS_NUMERICAL_INVARIANT` and exits non-zero; both screenshots
  attached to PH17 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH17 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
