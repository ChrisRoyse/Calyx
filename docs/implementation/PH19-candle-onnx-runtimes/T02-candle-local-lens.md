# PH19 · T02 — CandleLocalLens runtime

| Field | Value |
|---|---|
| **Phase** | PH19 — candle-local + onnx runtimes |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/runtime/candle.rs` (≤500) |
| **Depends on** | T01 (this phase), PH18 T05 (frozen contract guards) |
| **Axioms** | A4 |
| **PRD** | `dbprdplans/05 §2`, `13_STAGE3_REGISTRY.md §PH19` |

## Goal

Implement `CandleLocalLens` that loads a BERT-family embedder from
`CALYX_HOME/.hf-cache`, runs the forward pass via `candle-transformers` on
the sm_120 CUDA device (or CPU fallback), L2-normalizes the output, and
returns a `SlotVector::Dense`. The lens must pass the full frozen contract
from PH18.

## Build (checklist of concrete, code-level steps)

- [x] `CandleLocalLens` struct: `id: LensId`, `model_path: PathBuf`,
  `dim: u32`, `modality: Modality`, `device: candle_core::Device`
  (`Device::Cuda(0)` on aiwonder, `Device::Cpu` in unit tests).
- [x] `CandleLocalLens::load(spec: &LensSpec, cache: &HfCacheConfig) -> Result<Self>`:
  - resolve model dir via `hf_cache::resolve`.
  - load `config.json` + tokenizer + `model.safetensors` (use
    `candle_nn::VarBuilder::from_mmaped_safetensors`).
  - build a `BertModel` (or appropriate candle-transformers model).
  - compute `weights_sha256` of the safetensors bytes with `sha2::Sha256`.
  - call `check_weights_sha256(computed, spec)` → fail if mismatch.
- [x] `measure(&self, input: &Input) -> Result<SlotVector>`:
  - tokenize `input.bytes` (UTF-8 text) with the loaded tokenizer.
  - run forward pass → pooled output tensor `[1, dim]`.
  - extract `Vec<f32>` from tensor.
  - L2-normalize (use Forge `normalize` from PH12 or inline for now).
  - call `check_output(vec, spec)` (dim + finite + norm guards).
  - return `SlotVector::Dense { dim, data }`.
- [x] `#[cfg(feature = "candle-cuda")]` gate for CUDA device path;
  compile/test without it in CI.
- [x] Integration test `candle_gte_produces_valid_vector` (`#[ignore]`):
  load a small model from `.hf-cache`; measure `"hello world"` → unit-norm
  finite Dense vector; print norm.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit (mock weights): construct `CandleLocalLens` with a tiny hand-built
  candle model (2-layer, dim=4); measure `b"test"` → `SlotVector::Dense { dim: 4 }`,
  all values finite, norm ≈ 1.0.
- [x] unit: wrong `weights_sha256` in spec → `CALYX_LENS_FROZEN_VIOLATION`
  at load time.
- [x] edge (≥3): (1) empty input bytes → `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`
  (tokenizer produces empty sequence); (2) input exceeding max sequence length
  → truncated without panic; (3) CPU device fallback works when CUDA absent.
- [x] fail-closed: safetensors file missing → `CALYX_REGISTRY_RUNTIME_UNAVAILABLE`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** integration test output + `$CALYX_HOME/.hf-cache/<model>/model.safetensors`
  existence on aiwonder
- **Readback:** `cargo test -p calyx-registry candle -- --include-ignored --nocapture 2>&1`
- **Prove:** output shows `CandleLocalLens dim=768 norm=1.000±0.0001`;
  `ls $CALYX_HOME/.hf-cache/<model>/` printed showing safetensors file;
  attached to PH19 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] [Forge-touching] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [x] FSV evidence (readback output / screenshot) attached to the PH19 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
