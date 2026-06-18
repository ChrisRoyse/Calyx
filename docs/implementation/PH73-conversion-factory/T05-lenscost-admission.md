# PH73 T05 - LensCost Admission

## Scope

Converted lenses now carry measured resource cost and deterministic placement. `SlotResource` persists a `LensCost` plus `Placement` on every slot, and lens catalog entries record the same cost/placement when `calyx lens add` admits a manifest.

## Resource Contract

- `LensCost.total_ms` and `ms_per_input`: profile wall-clock over the probe path.
- `LensCost.vram_bytes`: lens-local GPU bytes, not total device usage, so resident TEI containers are not double-counted.
- `LensCost.ram_bytes`: resident CPU artifact/probe footprint.
- `LensCost.batch_ceiling`: derived from the measured `ms/input` against a 1s admission envelope.
- `Placement`: `cpu` or `gpu`, chosen from runtime class, cost, CPU pool pressure, and post-TEI VRAM headroom.

## Admission Rules

- Algorithmic zero-cost lenses always admit as CPU.
- Static/model2vec and external command runtimes prefer CPU and are checked against the CPU resident pool.
- ONNX prefers GPU and fails closed when the post-TEI VRAM cap is exhausted; CPU execution must be explicit rather than a hidden fallback.
- Candle-local and TEI HTTP runtimes are GPU placements; Candle-local refuses when it cannot fit.
- Forge lens admission subtracts the TEI reservation before reserving GPU bytes and fails closed with `CALYX_VRAM_BUDGET_EXCEEDED` when a GPU runtime cannot fit.
- The CPU pool is LRU-bounded by resident count and RAM bytes; cold entries are evicted before a CPU lens is refused.

## CLI Surfaces

- `calyx lens add --manifest <path> --home <dir>` persists `cost` and `placement` in `<home>/lenses/registry.json`.
- `calyx panel status --home <dir>` reads the catalog source-of-truth and prints per-lens cost, placement, RAM/VRAM MB, and totals.
- `calyx assay corpus-build --cost-override-json <json>` may supply measured resident resources, but GPU runtimes cannot be overridden to CPU; that fails with `CALYX_FSV_ASSAY_CORPUS_BUILD_GPU_OVERRIDE_PLACEMENT`.
- Existing catalogs without cost/placement deserialize with CPU/zero defaults so old bytes stay readable.

## Required FSV

Source of truth is aiwonder:

- `<home>/lenses/registry.json` before and after lens admission.
- `calyx panel status --home <home>` readback for at least ten admitted converted lenses.
- `nvidia-smi` before/after readback showing resident VRAM remains under the cap.
- process RSS readback during a 100000-measure soak.
- three edge readbacks: zero-cost algorithmic admission, GPU cap refusal or GPU override downgrade refusal, and CPU pool LRU eviction/oversized refusal.

Gates:

- `cargo fmt --all -- --check`
- `scripts/linecount.sh`
- `cargo check --workspace`
- `cargo clippy --workspace --tests -- -D warnings`
- `cargo test --workspace -- --nocapture`
- targeted CUDA/VRAM FSV where GPU runtimes are exercised on aiwonder
