# PH68 — DiskANN dense + SPANN sparse

**Stage:** S17 — Scale: DiskANN + SPANN  ·  **Crate:** `calyx-sextant`  ·
**PRD roadmap:** P10  ·  **Axioms:** A14, A15, A16, A26, A32

## Objective

Extend `calyx-sextant` from in-RAM HNSW (1e6–1e7 constellations) to disk-resident
billion-scale indexes: DiskANN on-disk graph for dense slots and SPANN
(centroids-in-RAM, posting-lists-on-NVMe) for sparse slots. Add dual-DiskANN for
asymmetric slots and the kernel-first 3-hop funnel (kernel-of-regions → region →
cx) that keeps huge-vault queries sublinear. Wire Anneal to autotune beamwidth and
posting-cutoff online. Deliver a 1e8–1e9-cx server vault that answers within the
search SLO: **KernelFirst@1e8 p99 < 25 ms** (`10 §8`).

> **SCALE HONESTY (binding, `17 §3.4`):** Billion-scale is a **SERVER** target,
> running on aiwonder's `hotpool` NVMe + HDD (RTX 5090, 1.5 TB hot). It is
> **NEVER** a laptop or embedded promise. Embedded vaults top out at 1e6–1e7 with
> in-RAM HNSW. Do not add any API or documentation that implies billion-scale on
> a consumer device.

## Dependencies

- **Phases:** PH23 (per-slot HNSW — provides the in-RAM ANN baseline, index trait,
  and slot-level index lifecycle that DiskANN replaces at server scale),
  PH25 (inverted index — provides the in-RAM sparse posting lists that SPANN
  replaces at server scale),
  PH46 (Anneal autotune loops — provides the Anneal hook DiskANN/SPANN
  beamwidth/cutoff autotune registers into)
- **Provides for:** PH70 (intelligence validation on real billion-scale corpora),
  PH72 (streaming ingest into large server vaults)

## Current state (build off what exists)

