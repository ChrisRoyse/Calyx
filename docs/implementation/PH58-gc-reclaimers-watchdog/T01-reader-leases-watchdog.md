# PH58 · T01 — Reader leases + snapshot-pin watchdog — `CALYX_READER_LEASE_EXPIRED`, version reclaim

| Field | Value |
|---|---|
| **Phase** | PH58 — GC reclaimers + long-reader watchdog + janitor |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/gc/snapshot_gc.rs` (≤500) |
| **Depends on** | PH08 (MVCC snapshot reads exist) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §4`, `24 §7` hazard 6 |

## Goal

Implement reader leases with a max age (default 5 s, the FoundationDB discipline generalized)
and a snapshot-pin watchdog. A read transaction that outlives its lease is aborted with
`CALYX_READER_LEASE_EXPIRED` so its pinned MVCC sequence number is released and old versions
can be GC'd. The watchdog tracks `oldest_pinned_seq` vs `newest_seq`; if the gap exceeds a
bound, it alerts and (configurable) force-aborts the offending reader. Long analytical scans
use bounded-staleness snapshots (read a checkpoint, not the live frontier) so they do not pin
the live sequence. Defends hazard 6 (MVCC version pile-up from long reader).

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct ReadLease { seq: u64, created_at: Instant, lease_duration: Duration, reader_id: ReaderId }` in `snapshot_gc.rs`
- [ ] Implement `ReadLease::is_expired(&self, clock: &dyn Clock) -> bool` — `clock.now() - created_at > lease_duration`
- [ ] Define `struct SnapshotPinWatchdog { leases: Mutex<HashMap<ReaderId, ReadLease>>, max_gap_seqs: u64, clock: Arc<dyn Clock> }`
- [ ] Implement `SnapshotPinWatchdog::register(&self, reader_id: ReaderId, seq: u64, duration: Duration)` — inserts lease
- [ ] Implement `SnapshotPinWatchdog::check_and_abort_expired(&self) -> Vec<ReaderId>` — iterates leases; for expired ones: removes from map, returns their `reader_id` list; callers abort those readers and emit `CALYX_READER_LEASE_EXPIRED`
- [ ] Implement `SnapshotPinWatchdog::oldest_pinned_seq(&self) -> Option<u64>` — minimum seq across all live leases
- [ ] Implement `SnapshotPinWatchdog::check_gap(&self, newest_seq: u64) -> Option<GapAlert>` — if `newest_seq - oldest_pinned > max_gap_seqs` return `Some(GapAlert { gap, oldest_reader_id })`; operator configures `max_gap_seqs` (default 1e6)
- [ ] Add `CALYX_READER_LEASE_EXPIRED` to `calyx-core` error catalog; remediation: "read transaction outlived its lease; retry with a fresh snapshot; consider bounded-staleness mode for long scans"
- [ ] Implement `BoundedStalenessSnapshot::at_checkpoint(seq: u64) -> Self` — wraps a past checkpoint seq for long analytics; does not register with the watchdog (does not pin the live frontier)
- [ ] Wire `check_and_abort_expired` into a background tick (called from the GC scheduler every 1 s)
- [ ] Emit Prometheus `calyx_reader_lease_expired_total` counter and `calyx_oldest_pinned_seq_gap` gauge

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: register a lease with `duration=100ms`; advance mock clock by 101 ms; `check_and_abort_expired()` returns the reader_id; lease is removed from map
- [ ] unit: two readers — one expired, one not; `check_and_abort_expired()` returns exactly one id; map has one entry remaining
- [ ] unit: `oldest_pinned_seq()` with readers pinned at seqs [100, 200, 50] → returns `Some(50)`; after aborting seq=50 → returns `Some(100)`
- [ ] unit: `check_gap(newest=1_100_000, oldest=50, max_gap=1_000_000)` → `Some(GapAlert { gap: 1_099_950 })`; with `newest=1_000_000` → `None`
- [ ] unit: `BoundedStalenessSnapshot` does not appear in `oldest_pinned_seq()` (not registered with watchdog)
- [ ] proptest: `forall leases: Vec<(seq, duration)>, clock_advance: Duration` — after `check_and_abort_expired`, all returned ids have `is_expired == true`; no non-expired lease is aborted
- [ ] edge: empty watchdog — `oldest_pinned_seq()` returns `None`; `check_gap` with no leases returns `None`
- [ ] edge: lease exactly at expiry boundary (clock_now == created_at + duration) → expired (inclusive boundary)
- [ ] fail-closed: `CALYX_READER_LEASE_EXPIRED` is returned to the read caller (not silently discarded); the read operation returns partial results + the error code

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx_reader_lease_expired_total` counter and `calyx_oldest_pinned_seq_gap` gauge on aiwonder; and the actual disk usage of the SST files before/after the long reader is aborted
- **Readback:**
  ```
  calyx readback --metric reader_lease_expired_total
  calyx readback --metric oldest_pinned_seq_gap
  df -h /hotpool
  ```
