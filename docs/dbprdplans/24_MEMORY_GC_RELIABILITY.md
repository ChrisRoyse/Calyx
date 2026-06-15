# 24 — Memory, Garbage Collection & Reliability Engineering

Implements new **A26 (bounded, leak-free, self-reclaiming)**. The founder's mandate: *optimize for garbage collection and any memory issues and everything that could go bad with a database like this, clearly addressed.* Exhaustive treatment of memory, GC, resource exhaustion, and the failure register. Complements `17` (durability/consistency/intelligence risks); `24` owns **resource lifecycle**.

## 1. Memory model — no managed GC, bounded by construction

Calyx is **Rust**: no language GC, no stop-the-world pauses, deterministic destruction (RAII). Memory discipline:

| Mechanism | Use |
|---|---|
| **Ownership/RAII** | every buffer, mmap, GPU allocation freed deterministically at scope end; no reference cycles (no `Rc` cycles on hot paths) → **no leaks by construction** |
| **Arena / bump allocators** | per-request and per-microbatch transient buffers (scoring, cross-terms, MI working set) allocated from an arena, reset in O(1) at request end — no per-op `malloc`/`free` churn |
| **Slab/pool allocators** | fixed-size vector blocks, ANN nodes, GPU staging buffers pooled and reused (returned to pool, not freed) |
| **mmap for cold/columnar** | Aster columns are memory-mapped; the OS page cache (ZFS ARC) is the cache, paged in/out under pressure — Calyx never holds the whole vault in heap |
| **Bounded memtables** | LSM memtable has a hard byte cap → flush to SST at threshold; writers see backpressure, never unbounded heap growth |
| **Bounded caches** | every cache (lazy cross-terms, query plans, autotune configs, kernel results) is an LRU with a byte cap + TTL; no cache is unbounded |

**Invariant (A26):** every allocation has an owner and a bound. No unbounded queue, cache, buffer, or working set anywhere. Fail closed (reject/spill) before OOM, never crash.

## 2. VRAM management (the single shared RTX 5090)

The 32 GB GPU is shared with 3 resident TEI containers + dcgm-exporter (`16`). Forge MUST coexist:

| Control | Behavior |
|---|---|
| **VRAM budgeter** | soft cap (config) on Forge's VRAM; queries device free VRAM before large dispatch; never assumes the whole 32 GB |
| **Streaming from mmap** | working set streams from Aster columns via pinned-host double-buffering; VRAM holds only the current batch + ANN frontier, never the corpus (`04 §3`) |
| **Eviction** | LRU eviction of cached GPU resident blocks under pressure; ANN frontier capped |
| **Admission control** | a dispatch that would exceed budget is split or queued; if it cannot fit, fail closed with `CALYX_FORGE_VRAM_BUDGET` (never silent OOM) |
| **Yield to residents** | Anneal/background math yields VRAM/SM to serving + TEI; capped background budget (`12 §6`) |
| **OOM guard** | every CUDA alloc checked; OOM → reduce batch + retry, then fail closed; no driver-level abort |

Compression (`23`: TurboQuant + MXFP4) is the primary VRAM lever — quantized working sets are 4–10× smaller, so more fits and OOM is rarer.

## 3. Garbage collection (database sense) — what reclaims what

Calyx's reclaimers, each bounded and background:

| Garbage | Reclaimer | Policy |
|---|---|---|
| **LSM tombstones / overwritten keys** | compaction | leveled/tiered; **adaptive cadence** (Anneal) to avoid write-amp storms; throttled to not starve serving |
| **Soft-deleted constellations** | soft-delete GC | 30-day recovery window (inherited from ContextGraph), then purge; background sweep every 5 min |
| **Old MVCC versions** | snapshot GC | a version is reclaimable once no live reader pins a seq ≤ it; **long-reader watchdog** (below) prevents version pile-up |
| **WAL segments** | WAL recycler | segment freed once its writes are durable in an SST + manifest advanced; recycled (not reallocated) |
| **ANN graph tombstones** | index compaction | deleted nodes purged on the 10-min (adaptive) rebuild; safe concurrent reads |
| **Lazy cross-term cache** | LRU+TTL | computed-on-demand terms evicted by size/age |
| **Retired-lens columns** | panel GC | retained for historical interpretability, then pruned by retention policy (cold-tier first) |
| **Old panel/codebook versions** | version GC | immutable; pruned when no constellation references them |
| **Orphan slots/indexes** | reconciler | periodic scan finds index entries with no base constellation (and vice-versa) → repair/purge (inherited from Leapable `reconcile_files`) |
| **Ledger** | archival, never deleted | append-only; old ranges + Merkle checkpoints moved to cold/restic, never purged (audit) |
| **Time-series / blobs** | retention | per-collection TTL/rollup; downsample then drop raw |

**Anti-storm rules:** compaction and index rebuild rate-limited and prioritized below serving; a "compaction debt" metric alerts before a stall; GC runs reversible until the recovery window closes.

