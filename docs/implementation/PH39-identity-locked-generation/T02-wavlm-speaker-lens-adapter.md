# PH39 · T02 — WavLM speaker lens adapter (`embed_speaker`)

| Field | Value |
|---|---|
| **Phase** | PH39 — Identity-Locked Generation (Speaker / Style) |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/speaker_lens.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH19 (ONNX runtime) |
| **Axioms** | A12, A4 |
| **PRD** | `dbprdplans/09 §5b`, `05 §7` |

## Goal

Implement the WavLM speaker lens adapter: load the WavLM ONNX model from the
pinned checkpoint on aiwonder, expose `embed_speaker(audio_pcm: &[f32]) ->
Vec<f32>` returning a unit-norm speaker embedding, and integrate with the
`Lens` trait (PH17). The target is 0.961 mean WavLM cosine in-region on
matched-speaker pairs (`09 §5b`). Lens weights are frozen (A4); the adapter
must not mutate the model.

## Build (checklist of concrete, code-level steps)

- [ ] Define `SpeakerLens` struct:
      `model_path: PathBuf` (pinned at
      `/home/croyse/calyx/models/wavlm/wavlm-base-plus-sv.onnx`),
      `session: Arc<ort::Session>` (ONNX Runtime via PH19 onnx runtime),
      `dim: usize` (WavLM-base-plus speaker-embedding dim = 256),
      `lens_id: LensId` (content-addressed from model hash, PH18 pattern)
- [ ] Implement `SpeakerLens::new(model_path: &Path, clock: &dyn Clock)
      -> Result<Self, WardError>`:
      - Load ONNX session via PH19 runtime; fail loud if model absent
        (`WardError::ModelNotFound { path }` → `CALYX_WARD_MODEL_NOT_FOUND`)
      - Verify model output dim == 256; fail closed if mismatch
      - `lens_id = LensId::from_file_hash(model_path)` (SHA-256 of file bytes)
- [ ] Implement `embed_speaker(audio_pcm: &[f32], sample_rate: u32) -> Vec<f32>`:
      - Resample to 16 kHz if needed (simple linear interp; not a quality path —
        correctness only for the FSV test set)
      - Run ONNX session forward pass; extract speaker embedding tensor
      - L2-normalize to unit norm; assert `len() == 256`
      - Return the embedding
- [ ] Implement `Lens` trait (PH17) for `SpeakerLens`:
      `fn embed(&self, input: &LensInput) -> Result<Vec<f32>, LensError>`
      wrapping `embed_speaker`; slot = `SlotId("speaker")`
- [ ] **Frozen contract:** `SpeakerLens` fields are all `Arc` or `PathBuf`;
      no mutable state after construction; assert with `// FROZEN: A4` comment
- [ ] `lens_id()` returns the content-addressed ID (no re-hash at call time)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: mock ONNX session returning a fixed 256-dim vector (seed=42);
      `embed_speaker` returns unit-norm vec; assert `norm ≈ 1.0 ± 1e-5`
- [ ] unit: two identical audio buffers → identical embeddings (deterministic)
- [ ] unit: two zero-padded buffers of different length but same speech segment
      → cosine similarity ≥ 0.99 (length-invariance for padding)
- [ ] proptest: output vector always unit-norm for any non-zero input
- [ ] edge: empty audio `&[]` → `WardError::InvalidInput` (not panic)
- [ ] edge: model file absent → `WardError::ModelNotFound` containing
      `CALYX_WARD_MODEL_NOT_FOUND`; the path is in the error message
- [ ] fail-closed: ONNX session returns wrong dim (128) → `WardError` on
      construction; `embed_speaker` never called with a bad session

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root containing speaker embedding JSON,
  norm/determinism JSON, model-missing error JSON/log, real-model checksum
  readback, and a SHA-256 manifest.
- **Readback:** run the manual FSV fixture with
  `CALYX_WARD_SPEAKER_LENS_FSV_DIR=$root`, then separately inspect the JSON/log
  artifacts with `xxd`, `sha256sum`, and parsed JSON. On aiwonder, the real
  WavLM model directory must be read and hash-pinned before the fixture passes.
- **Prove:** durable readback shows norm approximately 1.0, deterministic
  duplicate embeddings, `CALYX_WARD_MODEL_NOT_FOUND` for a missing model, and a
  real-model 1-second silence embedding with expected dimensionality.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden speaker-embedding set (ONNX on
      CPU vs GPU — PH19 parity contract)
- [ ] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