- **Prove:** start a long reader (hold a snapshot at seq N); wait for lease to expire; verify `reader_lease_expired_total` increments; verify `oldest_pinned_seq_gap` decreases (old version now reclaimable); run snapshot GC (T02); verify disk usage is flat or decreased (version freed). Attach readback output to PH58 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH58 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV

## Implementation note — issue #481

- `crates/calyx-aster/src/gc/snapshot_gc.rs` owns `ReadLease`, `SnapshotPinWatchdog`,
  `GapAlert`, `BoundedStalenessSnapshot`, and `SnapshotGcTick`. The code uses Calyx's
  millisecond `Clock`/`Ts` instead of `Instant` so deterministic FSV and existing MVCC clocks
  stay byte-stable.
- Existing MVCC `ReaderLease` remains the read-path guard. Expired reads now release their
  registered pin and return `CALYX_READER_LEASE_EXPIRED`; the resource lease registry delegates
  to the watchdog so there is one live lease map and one `reader_lease_expired_total` counter.
- `AsterVault::snapshot_gc_tick(max_gap_seqs)` is the PH58 scheduler hook. No general PH58 GC
  scheduler exists yet; T02+ should call this hook at the 1 s cadence alongside version reclaim.
- `ResourceStatus::to_metrics_text` emits `calyx_reader_lease_expired_total` and the existing
  `calyx_oldest_pinned_seq_gap`.

## FSV evidence — issue #481

- Evidence root: `/home/croyse/calyx/data/fsv-issue481-ph58-reader-leases-20260614T221758Z`
- Synthetic known I/O: key `ph58-key`; initial value `v1`; four known updates `v2..v5`; pinned
  reader at seq 1, newest seq 5, hand-computed gap `5 - 1 = 4`.
- Readback: `ph58-reader-leases-readback.json` reports `before_abort.oldest_pinned_seq_gap=4`,
  watchdog tick `aborted_readers=[1]`, `reader_lease_expired_total=1`, and after the exact-boundary
  read failure `reader_lease_expired_total=2`, `active_leases=0`, `oldest_pinned_seq_gap=0`.
- Metrics: `ph58-reader-leases.prom` contains
  `calyx_reader_lease_expired_total{vault="issue481"} 2` and
  `calyx_oldest_pinned_seq_gap{vault="issue481"} 0`.
- Disk/SST readback: `sst_bytes_before_abort=1502` and `sst_bytes_after_abort=1502`; `df-before.txt`
  and `df-after.txt` both show `hotpool/calyx` used bytes `3199598592`.
- Byte readback: `base-cf-readback.txt` shows on-disk base CF rows for `ph58-key` values `v1..v5`;
  `wal-readback.txt` shows WAL seq `1..5`; `base-sst-head.hex` and `wal-head.hex` record raw bytes.
