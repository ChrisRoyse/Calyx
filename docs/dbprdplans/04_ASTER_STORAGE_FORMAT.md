# 04 — Aster Storage Format & Engine

> **Living-system role:** metabolism + memory — ingest transforms inputs into grounded structure and stores it (A31 — DOCTRINE §1b)

The on-disk substrate. Logical model is `03`; math kernels are `13`.

## 1. Design targets

| Target | Value | Why |
|---|---|---|
| Constellation write p95 | < 5 ms (1-slot), < 20 ms (15-slot, GPU-batched) | Leapable ingest throughput |
| Slot ANN search | O(log n), p99 < 10 ms @ 1e6 cx | interactive Ask |
| Vault scale (embedded) | 1e6–1e7 constellations on a laptop NVMe | end-user vault |
| Vault scale (server) | 1e9 constellations / vault on aiwonder NVMe+HDD | Core corpus |
| Hot-add lens | no full re-embed; lazy backfill | A5 |
| Crash recovery | replay to last consistent seq, byte-exact | A15 |
| Disk amplification | quantized vectors default; raw recoverable | 32 GB VRAM / 1.5 TB NVMe budget |

## 2. Why a custom format (and what we borrow)

Borrow proven ideas, build the association-specific parts:

| Borrowed | From | Calyx use |
|---|---|---|
| Columnar, Arrow-compatible, disk-native vector scan, zero-copy mmap, SIMD scan | **Lance / LanceDB** (Rust) | Aster column chunks are Arrow-layout so slot columns scan with SIMD straight from mmap |
| LSM + column families + WAL + background compaction | **RocksDB** (ContextGraph already uses it) | base KV + per-slot CFs |
| Graph ANN with vectors co-located for locality | **DiskANN** | per-slot disk-resident graph index for billion-scale server vaults |
| Memory/disk hybrid inverted posting lists | **SPANN** | sparse-lens (SPLADE/keyword) posting lists: centroids in RAM, lists on disk |
| Named/multi-vector with per-vector index/quant config | **Qdrant** | each Slot owns its own ANN + quant policy |

Calyx is **not** a fork of any of these; it is a thin LSM+columnar core with **association-native column families and indexes** the others lack (cross-terms, kernel index, bits store, guard profiles, ledger).

### Aster as the universal core (general data layer, `20`)
Aster is an **ordered, transactional** keyspace (the FoundationDB layer pattern): every paradigm is a **key-encoding layer** on it; ACID-per-vault transactions make those layers correct. Indexing = "write the data key **and** the index key in one transaction."

| Layer | Key encoding | Serves |
|---|---|---|
| Relational | `(table, pk) → row`; `(idx, val, pk) → ∅` | tables, secondary indexes, range/point |
| Document | tuple path keys `(doc_id, p1, p2, …) → leaf` | nested docs, subtree prefix-reads |
| KV | `(ns, key) → val` + TTL | O(1) state |
| Columnar/OLAP | Arrow column chunks (mmap SIMD scan) + HTAP row mirror | aggregates/scans |
| Graph | `(node)→props`, `(src, etype, dst)→edge` + CSR projection | traversal (also the native association graph) |
| Time-series | `(series, ts)→point` + rollups + retention | range/rollup |
| Full-text | inverted `(term)→postings` (SPANN) | = a sparse lexical **lens** |
| Vector | per-slot ANN | = a dense **lens** |
| Blob | chunked payload + manifest | large objects, cold tier |

Constellation collections are the richest layer (per-slot columns + cross-terms + anchors + ledger). One core, one transaction, one source of truth — the whole point of `20`.

## 3. Physical layout (a vault on disk)

```
vault.calyx/                         # a vault = a directory
  MANIFEST                           # current panel_version, kernel_ref, guard_ref, format_version
  CURRENT -> manifest-NNNN           # atomic pointer (rename())
  wal/                               # write-ahead log segments (group-commit, fsync)
    000123.wal
  cf/                                # column families (LSM SSTables)
    base/                            # CxId -> ConstellationHeader (modality, flags, scalars, anchors, prov ref)
    slot_00/ ... slot_NN/            # per-slot quantized vector column chunks (Arrow layout)
    slot_00.raw/ ...                 # optional raw-f32 sidecar (cold tier) for re-quant / exact rescore
    xterm/                           # materialized cross-terms (CxId,a,b,kind) -> value
    scalars/                         # scalar columns (btree-indexed)
    anchors/                         # (CxId,kind) -> AnchorValue   [grounded outcomes]
    ledger/                          # append-only hash-chained provenance (see 11)
    online/                          # mistake log, replay buffer, online head state
  idx/
    slot_00.ann/ ...                 # per-slot HNSW (embedded) or DiskANN (server) graph
    slot_00.token.ann/ ...           # server token DiskANN + MaxSim sidecars for multi slots
    slot_00.asym_a/ slot_00.asym_b/  # asymmetric dual indexes when Slot.asymmetry = Dual
    xterm.concat.ann/                # server DiskANN over materialized Concat xterm rows
    slot_06.sparse/                  # SPANN-style inverted lists for sparse lenses
    kernel/                          # Lodestar kernel index (kernel CxIds + recall meta)
    scalars.btree/
  codebooks/                         # per-slot PQ/Float8 codebooks (quant artifacts)
  guard/                             # GuardProfile versions (per-slot τ + calibration prov)
  panel/                             # Panel version history (slots, lens ids, shapes)
```

