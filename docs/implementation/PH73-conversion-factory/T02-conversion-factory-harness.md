# PH73 T02 - Universal Conversion Factory Harness

## Scope

`tools/lensforge/convert.py` is the PH73 conversion entry point. It accepts a YAML list of `{hf_id, modality, formats}` entries and writes lightweight lens artifacts plus a deterministic `manifest.json` under `CALYX_HOME/lenses` by default.

The first runtime path is `onnx-int8`. The harness prefers public HuggingFace repositories that already publish `onnx/model_int8.onnx`, then records the exact model/config/tokenizer or preprocessor bytes Calyx will freeze. `onnx-fp32` is also supported for domain lenses whose dynamic int8 graph fails strict batch stability; it uses preconverted `model.onnx` when present or an Optimum feature-extraction export when `optimum-cli` is installed. Hooks for `model2vec` and `candle-fp16` are fail-closed: unsupported modality or missing local conversion dependencies produces a JSONL skip record instead of a crash.

## Manifest Contract

Each manifest includes:

- `name`, `modality`, `runtime`, `target_format`
- `dim`, output `dtype`, `pooling`, `norm`
- `weights_sha256`, the plain SHA-256 of the model weight file for direct `sha256sum` readback
- `artifact_set_sha256`, the Calyx frozen hash over the ordered runtime artifact bytes
- `files[]` with role, relative path, per-file SHA-256, and byte length
- `source_hf_id`, `license`, and `non_commercial`

`calyx-registry::lens_spec_from_manifest_path` verifies the per-file hashes, verifies the model `weights_sha256`, verifies `artifact_set_sha256`, and then builds a stable `LensSpec`. Bad JSON, missing required fields, unsupported runtime/norm, or missing files return module-local `CALYX_LENS_CONFIG_INVALID`. Byte drift returns `CALYX_LENS_FROZEN_VIOLATION`.

## CLI Readback

`calyx lens add --manifest <manifest.json>` ingests a manifest into the local lens catalog at `$CALYX_HOME/lenses/registry.json`. `calyx lens list` reads that catalog back as JSON so FSV can inspect the persisted source-of-truth file, not just the add command return value.

## Default Registry

`tools/lensforge/registry.yaml` currently covers three real aiwonder FSV models:

- semantic text: `Xenova/bge-small-en-v1.5`
- domain/scientific text: `malteos/scincl` via `onnx-fp32` (`max_batch=64`)
- non-text audio: `Xenova/wav2vec2-base-960h`

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

- artifact bytes and `manifest.json` files under `$CALYX_HOME/lenses`
- `sha256sum` of each model file equals manifest `weights_sha256`
- `calyx lens add --manifest <m>` writes `$CALYX_HOME/lenses/registry.json`
- `calyx lens list` reads back the three registered lens IDs
- edge records in `conversion-log.jsonl` prove unsupported format/modality skips without crashing
