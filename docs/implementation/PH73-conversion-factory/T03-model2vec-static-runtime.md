# PH73 T03 - Model2Vec Static Lookup Runtime

## Scope

`LensRuntime::StaticLookup` runs Model2Vec-style static text embeddings without a transformer at inference time. It tokenizes text, looks up one frozen vector per token from a read-only mmap matrix, mean-pools the rows, and applies the registered `NormPolicy`.

## Runtime Artifacts

LensForge writes two frozen runtime bytes for `runtime=model2vec`:

- `embeddings.cslm` - Calyx static lookup matrix, row-major, read through `memmap2`
- `tokenizer.json` - HuggingFace tokenizer JSON consumed by the `tokenizers` crate

The matrix header is:

- magic `CXLKUP1\0`
- little-endian `u32 rows`
- little-endian `u32 dim`
- dtype byte: `1=int8`, `2=f16`, `3=f32`
- three reserved zero bytes
- little-endian `f32 scale`

The frozen contract hash is the Calyx length-delimited SHA-256 of `embeddings.cslm` plus `tokenizer.json`. Extra manifest files may be verified per-file, but they are not part of the static runtime identity.

## Behavior

- Unknown tokens are skipped deterministically.
- Empty input or all-OOV input returns a stable unit fallback vector `[1, 0, ...]`.
- Dimension mismatch fails with `CALYX_LENS_DIM_MISMATCH`.
- Matrix/tokenizer byte drift fails with `CALYX_LENS_FROZEN_VIOLATION`.
- Static lookup uses CPU/RAM only; `calyx lens explain` reports `vram_bytes=0`.

## CLI Readback

`calyx lens explain --manifest <manifest.json> --input <text> --repeat <n>` loads the static lens, measures the probe, and prints JSON with `runtime=static_lookup`, actual matrix dtype, row count, output norm, a short `first_values` vector prefix, timing, and VRAM bytes.

## Verification

Required gates:

- `python3 tools/lensforge/convert.py --self-test`
- `cargo fmt --all -- --check`
- `cargo clippy -p calyx-registry --tests -- -D warnings`
- `cargo test -p calyx-registry -- --nocapture`
- `cargo clippy -p calyx-cli --tests -- -D warnings`
- `cargo test -p calyx-cli -- --nocapture`
- source `.rs` files remain at or below 500 lines

Manual FSV source of truth:

- real `semantic-potion-base-8m/model2vec/embeddings.cslm`
- `tokenizer.json` beside it
- manifest `weights_sha256` equals `sha256sum embeddings.cslm`
- manifest `artifact_set_sha256` equals the static runtime contract bytes
- `$CALYX_HOME/lenses/registry.json` persists the `static_lookup` lens after `calyx lens add`
- `calyx lens explain` reads the actual matrix/tokenizer, emits norm near `1.0`, dtype `int8`, and `vram_bytes=0`
