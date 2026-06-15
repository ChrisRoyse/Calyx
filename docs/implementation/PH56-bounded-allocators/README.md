# PH56 — Bounded caches/queues/memtables + arenas/pools

**Stage:** S13 — Resource, GC & Reliability Hardening  ·  **Crate:** `calyx-aster`, `calyx-core`  ·
**PRD roadmap:** RESOURCE  ·  **Axioms:** A26

## Objective

Every allocation in Calyx has an owner and a hard bound (A26). This phase wires arena/bump
allocators for per-request and per-microbatch transient work, slab/pool allocators for
fixed-size hot objects (vector blocks, ANN nodes, GPU staging buffers), a byte-capped bounded
memtable with backpressure, LRU+TTL byte-capped caches for every cached artifact, and
mmap-backed cold/columnar access so the vault never lives fully in heap. Fail closed
(reject/spill via `CALYX_BACKPRESSURE`/`CALYX_DISK_PRESSURE`) before OOM, never crash.
Cross-cutting hardening from Stage 1, finalized here.

## Dependencies

- **Phases:** PH08 (MVCC sequence numbers + snapshot reads — memtable bounded around MVCC
  writes; bounded caches protect snapshot state)
- **Provides for:** PH57 (VRAM budgeter builds on the same bounded-allocation discipline),
  PH58 (GC reclaimers manage the objects allocated here), PH59 (soak drives these bounds)

## Current state (build off what exists)

`calyx-aster` has the Stage 1 storage core (WAL, memtable, SSTable, MVCC,
manifest, compaction, and vault CRUD). None of these modules yet enforce the
hard allocation discipline PH56 introduces — bounded memtables are partial,
caches are absent or local maps without a shared LRU/TTL policy, and no
arena/slab allocator module exists. `calyx-core` has the shared IDs, enums,
error catalog, traits, and engine data types; PH56 adds allocator/cache
primitives there. Single-NVMe `hotpool` has no redundancy; buildup = an outage.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-core/src/alloc/arena.rs` | Arena/bump allocator; per-request and per-microbatch; O(1) reset |
| `crates/calyx-core/src/alloc/slab.rs` | Slab/pool allocator for fixed-size objects (vector blocks, ANN nodes, GPU staging) |
| `crates/calyx-core/src/alloc/mod.rs` | Re-exports + `AllocStats` struct |
| `crates/calyx-core/src/cache/lru_ttl.rs` | Generic LRU+TTL cache with hard byte cap; `CALYX_CACHE_EVICTED` metric hook |
| `crates/calyx-aster/src/memtable/bounded.rs` | Byte-capped memtable; backpressure on high-water; `CALYX_BACKPRESSURE` on full |
| `crates/calyx-aster/src/mmap_col.rs` | mmap accessor for cold/columnar Aster columns; page-cache delegated to ZFS ARC |
| `crates/calyx-aster/src/pressure.rs` | Disk-pressure guard; `CALYX_DISK_PRESSURE` before `hotpool` fills; spill trigger |
| `crates/calyx-sextant/src/query_admission.rs` | Bounded query admission; deadline reject with `CALYX_BACKPRESSURE` |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Arena/bump allocator — per-request and per-microbatch, O(1) reset | — |
| T02 | Slab/pool allocator — vector blocks, ANN nodes, GPU staging | T01 |
| T03 | LRU+TTL byte-capped cache — every cache an LRU with byte cap | T01 |
| T04 | Bounded memtable + backpressure — hard byte cap, `CALYX_BACKPRESSURE` | T03 |
| T05 | mmap cold/columnar access — OS page cache, never full vault in heap | T04 |
| T06 | Disk-pressure guard — `CALYX_DISK_PRESSURE`, spill cold to archive | T05 |
| T07 | 1e7-op soak — RSS bounded, no leak, backpressure fires before OOM | T01, T02, T03, T04, T05, T06 |
| T08 | Bounded concurrent-query admission — finite queue, deadline reject | PH24 search |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run a 1e7-operation soak (mixed reads/writes/queries) on aiwonder targeting `calyx-aster`:

```
calyx readback --metric rss_bytes --op-count 1e7
```

- RSS must remain bounded (flat trend) over the full 1e7 ops — read the metric series
- A write flood → backpressure (`CALYX_BACKPRESSURE`) then reject, never OOM
- A query flood past the concurrent cap → bounded queue, deadline reject with
  `CALYX_BACKPRESSURE`, and query reject metrics rise while queue depth returns to zero
- `CALYX_DISK_PRESSURE` fires before `hotpool` fills (inject a fill test, read `disk_free`)
- Evidence (metric chart + reject log) attached to the PH56 GitHub issue

## Risks / landmines

- **mmap + ZFS ARC double-counting:** RSS can look low while ARC is huge; measure both;
  set `primarycache=metadata` on the SST dataset to avoid ARC thrash (hazard 18)
- **Arena size under-estimation for microbatch:** if the arena is too small, reallocation
  defeats O(1) reset semantics; size arenas from actual microbatch profiles on aiwonder
- **Slab alignment for CUDA staging:** GPU staging slabs must be page-aligned (4 KiB) for
  pinned-host transfers; use `std::alloc::alloc` with `Layout::from_size_align`
- **Backpressure + MVCC interaction:** writer backpressure must not pin a snapshot; the
  bounded memtable must flush before rejecting so old versions can advance
- **Single NVMe no redundancy:** `CALYX_DISK_PRESSURE` must fire well before 100% full;
  set high-water at 85% to leave room for compaction temporary space