### Tiering onto aiwonder ZFS
| Tier | Lives on | Holds |
|---|---|---|
| Hot | `hotpool` NVMe (`/zfs/hot/calyx/`) | WAL, base CF, active-slot quantized columns, ANN graphs, kernel/guard, online state |
| Cold | `archive` HDD mirror (`/zfs/archive/calyx/`) | `*.raw` f32 sidecars, retired-slot columns, old panel versions, ledger archive, restic source |
| VRAM | RTX 5090 32 GB | hot ANN frontier + batched matmul/MI working set (Forge), never the source of truth |

ZFS notes (from `aiwonder-system.md` gotchas): reference disks by `wwn-`/`eui-`, never `/dev/nvmeXn1`; stage temp files **in the destination dataset** to avoid `EXDEV` on cross-dataset rename; `hotpool` has no redundancy → WAL + restic + ZFS snapshots are the durability story; whole-host loss is accepted.

## 4. Column families & key schema

| CF | Key | Value | Index |
|---|---|---|---|
| `base` | `CxId` | header (modality, flags, scalar refs, anchor refs, ledger ref, created_at) | primary |
| `slot_k` | `CxId` | quantized SlotVector (dense PQ/F8 / sparse / multi) | ANN/inverted in `idx/`; multi server slots use `idx/slot_k.token.ann/` |
| `slot_k.raw` | `CxId` | raw f32 (cold) | none (scan/rescore only) |
| `xterm` | `(CxId, a, b, kind)` | cross-term value | optional ANN on Concat keys in `idx/xterm.concat.ann/` |
| `scalars` | `(ScalarId, CxId)` | f64 | btree |
| `anchors` | `(CxId, AnchorKind)` | AnchorValue + source + ts | by-kind secondary |
| `ledger` | `seq` | hash-chained provenance entry | merkle (`11`) |
| `online` | typed | mistake/replay/head | — |

Keys are big-endian-ordered for range scans; `CxId` is 16 B blake3 prefix (collision-checked on write).

## 5. Write path (group-commit, fail-closed)

```
ingest(input, panel) ->
  1. cx_id = blake3(input ‖ panel_version ‖ salt); if exists -> idempotent return (dedup)
  2. Forge.embed_batch(input, active_lenses) -> slot vectors        # GPU-batched across lenses
  3. quantize per Slot.quant; keep raw in write buffer if cold-tier sidecar enabled
  4. Loom.plan_cross_terms(slots) -> {pairs to eager-materialize}   # most: lazy (06)
  5. WAL append {base, slot_*, anchors?, ledger entry}; fsync (group-commit window ≤ 2 ms)
  6. apply to memtable; ack after the fsynced WAL sequence is visible in MVCC
  7. async: ANN insert per slot; xterm lazy; Ledger merkle update; Anneal counters
  fail-closed before WAL fsync -> structured error, WAL not acked, no partial visible state
  fail after WAL fsync -> WAL sequence is authoritative and must be restored or recovered
```

Embedding dominates cost; step 2 batches all lenses for a constellation (and across a microbatch of constellations) into one GPU dispatch (`13`).

## 6. Read / snapshot / MVCC

- Each write advances a vault **sequence number**; readers pin a seq → consistent snapshot across all CFs (LSM read-snapshot semantics).
- Derived structures (ANN, xterm, kernel, guard) carry the `base` seq they were built at; a read can demand `fresh_derived` (block on rebuild) or accept `stale ≤ S` (default), surfacing staleness in the result.
- Compaction is background, snapshot-safe (concurrent reads during rebuild), on a timer (mirrors ContextGraph's 10-min HNSW compaction); Anneal makes the cadence adaptive (`12`).

## 7. Crash safety & recovery

- WAL is the source of truth for un-compacted writes; recovery replays WAL past the last durable manifest to the last fsync'd record, discards a torn tail, and normal vault open exposes the torn-tail diagnostic.
- Manifest swap is atomic (`rename()` of `CURRENT`). Codebooks and panel versions are immutable once referenced.
- ANN/kernel/guard are **rebuildable** from base+slots, so a corrupt index is a `degraded` flag + background rebuild, never data loss (A16). A corrupt base shard fails the read closed and points the operator at restic/snapshot restore.
- FSV for storage (A15): recovery is proven by killing `calyxd` mid-write and reading back the exact persisted constellations/anchors/ledger bytes — not by a harness asserting "recovered: true".

## 8. Compression & disk budget

| Object | Default policy |
|---|---|
| Dense slot vector | PQ-8 (m subquantizers) or Float8 for ≤768-d; binary for recall-prefilter |
| Raw f32 sidecar | zstd, cold tier, optional (on for rescore-sensitive slots) |
| Sparse slot | posting lists, varint + zstd block |
| Cross-terms | only materialized subset; Concat keys quantized like dense |
| Ledger | append-only, periodic merkle checkpoint, zstd archive to cold |

Budget example (server vault, 1e8 cx, 15 slots avg 512-d, PQ-8): ≈ `1e8 × 15 × 64 B ≈ 96 GB` quantized hot + raw sidecars on cold — fits `hotpool` 1.5 TB with room for ANN graphs; billion-scale uses DiskANN on-disk graphs (SPANN for sparse) per `10`.

## 9. Format versioning

`format_version` in MANIFEST; readers refuse unknown-major, migrate known-minor. Aster is **forward-append**: new CFs/indexes appear without rewriting old shards. A panel/lens/codebook is content-addressed and immutable, so format evolution never silently reinterprets an existing constellation (A4/A16).

Sources: [Lance columnar format](https://github.com/lancedb/lancedb) · [DiskANN (single-node billion-scale)](https://www.microsoft.com/en-us/research/publication/diskann-fast-accurate-billion-point-nearest-neighbor-search-on-a-single-node/) · [SPANN](https://arxiv.org/abs/2111.08566) · [Qdrant per-vector config](https://qdrant.tech/documentation/manage-data/collections/).
