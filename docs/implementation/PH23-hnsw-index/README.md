# PH23 — Per-slot HNSW index

> **Status: DONE / FSV-signed-off as the Stage 4 dense-index seam, with
> post-sweep hardening.** Current code is
> `crates/calyx-sextant/src/index/hnsw.rs`: deterministic layer assignment,
> bounded bidirectional neighbor metadata, native `ef` beam traversal,
> brute-force recall reference, rebuild, dual-index scaffold, quant config lock,
> and fail-closed search edges. DiskANN/SPANN scale work remains deferred to
> Stage 17, but Stage 6 no longer consumes an exact-scan placeholder.

**Stage:** S4 — Sextant Search & Navigation  ·  **Crate:** `calyx-sextant`  ·
**PRD roadmap:** P3  ·  **Axioms:** A15, A16, A26

## Objective

Build an in-RAM HNSW index per dense slot (DiskANN deferred to Stage 17) that
implements the `Index` trait, owns a local per-slot quant config, supports
`ef`-controlled search, and provides a dual-index scaffold for asymmetric slots.
Forge CPU/CUDA kernels are proven in Stage 2, but Sextant does not claim a wired
GPU HNSW or GPU quantization path until that integration exists. Each slot owns
its index plus quant config (Qdrant-style per-vector config) so
search cost is paid only on participating slots (`10 §3`). Current PH23 FSV
proves recall and p99 on a 10,000-row synthetic in-RAM HNSW corpus; the 1e6-cx
SingleLens p99 target remains a future scale/performance FSV, not a Stage 4
claim.

## Dependencies

- **Phases:** PH20 (lenses — slot definitions, `Lens` trait, `SlotId`), PH13 (Forge distance — CPU↔GPU distance kernels, bit-parity ≤ 1e-3)
- **Provides for:** PH24 (RRF fusion consumes the `Index` search API), PH40 (temporal fusion uses per-slot ANN), PH68 (Stage 17 DiskANN replaces the in-RAM graph)

## Current state (build off what exists)

`calyx-sextant` now provides the Stage 4 search stack. `HnswIndex` stores
vectors in RAM with deterministic layer IDs and bounded bidirectional neighbor
metadata; `search` performs greedy descent plus `ef`-bounded beam traversal
instead of exact dense scan. Brute force remains a reference helper for recall
readback only. Post-sweep hardening fixed the T06 registry blind spot (#282):
duplicate slot registration now fails closed with
`CALYX_SEXTANT_SLOT_ALREADY_REGISTERED` instead of replacing the existing index.
Post-sweep hardening #284 added T03-native `ef` search fail-closed edges:
`CALYX_SEXTANT_INDEX_EMPTY`, `CALYX_SEXTANT_EF_TOO_SMALL`, and
`CALYX_SEXTANT_DIM_MISMATCH`. Post-sweep hardening #299 removed CPU-self GPU
parity shims: `MaxSimIndex::cpu_gpu_delta` and
`QuantConfig::cpu_gpu_delta` now fail loud with
`CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE`, and top-level search fan-out is
documented as per-slot CPU/index-owned.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/lib.rs` | crate root; re-exports; feature flags |
| `crates/calyx-sextant/src/index/mod.rs` | `Index` trait definition |
| `crates/calyx-sextant/src/index/hnsw.rs` | in-RAM HNSW implementation (insert, search, rebuild) |
| `crates/calyx-sextant/src/index/dual.rs` | dual-index scaffold for asymmetric slots (a/b sub-indexes) |
| `crates/calyx-sextant/src/index/quant_config.rs` | local per-slot quant config (`None`, `Scalar8`, `Binary`); no wired Forge TurboQuant/GPU path yet |
| `crates/calyx-sextant/src/slot_index_map.rs` | `SlotId → Box<dyn Index>` registry with concurrent-read safety |
| `tests/hnsw_recall.rs` | recall-vs-brute-force harness + SingleLens p99 measurement |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `Index` trait + module skeleton | — |
| T02 | HNSW insert + layer management | T01 |
| T03 | HNSW `ef` search + brute-force recall harness | T02 |
| T04 | Dual-index scaffold for asymmetric slots | T03 |
| T05 | Per-slot quant config + explicit GPU-unavailable state | T04 |
| T06 | `SlotIndexMap` concurrent-read-safe registry | T05 |
| T07 | Rebuild-from-base + SingleLens p99 FSV | T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Insert N vectors per slot, run `search` with calibrated `ef`, compare results to
brute-force cosine scan; recall@10 is read from `tests/hnsw_recall.rs` output on
aiwonder. Current byte-proven PH23 evidence is `hnsw_recall_aiwonder_fsv` over
10,000 synthetic rows. A 1e6-cx SingleLens benchmark remains a separate future
scale gate and is not claimed by current PH23 artifacts.

## Risks / landmines

- **HNSW layer RNG**: seed all RNG with a fixed value (`Clock`-injected seed);
  non-deterministic layer assignment will break reproducibility and make FSV
  impossible to repeat byte-for-byte.
- **Concurrent reads**: `RwLock<HnswGraph>` with many readers is fine; writer
  starvation on high-read workloads — use `parking_lot::RwLock` and document the
  choice.
- **GPU overclaim risk**: Forge distance/quant kernels are proven in Stage 2,
  but Sextant PH23 currently uses CPU/index-owned search paths. Any Sextant
  CPU/GPU parity request must return `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE`
  until a real Forge GPU integration is wired and byte-proven on aiwonder.
- **DiskANN deferral**: code must leave a clean `trait Index` seam so Stage 17
  can swap in DiskANN without touching PH24+ fusion code.