`calyx-sextant` has per-slot HNSW (PH23), inverted index (PH25), DiskANN graph
format/search, token DiskANN + segmented MaxSim, and concat cross-term DiskANN.
The remaining PH68 work is SPANN, dual DiskANN, kernel-first routing, and the
full 1e8+ phase-exit soak. The vault physical layout (`04 §3`) reserves
`idx/slot_NN.ann/` for dense DiskANN, `idx/slot_NN.token.ann/` for multi-vector
token DiskANN, `idx/xterm.concat.ann/` for materialized concat xterm DiskANN,
and `idx/slot_NN.sparse/` for SPANN.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/index/diskann/graph.rs` | On-disk graph format: node layout (vector + neighbor list co-located for I/O locality), page-aligned blocks, mmap reader, graph builder (Vamana-style greedy insert + prune) |
| `crates/calyx-sextant/src/index/diskann/search.rs` | Beam search over on-disk graph: beamwidth-tuned BFS, I/O prefetch, raw-f32 rescore from cold sidecar; `DiskAnnSearch` impl of `SlotIndex` |
| `crates/calyx-sextant/src/index/diskann/token.rs` | Token DiskANN over flattened multi-vector tokens; candidate token hits are grouped by document and reranked with segmented MaxSim from raw token bytes |
| `crates/calyx-sextant/src/index/diskann/token_sidecar.rs` | Token DiskANN sidecars: `docs.cdt`, `token_docs.u32`, and `tokens.f32` with byte-readable document segments and token-to-document ordinals |
| `crates/calyx-sextant/src/index/diskann/concat.rs` | DiskANN over materialized `xterm` Concat rows, with `keys.cdx` preserving `(CxId, slot_a, slot_b, Concat)` identity for hits |
| `crates/calyx-sextant/src/index/diskann/dual.rs` | Dual-DiskANN for asymmetric slots: `asym_a` and `asym_b` graph pair, directional dispatch, dual-beam search, merge of asymmetric hit lists |
| `crates/calyx-sextant/src/index/spann/centroids.rs` | SPANN centroid index: k-means clustering into centroids (held in RAM), centroid ANN (tiny HNSW), centroid-to-posting-list map persisted to disk |
| `crates/calyx-sextant/src/index/spann/posting.rs` | SPANN posting lists on NVMe: varint+zstd block encoding, page-aligned I/O, append writer, random-access reader; eviction when RAM budget exceeded |
| `crates/calyx-sextant/src/index/funnel.rs` | Kernel-first 3-hop funnel for huge vaults (1e8+): kernel-of-regions → region ANN → cx ANN; `KernelFirstSearch` dispatch over the three tiers; `KernelFirst@1e8` p99 < 25 ms SLO |
| `crates/calyx-sextant/src/index/autotune.rs` | Anneal autotune hook: `BwPostcutoffTuner` observes p99 latency + recall@10; adjusts beamwidth and posting-cutoff via Anneal bandit; tripwire if recall drops below floor |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | DiskANN on-disk graph format + builder | — |
| T02 | DiskANN beam search + raw-f32 rescore | T01 |
| T03 | SPANN centroids-in-RAM + posting-lists-on-NVMe | — |
| T04 | Dual-DiskANN for asymmetric slots | T01, T02 |
| T05 | Kernel-first 3-hop funnel for huge vaults (1e8+) | T02, T03 |
| T06 | Anneal autotune of beamwidth/posting-cutoff + 1e8-cx SLO soak FSV | T02, T03, T05 |
| Gap #604 | Token DiskANN + MaxSim and concat xterm DiskANN | T01, T02 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

On aiwonder (`hotpool` NVMe, `/zfs/hot/calyx/`):

1. Build a synthetic 1e8-cx server vault with DiskANN graphs in
   `idx/slot_NN.ann/` and SPANN lists in `idx/slot_NN.sparse/`. Verify physical
   presence: `ls -lh /zfs/hot/calyx/<vault>/idx/slot_00.ann/` and
   `ls -lh /zfs/hot/calyx/<vault>/idx/slot_00.sparse/` — both directories must
   contain byte-populated files (non-zero size).
2. Run `calyx bench search --vault <vault> --strategy KernelFirst --n 1000
   --report p99` on aiwonder. Observed p99 must be **< 25 ms** (the
   `KernelFirst@1e8 p99 < 25 ms` SLO from `10 §8`).
3. Verify beamwidth and posting-cutoff were autotuned: `calyx anneal status
   --vault <vault> --tuner bw_postcutoff` prints non-default values and shows at
   least one Ledger-logged autotune event.
4. For dual-DiskANN: a vault with an asymmetric slot has both `asym_a` and
   `asym_b` graph directories populated on disk.
5. For multi and concat server indexes: a synthetic >1e6-row issue #604 evidence
   root contains populated `idx/slot_00.token.ann/{graph.cda,docs.cdt,token_docs.u32,tokens.f32}`
   and `idx/xterm.concat.ann/{graph.cda,keys.cdx}`, with recall checked against
   brute-force and byte headers read back with `xxd`/`stat` on aiwonder.
6. All byte-level evidence (file sizes, p99 measurement, autotune log) attached to
   the PH68 GitHub issue.

## Risks / landmines

- **`hotpool` has no redundancy.** ANN graphs and posting lists are rebuildable
  from base+slots (A16 / `04 §7`); a corrupt index triggers a `degraded` flag and
  background rebuild, never data loss. Do not claim durability for the index files
  themselves — only for the base CF + WAL.
- **I/O amplification on random beams.** DiskANN beam search issues one random
  I/O per beam step; beamwidth of 64 on a cold cache = 64 seeks per query. Size
  the page-aligned block and prefetch depth so the NVMe queue depth is saturated,
  not serialized. Profile on `hotpool` before declaring the SLO met.
- **RAM footprint of SPANN centroids.** Centroid count (typically √N) for 1e9 cx
  with 15 slots must fit within the VRAM+RAM budget alongside active TEI and Forge
  matmul working sets. Measure with `calyx bench memory --vault <vault>` before
  commit.
- **Anneal oscillation.** Beamwidth/posting-cutoff autotune must have a tripwire
  (A14): if recall@10 drops below the floor, revert immediately and Ledger-log the
  revert. The Anneal soak in T06 must prove no oscillation over ≥1e5 queries.
- **`EXDEV` on ZFS temp writes.** Any tmp file produced during graph build or
  posting-list compaction must be staged inside the target dataset
  (`/zfs/hot/calyx/<vault>/`), never in `/tmp` or another dataset
  (`04 §3` / `aiwonder-system.md`).
- **GPU contention.** Distance recomputation during rescore uses Forge CUDA. The
  VRAM budgeter (PH57) must gate these dispatches so they coexist with the 3
  resident TEI containers on the RTX 5090.
- **Billion-scale embedded** is explicitly out of scope (`17 §3.4`). If any code
  path or test parametrizes over 1e8+ cx without an `#[cfg(server)]` or
  `#[ignore = "server-only"]` annotation, it is a bug.
