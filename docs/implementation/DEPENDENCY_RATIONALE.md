# Dependency Rationale

This note records supply-chain pins and duplicate-major decisions that are
intentional after dependency consolidation work. It is private-repo operator
state, not public API.

## Durable Serializer

`bincode` is intentionally aliased to `bincode_reloaded = 3.1.6` in the
workspace dependency table. Calyx uses it on durable Aster/Sextant/Ledger/Search
paths, so a replacement is a storage-format migration rather than a routine
crate bump. The pin remains acceptable only while:

- `Cargo.lock` pins the exact `3.1.6` checksum.
- Golden/round-trip tests continue to cover the durable byte formats.
- Any move to upstream `bincode` or vendored source is tracked as an explicit
  storage migration issue with compatibility readback.

## CUDA Bindings

Workspace CUDA code depends directly on `cudarc 0.19.8` through `calyx-forge`
and `calyx-sextant`. CUDA-enabled Candle pulls the older chain
`candle-core 0.10.2 -> candle-ug 0.10.2 -> ug-cuda 0.5.0 -> cudarc 0.17.8`.
That duplicate is transitive through Candle rather than first-party Calyx CUDA
code. Keep the direct workspace pin on `0.19.8`; revisit when Candle/ug-cuda
publishes a compatible release on the same `cudarc` major.

## Remaining Duplicate Majors

After the rand migration, remaining duplicate majors observed by Cargo metadata
and `cargo tree -d` are transitive through third-party stacks:

- `base64 0.13` via `spm_precompiled`/`tokenizers`, and `base64 0.22` via
  `ureq`/`pem`.
- `gemm 0.18` via `candle-core 0.10.2 -> candle-ug 0.10.2 -> ug 0.5.0`, and
  `gemm 0.19` via `candle-core` itself.
- `hashbrown 0.15` via the wasm toolchain (`wasmparser 0.244.0`), `hashbrown
  0.16` via `safetensors`/`sqlite-wasm-rs`, and `hashbrown 0.17` via
  `hashlink`/`indexmap`.
- `nom 7` via `spm_precompiled`, and `nom 8` via `av1-grain`.
- `getrandom 0.2/0.3/0.4` through `rand_core`, `ring`, `uuid`, `tempfile`, and
  tokenizer/image support crates.

Do not add first-party direct dependencies on older majors to paper over those
transitives. Prefer upstream crate upgrades when they converge the graph without
changing Calyx behavior.
