# PH39 · T03 — Style lens adapter (`embed_style`)

| Field | Value |
|---|---|
| **Phase** | PH39 — Identity-Locked Generation (Speaker / Style) |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/style_lens.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH19 (candle-local runtime) |
| **Axioms** | A4, A12 |
| **PRD** | `dbprdplans/09 §5b`, `05 §7` |

## Goal

Implement the style lens adapter: load a persona/writing-style model (HF
candle-local or ONNX) from the pinned checkpoint on aiwonder, expose
`embed_style(text: &str) -> Vec<f32>` returning a unit-norm style embedding,
and integrate with the `Lens` trait (PH17). The style lens must hold character
under prompt injection — a text that would break persona lands outside τ on
the style slot, enabling quarantine. The paper's result: emergent zero-shot
transfer to Golden-Age Spanish demonstrates the lens measures voice/register
generalizably (`09 §5b`).

## Build (checklist of concrete, code-level steps)

- [ ] Before implementation, select and pin the real aiwonder style model:
      source/repo, revision, model/tokenizer file hashes, input/output tensor
      names, expected embedding dim, and CPU/GPU provider plan. Placeholder
      paths are not acceptable FSV evidence.
- [ ] Define `StyleLens` struct:
      `model_path: PathBuf` (pinned at
      `/home/croyse/calyx/models/style/style-embed-v1.onnx` or candle path),
      `runtime: StyleRuntime` (for ONNX, wrap `ort::Session` in `Mutex` because
      `Session::run` requires `&mut self`; for Candle, use the PH19 frozen
      runtime handle),
      `dim: usize`,
      `lens_id: LensId`
- [ ] Implement `StyleLens::new(model_path: &Path, clock: &dyn Clock)
      -> Result<Self, WardError>` — same pattern as `SpeakerLens::new`; fail
      loud on missing model (`CALYX_WARD_MODEL_NOT_FOUND`)
- [ ] Implement `embed_style(text: &str) -> Result<Vec<f32>, WardError>`:
      - Tokenize with a bundled BPE vocab (or call PH19 tokenizer); max 512 tokens
      - Run forward pass; extract style/register embedding; L2-normalize
      - Return unit-norm vec; assert `len() == dim`
- [ ] Implement `Lens` trait (PH17) for `StyleLens`:
      `fn measure(&self, input: &Input) -> calyx_core::Result<SlotVector>`
      wrapping `embed_style`. Calyx `SlotId` values are numeric; the caller's
      panel maps the style identity slot to the lens output.
- [ ] **Frozen contract:** no mutable state after construction; `// FROZEN: A4`
- [ ] Add `embed_style_batch(texts: &[&str]) -> Result<Vec<Vec<f32>>, WardError>`
      for the injection batch test (T05)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: mock runtime returning a fixed dim-vec (seed=42); `embed_style`
      returns unit-norm; assert `norm ≈ 1.0 ± 1e-5`
- [ ] unit: same text embedded twice → identical vectors (determinism)
- [ ] unit: in-persona text vs injection text — with a mock runtime that returns
      close (0.92) vs far (0.38) vectors — assert cosine below τ=0.7 triggers
      a guard fail when passed to `guard()` on the style slot
- [ ] proptest: output always unit-norm for any non-empty ASCII text
- [ ] edge: empty text `""` → `WardError::InvalidInput` (not unit-zero vec)
- [ ] edge: text > 512 tokens → truncated silently to 512; no panic; embedding
      returned
- [ ] fail-closed: model absent → `WardError::ModelNotFound` containing
      `CALYX_WARD_MODEL_NOT_FOUND`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root containing style embedding JSON,
  mock-injection guard verdict JSON, model-missing error JSON/log, real-model
  checksum readback, and a SHA-256 manifest.
- **Readback:** run the manual FSV fixture with
  `CALYX_WARD_STYLE_LENS_FSV_DIR=$root`, then separately inspect the JSON/log
  artifacts with `xxd`, `sha256sum`, and parsed JSON. On aiwonder, the real
  style model directory must be read and hash-pinned before the fixture passes.
- **Prove:** durable readback shows norm approximately 1.0; the mock injection
  unit verdict has cos=0.38 < tau=0.7 and fails on the style slot;
  `CALYX_WARD_MODEL_NOT_FOUND` appears for a missing model; the real-model
  embedding readback has expected dimensionality. Real injection/persona
  separation is proved in #273 and must not be treated as satisfied by the mock
  unit verdict alone.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 (Forge-touching via ONNX/candle backend)
- [ ] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
