# PH17 · T02 — AlgorithmicLens runtime

| Field | Value |
|---|---|
| **Phase** | PH17 — Lens trait + algorithmic + tei-http runtimes |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/runtime/algorithmic.rs` (≤500), `crates/calyx-registry/src/runtime/mod.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A4, A6 |
| **PRD** | `dbprdplans/05 §2` |

## Goal

Implement `AlgorithmicLens` — a deterministic feature encoder with no neural
network. Supports three encoder kinds: `Scalar` (fixed typed fields → Dense
float vector), `OneHot` (categorical → binary sparse), and `AstStyle` (token-
sequence hash-projection). These are absorbed from ContextGraph's
`algorithmic_embedder_synthesis` patterns. Output must be bit-for-bit
reproducible for the same input on any run.

## Build (checklist of concrete, code-level steps)

- [x] `AlgorithmicKind` enum:
  `Scalar { fields: Vec<ScalarField> }`,
  `OneHot { vocab_size: u32 }`,
  `AstStyle { hash_dim: u32 }`.
- [x] `ScalarField` struct: `name: String`, `scale: f32`, `offset: f32`.
- [x] `AlgorithmicLens` struct implementing `calyx_core::Lens`:
  - `id()` → pre-computed `LensId` from spec.
  - `shape()` → `SlotShape::Dense(n)` where `n` matches the encoder config.
  - `modality()` → declared at construction.
  - `measure(&self, input: &Input) -> Result<SlotVector>`:
    - `Scalar`: interpret `input.bytes` as JSON object; extract fields by name;
      apply `(v + offset) * scale`; fill missing fields with 0.0; L2-normalize
      if `NormPolicy::L2`; return `SlotVector::Dense`.
    - `OneHot`: hash `input.bytes` with blake3; take `hash % vocab_size` as the
      active index; return `SlotVector::Sparse` with one entry of 1.0.
    - `AstStyle`: split `input.bytes` on whitespace; for each token compute
      `blake3(token)[0..4]` as u32; map to `hash_dim` bins via `% hash_dim`;
      accumulate bin counts; L2-normalize; return `SlotVector::Dense`.
- [x] All three variants produce **no non-finite values** by construction
  (document invariant in code comment).
- [x] `measure_batch` uses the default iterator path (correctness); no special
  batching needed for algorithmic lenses.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `Scalar` on `{"x": 2.0, "y": -1.0}` with `scale=0.5, offset=0.0`
  → known float vector; re-measure same bytes → identical bits.
- [x] unit: `OneHot` on `b"hello"` → `SlotVector::Sparse` with exactly one
  non-zero entry equal to 1.0.
- [x] unit: `AstStyle` on `b"fn foo bar"` → `SlotVector::Dense`; same input
  twice → bit-identical output.
- [x] proptest: for any non-empty byte slice, `AstStyle` output is finite
  (no NaN/Inf) and L2-norm ≈ 1.0 (within 1e-5).
- [x] edge (≥3): (1) empty `input.bytes` for `Scalar` → all-zeros vector, no
  panic; (2) `OneHot` on max-size input (64 KiB) → still one entry, no alloc
  explosion; (3) `AstStyle` on single-token input → single occupied bin, norm 1.
- [x] fail-closed: `Scalar` on non-UTF-8 bytes → `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`
  with remediation "input must be valid UTF-8 JSON for Scalar algorithmic lens".

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-registry algorithmic` output on aiwonder
- **Readback:** `cargo test -p calyx-registry -- --nocapture -q 2>&1 | grep -E 'algorithmic|PASS|FAIL'`
- **Prove:** all three encoder kinds produce identical bit patterns on two
  consecutive runs with the same seed input; hex-dump of the `AstStyle` result
  for `b"fn foo bar"` attached to PH17 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH17 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
