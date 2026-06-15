# PH17 · T03 — TeiHttpLens runtime + batching

| Field | Value |
|---|---|
| **Phase** | PH17 — Lens trait + algorithmic + tei-http runtimes |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/runtime/tei_http.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A4 |
| **PRD** | `dbprdplans/05 §2` |

## Goal

Implement `TeiHttpLens`, the HTTP client that calls the resident TEI embedding
service on aiwonder. Correctly handles single-input and batched calls, chunks
over the batch ceiling, and reassembles output in original order. This runtime
is the primary path for the GTE 768-d general lens at `:8088`.

**Never spawn a throwaway TEI process.** Always call the already-running
resident service (`05 §2` gotcha).

## Build (checklist of concrete, code-level steps)

- [x] `TeiHttpLens` struct: `id: LensId`, `endpoint: Url`, `dim: u32`,
  `modality: Modality`, `batch_ceiling: usize` (default 32).
- [x] HTTP client: use `reqwest` (blocking feature) or `ureq`; POST to
  `{endpoint}/embed` with body `{"inputs": ["text1", "text2", …]}`.
- [x] Parse response: `[[f32; dim]; n]` JSON array; fail-closed with
  `CALYX_REGISTRY_RUNTIME_UNAVAILABLE` if connection refused, timeout, or
  non-200.
- [x] `measure(&self, input: &Input) -> Result<SlotVector>`:
  - decode `input.bytes` as UTF-8; error if not valid UTF-8.
  - single-item POST; parse first embedding row.
  - return `SlotVector::Dense { dim, data }`.
- [x] `measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>>`:
  - chunk inputs into `batch_ceiling`-sized groups.
  - POST each chunk; flatten responses maintaining order.
  - return `Vec<SlotVector>` with same len as inputs.
- [x] Implement `calyx_core::Lens` trait for `TeiHttpLens`.
- [x] `#[cfg(feature = "tei-integration")]` gate all network calls so unit tests
  compile without a live TEI; integration tests marked `#[ignore]` by default.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit (mock): construct `TeiHttpLens` pointing at a local mock server
  that returns a known 768-d unit-norm vector; `measure` returns
  `SlotVector::Dense { dim: 768, data }` with the expected values.
- [x] unit (mock): `measure_batch` with 70 inputs → two POST calls (32 + 32 + 6);
  result length equals 70 and order is preserved.
- [x] edge (≥3): (1) server returns HTTP 500 → `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`;
  (2) connection refused → same error code; (3) response array shorter than
  inputs → `CALYX_REGISTRY_RUNTIME_UNAVAILABLE` with remediation.
- [x] fail-closed: non-UTF-8 input bytes → `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`
  with remediation "TEI runtime requires UTF-8 text input".
- [x] integration (`#[ignore]`): POST to `:8088` with `"hello world"` →
  `SlotVector::Dense { dim: 768 }`; all values finite.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** integration test output on aiwonder running with `--include-ignored`
- **Readback:**
  `cargo test -p calyx-registry tei -- --include-ignored --nocapture 2>&1`
- **Prove:** output shows `dim=768`, all 768 values printed as finite floats;
  no `NaN` or `Inf` present; first 8 values hex-dumped and attached to PH17
  GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH17 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
