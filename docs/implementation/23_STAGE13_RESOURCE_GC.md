# Stage 13 — Resource, GC & Reliability Hardening (PH56–PH59)

Bounded by construction, leak-free, self-reclaiming, fail-closed under pressure.
Rust RAII (no managed GC pauses) + bounded everything + throttled reclaimers +
the 25-hazard register, each FSV-proven. Cross-cutting — **harden continuously
from Stage 1**, finalized here. **Living-system role:** homeostasis (the body
that doesn't tip over). Single-NVMe `hotpool` has no redundancy → buildup = an
outage.

---

## PH56 — Bounded caches/queues/memtables + arenas/pools
- **Objective.** Every allocation has an owner and a hard bound (A26).
- **Deps.** PH08.
- **Deliverables.** arena/bump allocators (per-request/microbatch), slab/pool
  allocators (vector blocks, ANN nodes, GPU staging), bounded memtable +
  backpressure, every cache an LRU+TTL with a byte cap, mmap for cold/columnar.
- **Key tasks.** no unbounded queue/cache/buffer anywhere; fail closed (reject/
  spill) before OOM; `CALYX_BACKPRESSURE`/`CALYX_DISK_PRESSURE`.
- **FSV gate.** **1e7-op soak** on aiwonder → RSS bounded, no leak (read the
  metric series); a write flood → backpressure then reject, never OOM.
- **Axioms/PRD.** A26, `24 §1/§6`.

## PH57 — VRAM budgeter + admission control
- **Objective.** Forge coexists with the 3 resident TEI containers on the one
  RTX 5090.
- **Deps.** PH13.
- **Deliverables.** soft VRAM cap (config), query free VRAM before large
  dispatch, LRU eviction of GPU-resident blocks, admission control (split/queue/
  fail), OOM guard (reduce batch + retry → fail closed).
- **Key tasks.** stream working set from mmap (VRAM holds batch + ANN frontier,
  never the corpus); Anneal yields to serving/TEI; honor 600 W cap.
- **FSV gate.** under concurrent TEI load on aiwonder, a dispatch over budget →
  split/queue/`CALYX_FORGE_VRAM_BUDGET` (no silent OOM); search p99 SLO holds
  (read nvidia-smi + latency).
- **Axioms/PRD.** A26, `24 §2`, `13 §5`.

## PH58 — GC reclaimers + long-reader watchdog + janitor
- **Objective.** Reclaim every kind of database garbage; defeat the classic
  MVCC long-reader version pile-up; keep the operational footprint from
  accumulating.
- **Deps.** PH11.
- **Deliverables.** reclaimers (tombstones/compaction, soft-delete GC 30-day,
  snapshot GC, WAL recycler, ANN tombstones, lazy-xterm LRU, retired-lens
  columns, panel/codebook version GC, orphan reconciler), reader leases +
  snapshot-pin watchdog, the build-artifact/log/temp/dataset janitor (PRD
  `24 §7b`).
- **Key tasks.** reader outliving its lease → aborted
  (`CALYX_READER_LEASE_EXPIRED`) so its version is reclaimed; anti-storm rate
  limits; logs rotate+zstd+TTL; disk-pressure guard before NVMe fills.
- **FSV gate.** a long reader is aborted on lease → old version GC'd, disk flat
  (read oldest-pinned-seq gap + disk free); delete-heavy workload → tombstone
  ratio bounded; logs/build-artifacts bounded.
- **Axioms/PRD.** A26, `24 §3/§4/§7b`.

## PH59 — 25-hazard register FSV + soak
- **Objective.** Every hazard in PRD `24 §7` has a passing FSV; full soak.
- **Deps.** PH56, PH57, PH58.
- **Deliverables.** an FSV readback tool per hazard (compaction storm, flush
  stall, fsync spike, WAL bloat, VRAM/heap OOM, NaN propagation, quant drift,
  index corruption, hot-shard skew, lock contention, cache stampede, slow-lens
  HOL, disk full, ARC thrash, clock skew, Anneal thrash, secret leakage,
  nondeterminism, whole-host loss, upgrade skew).
- **Key tasks.** drive each hazard, read the SoT before/after, prove the
  mitigation; no green-checkmark harness.
- **FSV gate.** all 25 rows pass their byte-level FSV on aiwonder; a 1e7-op soak
  shows RSS/VRAM bounded, no leak, no oscillation (evidence in issues).
- **Axioms/PRD.** A26, `24 §7` (all 25 rows).

---

## Stage 13 exit
A database this ambitious stays bounded, leak-free, and self-reclaiming under
real load, with all 25 hazards FSV-proven and compression as primary pressure-
relief — PRD `RESOURCE`.
