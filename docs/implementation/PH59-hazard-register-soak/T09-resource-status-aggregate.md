# PH59 · T09 — `resource_status()` aggregate API (issue #592)

**Source (binding):** PRD doc 18 §4 (`fn resource_status(&self, v) -> Result<ResourceStatus>` —
heap/VRAM/compaction-debt/pinned-seq/backpressure), doc 24 §8 (observability for resource
health); coverage-audit gap #592.

## What was unowned

The pieces existed (compaction debt meter, MVCC reader leases, memtable byte-cap
backpressure, Anneal budget probe) but no card assembled the public aggregate accessor —
and two of the five PRD fields were physically uncollectable:

1. **Backpressure events were invisible.** `CfRouter::put` absorbs a memtable
   `CALYX_BACKPRESSURE` with an emergency flush and counted nothing; the durable commit
   path absorbs post-WAL memtable rejections via the restore path with only an `eprintln`.
2. **Oldest-pinned-seq gap was uncomputable.** Leases were minted by `pin_snapshot` but
   never registered anywhere, so "what is the oldest live pin" had no answer.

## Build

| Piece | Where | Notes |
|---|---|---|
| `ResourceCounters` | `calyx-aster/src/resource/counters.rs` | Atomic monotonic counters; shared `Arc` between `CfRouter` (increments at the fire point) and `VersionedCfStore`. One cap-hit event increments exactly one of absorbed/rejected. |
| `LeaseRegistry` | `calyx-aster/src/resource/leases.rs` | `pin_snapshot` registers; `release_lease` / expiry prune removes. Bounded by lease expiry (A26). |
| Heap probe | `calyx-aster/src/resource/heap.rs` | `/proc/self/status` `VmRSS` (proc_pid_status(5)). No cross-platform fallback: fails closed with `CALYX_RESOURCE_PROBE_UNAVAILABLE`. |
| `ResourceStatus` + Prometheus rendering | `calyx-aster/src/resource/status.rs` | Schema-versioned serde struct; `to_metrics_text` emits PRD 24 §8 metric names (`calyx_heap_rss_bytes`, `calyx_compaction_pending_compaction_bytes{cf}`, `calyx_oldest_pinned_seq_gap`, `calyx_backpressure_events_total{source}`, `calyx_wal_bytes`, …). |
| Collector | `calyx-aster/src/resource/collect.rs` | Each section read from its physical SoT at call time: SST shard sizes on disk (RocksDB `estimate-pending-compaction-bytes` pattern), `wal/*.wal` segment sizes, live store state. Any unreadable source fails the whole call. |
| Vault API | `AsterVault::{resource_status, pin_reader, release_reader}` | PRD 18 §4 surface; explicit readers are the tracked ones, vault-internal snapshot handles do not pollute the gap. |
| CLI | `calyx resource-status --vault <dir> [--metrics]`, `calyx resource-drill --vault <dir> --ops <n> --value-bytes <n> --memtable-cap <bytes> --pin-max-age-ms <ms>` | Status probe refuses to create vault state on a wrong path (fail-closed, `CALYX_DISK_PRESSURE`). Drill drives the real WAL+MVCC+router write path with deterministic rows and prints full status BEFORE/AFTER/FINAL. |
| VRAM section | CLI composes `calyx-anneal` `BudgetConfig::load_from_vault` + `BudgetEnforcer::tick` | Probe degradation (NVML unavailable) surfaces in `probe_warning`, never as silent zero. |

## Hand-computable drill model (2+2=4 discipline)

With `--memtable-cap 100 --value-bytes 52` (row = 8-byte key + 52 = 60 bytes):
put 1 fits (60 ≤ 100); every later put projects 120 > 100 → absorb-flush. So for N ops:
`memtable_absorbed_total = N-1`, SST flush files = N-1, `oldest_pinned_seq_gap = N`
while the pinned lease is live, 0 after release; WAL segment bytes match `stat`.

## FSV

Unit/integration: `crates/calyx-aster/src/resource/tests.rs` (13 tests; counters and debt
asserted against independent `fs::metadata` reads), `calyx-cli` `main_tests`.
Binding byte-level FSV ran on aiwonder per the issue protocol — synthetic known-I/O,
≥3 edge cases with BEFORE/AFTER SoT prints, evidence under
`/home/croyse/calyx/data/fsv-issue592-resource-status/`, attached to issue #592.