## 4. The long-reader / snapshot-pin hazard (the classic MVCC leak)

The most common MVCC/LSM "leak": a long-running read pins an old sequence, so no old version can be GC'd and disk/heap grows. Calyx defenses:
- **Reader leases** with a max age; a read that outlives its lease is aborted (`transaction_too_old`-style) so its pinned version can be reclaimed (the FoundationDB 5-second-version discipline, generalized).
- **Snapshot-pin watchdog** metric: oldest pinned seq vs newest; alert + (configurable) forced-abort if the gap exceeds a bound.
- Long analytical scans use **bounded-staleness snapshots** that don't pin the live frontier (read a checkpoint).

## 5. Memory issues specific to a multi-lens / array DB

| Issue | Mitigation |
|---|---|
| **Per-constellation memory grows with N** | quantize every slot (TurboQuant/MXFP, `23`); the bundle is the dominant cost → keep it small; raw f32 only as a cold sidecar |
| **Cross-term N² blowup** | lazy by default, Assay-gated, `n_eff`-budgeted (`06`) — materialized ≪ C(N,2) |
| **Ragged-array fragmentation** | SoA per-slot columns + slab pools → no per-block fragmentation; add/remove lens = block append/tombstone |
| **High-d k-NN (MI) memory** | random-projection pre-step (also what TurboQuant rotates) before KSG; batched, streamed |
| **ANN graph RAM (embedded)** | HNSW capped; spill to **DiskANN** on server scale (`10`) |
| **Sparse-lens posting lists** | SPANN: centroids in RAM, lists on NVMe (`04`) |
| **Panel-version explosion** | versions immutable + GC'd when unreferenced; backfill consolidates |

## 6. Backpressure & admission control (never tip over)

- Bounded write queue per vault; at high-water → **backpressure** to the writer (slow ack), then reject with `CALYX_BACKPRESSURE` (fail closed), never buffer to OOM.
- Bounded ingest microbatch; a slow lens endpoint causes **head-of-line** risk → per-lens timeout + circuit breaker + route to remaining lenses (graceful degradation, `12`).
- Bounded concurrent queries; excess queued with a deadline; deadline exceeded → reject, not pile up.
- Disk-pressure guard on `hotpool` (no redundancy): at high-water, stop accepting writes (fail closed) + alert + spill cold to `archive` — **never** fill the single NVMe to corruption.

## 7. Everything-that-could-go-wrong register (DB-building hazards)

Each: hazard · mitigation · the FSV that proves it handled. (Durability/consistency/intelligence hazards in `17`; here = resource/operational.)

| # | Hazard | Mitigation | FSV |
|---|---|---|---|
| 1 | **Write amplification / compaction storm** | adaptive, throttled, debt-metered compaction; tiered for write-heavy | soak: write-amp ≤ target, no serving p99 breach during compaction |
| 2 | **Memtable flush stall** | bounded memtable + backpressure + parallel flush | sustained-write test: no unbounded heap, acks keep flowing |
| 3 | **Tombstone buildup** | GC cadence + reconciler | delete-heavy workload: tombstone ratio bounded |
| 4 | **fsync latency spike** | group-commit window; WAL on NVMe; alert on fsync p99 | inject slow-disk: acks degrade gracefully, no data loss |
| 5 | **WAL bloat** | segment recycle once durable | crash/recover: WAL bounded, replays clean |
| 6 | **MVCC version pile-up (long reader)** | reader leases + watchdog (§4) | long-scan test: old versions reclaimed, disk flat |
| 7 | **VRAM OOM** | budgeter + admission + split/retry (§2) | concurrent-with-TEI load: no OOM, fail-closed if over |
| 8 | **Heap OOM** | bounded caches/queues/memtables; arenas | fuzz/soak: RSS bounded over 1e7 ops |
| 9 | **NaN/Inf propagation** | kernel-boundary guards → `CALYX_FORGE_NUMERICAL_INVARIANT` | inject NaN: fails closed, isolated |
| 10 | **Quantization drift / lossy beyond tolerance** | measured intelligence contract (`23 §4.4`); raw sidecar rescore | bits/cosine before-after within bound at chosen level |
| 11 | **Codebook/rotation staleness** | TurboQuant is data-oblivious (no codebook) → mostly N/A; QJL seed versioned | re-quant parity test |
| 12 | **ANN/kernel index corruption** | rebuildable from base; `degraded` flag + background rebuild | flip bytes: read degrades, rebuilds, no data loss |
| 13 | **Hot-shard / tenant skew** | per-vault isolation; rate limits; shard by CxId | skewed load: no single-shard collapse |
| 14 | **Lock contention** | single-writer-per-vault + lock-free reads (MVCC) | concurrency stress: no deadlock, read throughput holds |
| 15 | **Cache stampede / thundering herd** (kernel/cross-term recompute) | single-flight + LRU + TTL jitter | concurrent identical misses → one compute |
| 16 | **Slow-lens head-of-line** | per-lens timeout + breaker + degrade | kill a lens endpoint: search degrades, doesn't hang |
| 17 | **Disk full on `hotpool`** | disk-pressure guard, spill to archive, fail-closed writes | fill test: writes reject cleanly, no corruption |
| 18 | **ZFS ARC pressure / mmap thrash** | working-set caps; prefetch tuning; cold on HDD | memory-pressure test: graceful, no thrash collapse |
| 19 | **Clock skew (timestamps)** | server-stamped monotonic seq, not wall-clock for ordering | skew injection: ordering intact (seq-based) |
| 20 | **Anneal thrash / oscillation** | hysteresis + tripwires + shadow-first + reversible (`12`) | 1e6-query soak: converges, no flapping |
| 21 | **Panel-version / cross-term explosion** | version GC; lazy + `n_eff` budget | many add/retire cycles: storage bounded |
| 22 | **Secret leakage / candidate-text persistence** | Ledger stores hashes; redacted-input; request-scoped reranker text | scan persisted bytes: no secrets/candidate text |
| 23 | **Numerical nondeterminism (replay)** | determinism mode: fixed reduction order, no nondeterministic atomics | replay an answer: bit-parity within tolerance |
| 24 | **Whole-host loss (single box, no redundancy)** | WAL + ZFS snapshots + restic; accepted posture, documented | DR drill: restore + byte-verify (`16`) |
| 25 | **Upgrade/format skew** | content-addressed immutable lens/codebook/panel; forward-append format; refuse unknown-major | migrate test: old shards readable, no silent reinterpret |

