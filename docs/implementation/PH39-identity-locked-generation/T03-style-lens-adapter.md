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

**Status:** DONE / FSV #271. Durable aiwonder evidence:
`/home/croyse/calyx/data/fsv-issue271-style-lens-20260609-a43e546-ort126-sm120`.

## Pinned aiwonder style model

Selected model: `AnnaWegmann/Style-Embedding`, revision
`d7d0f5ca829316a8f5695e49dfce80b86db5e76c`. This is the published
content-independent style representation model (RoBERTa + sentence-transformer
mean pooling), not a generic semantic-only placeholder. Runtime files are pinned
on aiwonder under `/home/croyse/calyx/models/style/`.

| Artifact | SHA-256 |
|---|---|
| `style-embed-v1.onnx` | `fc3c80ead2e4ceef693fa67756f2e0f920fee7df326a565286b34d68d7a170af` |
| `tokenizer.json` | `82139106e603ee4e1d5bc99d056ccbed5a92bc24848b1b5a7137c26e00d0dbf6` |
| `config.json` | `2ed20b6297d7f5652f3a381221ce42cc592b7ebde6b61e3604df385904224311` |
| `tokenizer_config.json` | `72824f8b68a49929f38b29c0d2e6f7664ea68846b5447791fc83bf1ad1778127` |
| `vocab.json` | `ed19656ea1707df69134c4af35c8ceda2cc9860bf2c3495026153a133670ab5e` |
| `merges.txt` | `fe36cab26d4f4421ed725e10a2e9ddb7f799449c603a96e7f29b5a3c82a95862` |
| `special_tokens_map.json` | `378eb3bf733eb16e65792d7e3fda5b8a4631387ca04d2015199c4d4f22ae554d` |

Source weight readback: `source/pytorch_model.bin` SHA-256
`3186cd80660a7169a911bace4d54416cf5771a319a22f84c3a79a961ecb0c6f5`.
Runtime tensor contract: inputs `input_ids:int64[batch,sequence]` and
`attention_mask:int64[batch,sequence]`; output
`last_hidden_state:f32[batch,sequence,768]`; adapter applies attention-mask mean
pooling then L2 normalization. Max tokens: 512. Provider plan: CPU explicit for
deterministic fallback/readback; CUDA fail-loud through the custom aiwonder ORT
sm_120 provider with no silent CPU fallback. The durable source manifest is
`/home/croyse/calyx/models/style/SOURCE.json`.

## Build (checklist of concrete, code-level steps)

- [x] Before implementation, select and pin the real aiwonder style model:
      source/repo, revision, model/tokenizer file hashes, input/output tensor
      names, expected embedding dim, and CPU/GPU provider plan. Placeholder
      paths are not acceptable FSV evidence.
- [x] Define `StyleLens` struct:
      `model_path: PathBuf`, `tokenizer_path: PathBuf`, frozen `lens_id`, dim,
      and a backend seam; production backend wraps an `ort::Session` in
      `Mutex` because `Session::run` requires mutable access.
- [x] Implement `StyleLens::new(model_path: &Path) -> Result<Self, WardError>`
      — same pattern as `SpeakerLens::new`; fail loud on missing model or
      tokenizer (`CALYX_WARD_MODEL_NOT_FOUND`).
- [x] Implement `embed_style(text: &str) -> Result<Vec<f32>, WardError>`:
      - Tokenize with a bundled BPE vocab (or call PH19 tokenizer); max 512 tokens
      - Run forward pass; extract style/register embedding; L2-normalize
      - Return unit-norm vec; assert `len() == dim`
- [x] Implement `Lens` trait (PH17) for `StyleLens`:
      `fn measure(&self, input: &Input) -> calyx_core::Result<SlotVector>`
      wrapping `embed_style`. Calyx `SlotId` values are numeric; the caller's
      panel maps the style identity slot to the lens output.
- [x] **Frozen contract:** no mutable state after construction except the
      runtime session mutex required by ORT.
- [x] Add `embed_style_batch(texts: &[&str]) -> Result<Vec<Vec<f32>>, WardError>`
      for the injection batch test (T05)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: mock runtime returning a fixed dim-vec (seed=42); `embed_style`
      returns unit-norm; assert `norm ≈ 1.0 ± 1e-5`
- [x] unit: same text embedded twice → identical vectors (determinism)
- [x] unit: in-persona text vs injection text — with a mock runtime that returns
      close (0.92) vs far (0.38) vectors — assert cosine below τ=0.7 triggers
      a guard fail when passed to `guard()` on the style slot
- [x] proptest: output always unit-norm for any non-empty ASCII text
- [x] edge: empty text `""` → `WardError::InvalidInput` (not unit-zero vec)
- [x] edge: text > 512 tokens → truncated silently to 512; no panic; embedding
      returned
- [x] fail-closed: model absent → `WardError::ModelNotFound` containing
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

### #271 readback summary

FSV root:
`/home/croyse/calyx/data/fsv-issue271-style-lens-20260609-a43e546-ort126-sm120`.
The root was absent before the trigger. The ignored fixture wrote
`model-readback.json`, `style-embedding.json`, `norm-determinism.json`,
`mock-injection-guard-verdict.json`, `model-missing-error.json`,
`issue271-fsv.log`, `SHA256SUMS.txt`, and `SHA256SUMS.full.txt`. Separate
readback confirmed:

- model SHA-256:
  `fc3c80ead2e4ceef693fa67756f2e0f920fee7df326a565286b34d68d7a170af`
- vocab SHA-256:
  `82139106e603ee4e1d5bc99d056ccbed5a92bc24848b1b5a7137c26e00d0dbf6`
- source revision: `d7d0f5ca829316a8f5695e49dfce80b86db5e76c`
- input tensors: `input_ids`, `attention_mask`; output tensor:
  `last_hidden_state`
- lens id: `3a9aac62c199488e6ef9f233b54ef816`
- embedding dim: 768; norm: `1.0000003576278687`
- deterministic max abs diff: `0.0`
- CPU/CUDA max abs diff: `0.00016807019710540771`
- mock injection style slot: cos `0.3799999952316284` < tau
  `0.699999988079071`, `overall_pass=false`, action `Quarantine`
- missing model error: `CALYX_WARD_MODEL_NOT_FOUND`

Full evidence manifest SHA-256:
`f505efa94e13745fa6cc3068b531efefdfe047ecfcff83fba7cae196800bf452`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 (Forge-touching via ONNX/candle backend)
- [x] FSV evidence (readback output / screenshot) attached to the PH39 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
