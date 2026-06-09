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

## Status

DONE / FSV-signed-off at implementation commits `06a52d7` and `f91e5e8`, with
post-stage-5 contract/registry refresh `d4155e0`. Durable aiwonder evidence:
`/home/croyse/calyx/data/fsv-issue270-speaker-lens-20260609-ef729f8-ort126-sm120`.

Readback summary:
- WavLM model SHA-256:
  `22a38bdd854a11db171357cb997156511697d2f2c621d1262c82ba91b873d08b`.
- Custom aiwonder ORT CUDA provider SHA-256:
  `36172645abd04656263112e557ce8a150ce827ff6391a0027a151ffa5a09ad71`.
- `input_names == ["input_values"]`; `output_names == ["logits", "embeddings"]`.
- speaker embedding dim 512; norm `0.9999998211860657`.
- duplicate CPU max abs diff `0.0`; CPU/CUDA max abs diff
  `0.0009525101631879807`.
- missing model path returns `CALYX_WARD_MODEL_NOT_FOUND`.

## Goal

Implement the WavLM speaker lens adapter: load the WavLM ONNX model from the
pinned checkpoint on aiwonder, expose `embed_speaker(audio_pcm: &[f32]) ->
Vec<f32>` returning a unit-norm speaker embedding, and integrate with the
`Lens` trait (PH17). The target is 0.961 mean WavLM cosine in-region on
matched-speaker pairs (`09 §5b`). Lens weights are frozen (A4); the adapter
must not mutate the model.

## Build (checklist of concrete, code-level steps)

- [x] Define `SpeakerLens` struct:
      `model_path: PathBuf` (pinned at
      `/home/croyse/calyx/models/wavlm/wavlm-base-plus-sv.onnx`),
      `backend: Box<dyn SpeakerEmbeddingBackend>`; the production ONNX backend
      owns ORT's required `Mutex<Session>`,
      `dim: usize` (pinned WavLM-base-plus ONNX `embeddings` output dim = 512),
      `lens_id: LensId` (content-addressed from model hash, PH18 pattern)
- [x] Implement `SpeakerLens::new(model_path: &Path)
      -> Result<Self, WardError>` plus explicit CPU construction for local FSV:
      - Load ONNX session through the direct `ort` dependency in `calyx-ward`;
        fail loud if model absent
        (`WardError::ModelNotFound { path }` → `CALYX_WARD_MODEL_NOT_FOUND`)
      - Verify model `embeddings` output dim == 512; fail closed if mismatch.
        The post-sweep aiwonder readback corrected the old 256-dim assumption:
        the pinned ONNX model exposes the real speaker x-vector dim.
      - `lens_id = LensId::from_parts(...)` using the model SHA-256, pinned
        source revision, and dense audio output shape; do not invent a
        nonexistent `LensId::from_file_hash` helper
- [x] Implement `embed_speaker(audio_pcm: &[f32], sample_rate: u32) -> Vec<f32>`:
      - Resample to 16 kHz if needed (simple linear interp; not a quality path —
        correctness only for the FSV test set)
      - Run ONNX session forward pass; extract speaker embedding tensor
      - L2-normalize to unit norm; assert `len() == 512`
      - Return the embedding
- [x] Implement `Lens` trait (PH17) for `SpeakerLens`:
      `fn measure(&self, input: &Input) -> calyx_core::Result<SlotVector>`
      wrapping `embed_speaker`. The generic `Input` contract for this lens is
      little-endian f32 PCM already normalized to `WAVLM_SAMPLE_RATE` (16 kHz);
      callers with arbitrary-rate audio must use `embed_speaker(audio, sample_rate)`
      or resample before constructing `Input`. Calyx `SlotId` values are
      numeric; the caller's panel maps the speaker identity slot to the lens
      output.
- [x] **Frozen contract:** `SpeakerLens` stores immutable model metadata plus a
      backend trait object; production runtime state is limited to ORT's required
      `Mutex<Session>`, with no mutable model/config surface after construction.
- [x] `lens_id()` returns the content-addressed ID (no re-hash at call time)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: mock ONNX session returning a fixed 512-dim vector (seed=42);
      `embed_speaker` returns unit-norm vec; assert `norm ≈ 1.0 ± 1e-5`
- [x] unit: two identical audio buffers → identical embeddings (deterministic)
- [x] unit: two zero-padded buffers of different length but same speech segment
      → cosine similarity ≥ 0.99 (length-invariance for padding)
- [x] proptest: output vector always unit-norm for any non-zero input
- [x] edge: empty audio `&[]` → `WardError::InvalidInput` (not panic)
- [x] edge: malformed PCM `Input` bytes not divisible by 4 ->
      `WardError::InvalidInput` through `Lens::measure`
- [x] edge: model file absent → `WardError::ModelNotFound` containing
      `CALYX_WARD_MODEL_NOT_FOUND`; the path is in the error message
- [x] fail-closed: ONNX session returns wrong dim (128) → `WardError` on
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
  real-model 1-second silence embedding with expected dimensionality. Semantic
  same-speaker/cross-speaker proof belongs to #274 unless this card also
  captures a small same/different speaker sanity fixture.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden speaker-embedding set (ONNX on
      CPU vs GPU — requires an ORT CUDA provider built for aiwonder `sm_120`;
      the downloaded Pyke/`ort` provider that only contains kernels through
      `compute_90a` is not acceptable evidence)
- [x] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