## 7b. Build artifacts, logs & disk hygiene (no buildup)

Beyond runtime garbage (§3), the *operational* footprint must not accumulate — old build data, logs, temp files, and stale datasets are reclaimed on a schedule (A26; binding). The single NVMe `hotpool` has no redundancy and finite space, so buildup = an outage.

| Source | Hygiene |
|---|---|
| **Build artifacts** | `cargo` target dirs + old `.deb`/static binaries pruned — keep last N releases; `cargo clean` stale targets; never let `target/` accrue across versions on aiwonder; the build host caps its caches (e.g. `sccache`). |
| **Logs** | structured logs rotate by size+age (`tracing-appender`/logrotate), zstd-compress then drop on TTL; bounded total log bytes per service; the Ledger (audit) is *not* a log — it archives to cold, never grows unbounded hot (`11`). |
| **Temp / scratch** | staged temp files written **in the destination dataset** (avoid `EXDEV`, `16`), cleaned on commit/abort; synthetic FSV data cleanup-tagged + removed before the turn ends (`28`). |
| **Datasets** | downloaded datasets (`28 §3`) live on cold `archive`; unused ones pruned per MANIFEST; raw kept only while a test needs it, else the parsed/quantized form. |
| **WAL / SSTs / snapshots** | WAL recycled once durable (§3); ZFS snapshots pruned on a retention schedule (recent + restic); compaction keeps SST levels bounded. |
| **Disk-pressure guard** | at a `hotpool` high-water mark, **stop accepting writes (fail closed)** + alert + spill cold to `archive` — never fill the NVMe to corruption (§6); a `disk_free` tripwire pages before that point. |

Cadence: a bounded background **janitor** (Anneal/ops) runs the prunes on a timer, rate-limited below serving, reversible within the recovery window, Ledger-logged. **Nothing accumulates silently** — every reclaimer has a metric (§8) + alert threshold.

## 8. Observability for resource health (`16`)

Prometheus metrics catching the above early: heap RSS, arena high-water, VRAM budget use + OOM-avoided count, compaction debt + write-amp, tombstone ratio, oldest-pinned-seq gap, WAL bytes, memtable flush latency, cache hit/evict rates, backpressure events, disk free on hotpool, lens timeout/breaker trips, Anneal A/B + rollback count, fsync p99, NaN-guard trips. Alert thresholds on each; a resource tripwire pages before an incident.

## 9. The new axioms

- **A25 — Maximal measured compression.** Calyx MUST compress to the most aggressive level that **measurably preserves intelligence** (bits/cosine/FAR/kernel-recall within bound), using TurboQuant + microscaling + the kernel, never a guessed bit-width (`23`).
- **A26 — Bounded, leak-free, self-reclaiming.** Every allocation, cache, queue, and buffer MUST have an owner and a hard bound; every form of database garbage MUST have a bounded background reclaimer; the system MUST fail closed under resource pressure, never OOM/corrupt. No unbounded growth anywhere.

**One sentence:** Calyx has no managed-GC pauses (Rust RAII), bounds every allocation/cache/queue by construction, reclaims every kind of database garbage with throttled background collectors, defends the classic hazards (long-reader version pile-up, compaction storms, VRAM/heap OOM, tombstone buildup, cache stampede, disk-full on the single NVMe) with explicit mitigations + FSV proofs, and uses compression (`23`) as primary pressure-relief — so a database this ambitious stays bounded, leak-free, and self-reclaiming under real load.
