# PH58 · T04 — WAL recycler anti-storm + ANN tombstone purge

| Field | Value |
|---|---|
| **Phase** | PH58 — GC reclaimers + long-reader watchdog + janitor |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/gc/wal_recycler.rs` (≤500), `crates/calyx-aster/src/gc/ann_gc.rs` (≤500) |
| **Depends on** | T02 (snapshot GC — WAL recycle depends on durability fence), T03 (rate-limit pattern) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §3`, `24 §7` hazard 4, hazard 5 |

## Goal

Retrofit the WAL recycler (PH05's WAL has segment recycle logic) with an anti-storm rate
limiter so WAL recycling never spikes I/O during serving. Also implement the ANN index
tombstone GC: deleted nodes accumulate in the HNSW graph as tombstones; every 10 minutes
(adaptive per Anneal) trigger a bounded rebuild pass that purges them, with safe concurrent
reads throughout. Defends hazard 4 (fsync latency spike) and hazard 5 (WAL bloat).

## Build (checklist of concrete, code-level steps)

**WAL Recycler (`wal_recycler.rs`):**
- [ ] Define `struct WalRecycler { max_recycle_per_tick: usize, fsync_budget_per_tick: usize }`
- [ ] Implement `WalRecycler::run_once(&self, wal: &mut Wal, newest_durable_seq: u64) -> GcResult` — identifies WAL segments where all writes are durable in an SST and the manifest has advanced past them; recycles (resets, not deallocates) up to `max_recycle_per_tick` segments per run; rate-limited by `fsync_budget_per_tick`
- [ ] Implement `WalRecycler::fsync_p99_guard(&self) -> bool` — reads `calyx_fsync_p99_us` metric; if > alert threshold (e.g., 10 ms), skip recycling this tick (don't add I/O under fsync pressure)
- [ ] Anti-storm: if `fsync_p99_guard()` returns true, back off for `2× min_interval` before next attempt
- [ ] Emit Prometheus: `calyx_wal_bytes_active`, `calyx_wal_segments_recycled_total`, `calyx_fsync_p99_us`

**ANN GC (`ann_gc.rs`):**
- [ ] Define `struct AnnGcReclaimer { rebuild_interval: Duration, max_tombstone_ratio: f64 }` (default `rebuild_interval=10min`)
- [ ] Implement `AnnGcReclaimer::tombstone_ratio(&self, index_id: IndexId) -> f64` — count tombstoned nodes / total nodes in the HNSW graph for a given slot
- [ ] Implement `AnnGcReclaimer::run_once(&self, index_id: IndexId) -> GcResult` — if `tombstone_ratio > max_tombstone_ratio`: acquire a read-safe rebuild slot; rebuild the HNSW graph without deleted nodes; atomic pointer swap to the new graph; old graph dropped; reads continue on the live graph throughout (concurrent read safety via Arc + swap)
- [ ] Rate limit: check serving I/O load before triggering rebuild; skip if load > threshold
- [ ] Emit Prometheus: `calyx_ann_tombstone_ratio`, `calyx_ann_gc_rebuild_total`

## Tests (synthetic, deterministic — known input → known bytes/number)

**WAL Recycler:**
- [ ] unit: WAL with 5 segments; seqs 1–100 durable in SST; manifest advanced to seq 100; `run_once` → 5 segments recycled (reset to empty); `wal_bytes_active` decreases
- [ ] unit: `fsync_p99_guard()` with mocked metric at 15 ms → returns true (skip); 5 ms → returns false (proceed)
- [ ] unit: anti-storm backoff — guard triggers, backoff set; next `run_once` before backoff expires is a no-op
- [ ] edge: no durable segments yet (all active) → `run_once` → 0 recycled; no error
- [ ] edge: `max_recycle_per_tick=0` → no recycling (explicit disable)

**ANN GC:**
- [ ] unit: HNSW graph with 100 nodes, 30 tombstoned; `tombstone_ratio == 0.30`; `max_tombstone_ratio=0.25` → rebuild triggered; after rebuild, `tombstone_ratio == 0.0`; all non-tombstoned nodes present
- [ ] unit: concurrent read during rebuild — reader holds Arc to old graph; rebuild completes; reader completes successfully on old graph; old graph dropped after reader releases Arc
- [ ] unit: `tombstone_ratio=0.20 < max_tombstone_ratio=0.25` → `run_once` is a no-op; no rebuild
- [ ] edge: all nodes tombstoned → rebuild produces empty graph; no panic
- [ ] fail-closed: rebuild I/O error → old graph retained (pointer not swapped); `CALYX_IO_ERROR` returned; graph remains queryable

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx_wal_bytes_active` gauge and `calyx_ann_tombstone_ratio` gauge on aiwonder; `calyx_fsync_p99_us`
- **Readback:**
  ```
  calyx readback --metric wal_bytes_active
  calyx readback --metric wal_segments_recycled_total
  calyx readback --metric ann_tombstone_ratio
  calyx readback --metric fsync_p99_us
  ```
- **Prove:** run 1e5 writes; verify `wal_bytes_active` stays bounded (segments recycled after flush); delete 30% of ANN nodes; verify `ann_tombstone_ratio` rises then falls after GC; `fsync_p99_us` does not spike during recycling (guard worked). Attach readback output to PH58 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH58 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
