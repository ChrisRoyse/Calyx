# PH73 T01 - Arbitrary Custom ONNX Lens Registration

## Scope

`calyx-registry` must register text lenses from explicit local ONNX artifacts, not only from `fastembed::EmbeddingModel`.

The runtime accepts:

- `model.onnx` or `model_int8.onnx`
- `tokenizer.json`
- `config.json`
- declared pooling: `mean`, `cls`, or `last_token`
- declared normalization: `NormPolicy::L2`/`Unit` for unit output, or finite-only policies for unnormalized vectors

The frozen contract is content-addressed from the actual model/tokenizer/config bytes. Any byte drift must create a distinct `LensId` unless the caller supplied the prior frozen hash, in which case registration fails with `CALYX_LENS_FROZEN_VIOLATION`.

## Runtime Contract

- `OnnxLens::from_files(OnnxFileSpec)` loads a direct `ort::Session`.
- `OnnxFileSpec::from_lens_spec` rebuilds a persisted `LensRuntime::Onnx { model_id, files }` using the stored hash and shape as fail-closed checks.
- Output dimension is read from the ONNX session output shape.
- Declared `SlotShape` mismatch returns `CALYX_LENS_DIM_MISMATCH`.
- Missing tokenizer/config/model, bad config JSON, unsupported pooling, unsupported input names, or ORT load failures return module-local `CALYX_LENS_CONFIG_INVALID`.
- `OnnxProviderPolicy::CudaFailLoud` and `CpuExplicit` are shared with the fastembed ONNX path.

## Verification

Required gates:

- `cargo check -p calyx-registry --tests`
- `cargo clippy -p calyx-registry --tests -- -D warnings`
- `cargo test -p calyx-registry -- --nocapture`
- source `.rs` files remain at or below 500 lines

Manual FSV source of truth:

- explicit ONNX/tokenizer/config bytes on aiwonder
- frozen `weights_sha256` read from the registered contract
- measured vector shape and norm read after inference
- `LensSpec` reload from `LensRuntime::Onnx` proving the runtime is not fastembed enum-bound

The happy path must register and measure a model that is not selected through `fastembed::EmbeddingModel`. Edge readbacks must cover missing tokenizer, declared dimension mismatch, hash drift/frozen violation, and non-finite output.
