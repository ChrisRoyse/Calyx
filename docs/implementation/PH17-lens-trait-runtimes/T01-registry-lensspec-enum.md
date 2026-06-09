# PH17 · T01 — Registry struct + LensSpec + LensRuntime enum

| Field | Value |
|---|---|
| **Phase** | PH17 — Lens trait + algorithmic + tei-http runtimes |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/lib.rs` (≤500), `crates/calyx-registry/src/lens.rs` (≤500), `crates/calyx-registry/src/error.rs` (≤500) |
| **Depends on** | PH09 (SlotId, panel_version), PH12 (Forge normalize) |
| **Axioms** | A4, A6 |
| **PRD** | `dbprdplans/05 §2` |

## Goal

Define the `Registry` struct that maps `LensId → Box<dyn Lens>`, the
`LensSpec` registration descriptor carrying all frozen metadata, and the
`LensRuntime` enum with all five variant names declared. This card establishes
the central data model that every subsequent card in PH17 extends.

## Build (checklist of concrete, code-level steps)

- [x] `LensRuntime` enum with five variants:
  `Algorithmic`, `TeiHttp { endpoint: Url }`, `CandleLocal { model_path: PathBuf }`,
  `Onnx { model_path: PathBuf }`, `ExternalCmd { cmd: String, args: Vec<String> }`.
- [x] `NormPolicy` enum: `L2`, `None`, `DeclaredByModel`.
- [x] `LensCost` struct: `ms_per_input: f32`, `vram_mb: u32`, `batch_ceiling: u32`.
- [x] `LensHealth` enum: `Loaded`, `Cold`, `Failing(String)`.
- [x] `LensSpec` struct mirroring `05 §2` fields:
  `lens_id: LensId`, `name: String`, `weights_sha256: [u8;32]`, `corpus_hash: [u8;32]`,
  `runtime: LensRuntime`, `output: SlotShape`, `modality: Modality`,
  `asymmetry: Option<Asymmetry>`, `normalize: NormPolicy`, `quant_default: QuantPolicy`,
  `cost: LensCost`, `health: LensHealth`.
- [x] `Registry` struct: `lenses: HashMap<LensId, (LensSpec, Box<dyn Lens>)>`.
  Methods: `register(spec, lens)`, `get_spec(id) -> Option<&LensSpec>`,
  `list() -> Vec<LensId>`.
- [x] `error.rs`: define `CALYX_REGISTRY_LENS_NOT_FOUND`,
  `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`, `CALYX_REGISTRY_DUPLICATE` as
  `CalyxError` constructors with remediation strings.
- [x] Wire `Cargo.toml` deps: `calyx-core`, `serde`, `serde_json`, `thiserror`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `register` then `get_spec` returns identical `LensSpec` fields
  (name, weights_sha256, output shape, modality all match byte-for-byte).
- [x] proptest: `LensSpec` serde round-trip: `deserialize(serialize(spec)) == spec`
  for arbitrary `name`, `weights_sha256`, `SlotShape::Dense(d)`.
- [x] edge (≥3): (1) `register` same `LensId` twice → `CALYX_REGISTRY_DUPLICATE`;
  (2) `get_spec` on unknown id → `CALYX_REGISTRY_LENS_NOT_FOUND`;
  (3) `LensSpec` with `SlotShape::Multi { token_dim: 0 }` constructs without
  panic (shape validity is enforced in PH18, not here).
- [x] fail-closed: missing id → exact `CALYX_REGISTRY_LENS_NOT_FOUND` code in
  `CalyxError`, remediation string non-empty.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the test binary output for `calyx-registry` unit tests
- **Readback:** `cargo test -p calyx-registry t01 -- --nocapture 2>&1`
- **Prove:** test output shows `LensSpec { name: "test-lens", output: Dense(4) … }`
  round-tripped to identical bytes; duplicate-register error prints
  `CALYX_REGISTRY_DUPLICATE` with a non-empty remediation; screenshot attached
  to PH17 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH17 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
