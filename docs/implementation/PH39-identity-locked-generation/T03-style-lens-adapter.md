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

- [ ] Define `StyleLens` struct:
      `model_path: PathBuf` (pinned at
      `/home/croyse/calyx/models/style/style-embed-v1.onnx` or candle path),
      `runtime: StyleRuntime` (enum: `Onnx(Arc<ort::Session>)` |
      `Candle(Arc<candle_nn::VarBuilder>)` — use whatever PH19 exposes),
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
      `fn embed(&self, input: &LensInput) -> Result<Vec<f32>, LensError>`
      wrapping `embed_style`; slot = `SlotId("style")`
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

- **SoT:** test stdout showing style embedding norm and mock injection test
- **Readback:**
  `cargo test -p calyx-ward style_lens -- --nocapture 2>&1 | grep -E "norm|injection|CALYX_WARD"`
- **Prove:** `norm ≈ 1.0`; the mock injection test shows cos=0.38 < τ=0.7 →
  guard fails on style slot; `ModelNotFound` shows `CALYX_WARD_MODEL_NOT_FOUND`;
  on aiwonder with real model: embed "Write in the style of Cervantes" → print
  first 8 dims of embedding

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 (Forge-touching via ONNX/candle backend)
- [ ] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
