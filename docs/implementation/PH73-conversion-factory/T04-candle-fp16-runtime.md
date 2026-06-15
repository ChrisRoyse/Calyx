# PH73 T04 - Candle FP16 Runtime

## Scope

`LensRuntime::CandleLocal` now preserves Candle runtime dtype and pooling from LensForge manifests. `candle-fp16` manifests load explicit `model.safetensors`, `tokenizer.json`, and `config.json` artifacts, fail closed on frozen hash drift, and request CUDA device 0 with no silent CPU fallback when reloaded from a `LensSpec`.

## Runtime Contract

- `dtype`: `f16`, `bf16`, or `f32`; aliases `fp16`, `float16`, and `bfloat16` normalize at load time.
- `pooling`: `mean` or `cls`.
- weights hash: Calyx length-delimited SHA-256 over the full ordered manifest artifact set. The runtime verifies this same set before loading the model.
- output: dense `f32` slot vector after deterministic fixed-order pooling and declared `NormPolicy`.
- CUDA: `CandleDevicePolicy::CudaFailLoud { ordinal: 0 }` for persisted `LensSpec` reload. Building without `calyx-registry/candle-cuda` returns a loud unreachable error instead of falling back to CPU.
- aiwonder CUDA build environment: `NVCC=/usr/local/cuda-13.2/bin/nvcc`, `CUDA_HOME=/usr/local/cuda-13.2`, and `CUDA_COMPUTE_CAP=120`.
- half-precision CUDA stability: f16/bf16 Candle BERT loads clamp in-memory `layer_norm_eps` to at least `1.0e-5`; artifact bytes and hashes remain unchanged. If a half/bf16 CUDA measure emits non-finite values, the runtime replays once through an F32 model on the same CUDA device and still never falls back to CPU.

## Guards

- Missing/corrupt artifacts -> `CALYX_LENS_CONFIG_INVALID` or `CALYX_LENS_FROZEN_VIOLATION`.
- Dim mismatch -> `CALYX_LENS_DIM_MISMATCH`.
- NaN/Inf/zero norm -> `CALYX_LENS_NUMERICAL_INVARIANT`.
- Candle allocation OOM text -> `CALYX_VRAM_OOM`.
- Unsupported quantized Candle dtype such as `mxfp8` currently fails closed as config invalid; vector-side quant/compression remains PH74 T03 work.

## LensForge

`tools/lensforge/convert.py` now handles `candle-fp16` by copying local files or downloading the HuggingFace trio:

- `model.safetensors`
- `tokenizer.json`
- `config.json`

The registry includes `semantic-all-minilm-l6-v2-candle` as a real `candle-fp16` target.

## Required FSV

Source of truth is aiwonder:

- generated candle manifest under `$CALYX_HOME/lenses/<lens>/candle-fp16`
- actual artifact sha256/byte counts
- `nvidia-smi` before/load/after readback on the RTX 5090
- repeated Candle measure output showing stable norm and bounded cosine drift

Gates:

- `cargo fmt --all -- --check`
- `python3 tools/lensforge/convert.py --self-test`
- `cargo clippy -p calyx-registry --tests -- -D warnings`
- `cargo test -p calyx-registry -- --nocapture`
- candle CUDA FSV with `--features candle-cuda` on aiwonder
- tracked `.rs` line gate at or below 500 lines
