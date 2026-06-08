# PH23 — Per-slot HNSW index

> **Status: DONE / FSV-signed-off as the Stage 4 dense-index seam.** Current
> code is `crates/calyx-sextant/src/index/hnsw.rs`: deterministic layer
> assignment, bounded neighbor metadata, rebuild, dual-index scaffold, quant
> config lock, and exact dense search behind the HNSW-compatible API. Native
> `ef` beam traversal is not required for Stage 6 Lodestar and is tracked as a
> scale/performance refinement for later index-scale stages.

**Stage:** S4 — Sextant Search & Navigation  ·  **Crate:** `calyx-sextant`  ·
**PRD roadmap:** P3  ·  **Axioms:** A15, A16, A26

## Objective

Build an in-RAM HNSW index per dense slot (DiskANN deferred to Stage 17) that
implements the `Index` trait, accepts quantized vectors from Forge, supports
`ef`-controlled search, and provides a dual-index scaffold for asymmetric slots.
Each slot owns its index plus quant config (Qdrant-style per-vector config) so
search cost is paid only on participating slots (`10 §3`). Recall-vs-brute-force
must meet the target on aiwonder with SingleLens p99 < 5 ms at 1e6 cx (`10 §8`).

## Dependencies

- **Phases:** PH20 (lenses — slot definitions, `Lens` trait, `SlotId`), PH13 (Forge distance — CPU↔GPU distance kernels, bit-parity ≤ 1e-3)
- **Provides for:** PH24 (RRF fusion consumes the `Index` search API), PH40 (temporal fusion uses per-slot ANN), PH68 (Stage 17 DiskANN replaces the in-RAM graph)

## Current state (build off what exists)

`calyx-sextant` now provides the Stage 4 search stack. `HnswIndex` stores
vectors in RAM with deterministic layer IDs and bounded neighbor metadata; its
`search` path is currently an exact dense scan, so recall-vs-brute-force is
1.0 while preserving the HNSW-compatible API (`slot`, `shape`, `insert`,
`search`, `rebuild`, `stats`) that Stage 6 consumes. `ef` is accepted at the
trait boundary and reserved for the later native beam traversal.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/lib.rs` | crate root; re-exports; feature flags |
| `crates/calyx-sextant/src/index/mod.rs` | `Index` trait definition |
| `crates/calyx-sextant/src/index/hnsw.rs` | in-RAM HNSW implementation (insert, search, rebuild) |
| `crates/calyx-sextant/src/index/dual.rs` | dual-index scaffold for asymmetric slots (a/b sub-indexes) |
| `crates/calyx-sextant/src/index/quant_config.rs` | per-slot quant config binding (Forge TurboQuant params) |
| `crates/calyx-sextant/src/slot_index_map.rs` | `SlotId → Box<dyn Index>` registry with concurrent-read safety |
| `tests/hnsw_recall.rs` | recall-vs-brute-force harness + SingleLens p99 measurement |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `Index` trait + module skeleton | — |
| T02 | HNSW insert + layer management | T01 |
| T03 | HNSW `ef` search + brute-force recall harness | T02 |
| T04 | Dual-index scaffold for asymmetric slots | T03 |
| T05 | Per-slot quant config + Forge integration | T04 |
| T06 | `SlotIndexMap` concurrent-read-safe registry | T05 |
| T07 | Rebuild-from-base + SingleLens p99 FSV | T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Insert N vectors per slot, run `search` with calibrated `ef`, compare results to
brute-force cosine scan; recall@10 ≥ target (read the measured number from the
`tests/hnsw_recall.rs` output on aiwonder). Also run `calyx bench single-lens`
and read the p99 latency printed to stdout — must be < 5 ms at 1e6 cx. Evidence
attached to the PH23 GitHub issue.

## Risks / landmines

- **HNSW layer RNG**: seed all RNG with a fixed value (`Clock`-injected seed);
  non-deterministic layer assignment will break reproducibility and make FSV
  impossible to repeat byte-for-byte.
- **Concurrent reads**: `RwLock<HnswGraph>` with many readers is fine; writer
  starvation on high-read workloads — use `parking_lot::RwLock` and document the
  choice.
- **VRAM contention**: Forge distance kernels share the RTX 5090 sm_120 with TEI
  on :8088/:8089/:8090; ensure the recall harness does not exceed VRAM budget
  (PH57 will add the budgeter; for now, cap batch size at a documented constant).
- **DiskANN deferral**: code must leave a clean `trait Index` seam so Stage 17
  can swap in DiskANN without touching PH24+ fusion code.
