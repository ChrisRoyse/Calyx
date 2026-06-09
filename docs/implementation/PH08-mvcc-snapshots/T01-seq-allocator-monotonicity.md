# PH08 · T01 — SeqAllocator monotonicity + proptest

| Field | Value |
|---|---|
| **Phase** | PH08 — MVCC sequence numbers + snapshot reads |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/mvcc/lease.rs` (≤500), `crates/calyx-aster/src/mvcc/tests.rs` (≤500) |
| **Depends on** | PH04 (Seq, Clock types) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §6`, `dbprdplans/03 §8` |

## Goal

Prove that `SeqAllocator::allocate()` is strictly monotonic under any number of
concurrent callers, that `current()` always reflects the latest committed seq,
and that `ReaderLease` expiry correctly gates `Snapshot::ensure_live`. Use a
multi-threaded stress test with a seeded thread count to verify no duplicate seqs
are produced. These are the invariants everything above depends on.

## Build (checklist of concrete, code-level steps)

- [x] Add proptest: for n in `1..=100` sequential `allocate()` calls on a single
  `SeqAllocator(start=0)`, the returned seqs are `[1, 2, ..., n]` with no gaps
  or duplicates.
- [x] Add a concurrent test: spawn 8 threads, each calling `allocate()` 100 times
  on a shared `Arc<SeqAllocator>`; collect all 800 returned seqs; assert they are
  all distinct and form a contiguous range `[1..=800]`.
- [x] Add test: `SeqAllocator::new(42).current() == 42`; after one `allocate()`,
  `current() == 43`.
- [x] Add test: `ReaderLease::is_expired` with `FixedClock` at issued_at +
  max_age_ms - 1 → false; at issued_at + max_age_ms + 1 → true.
- [x] Add test: `Snapshot::ensure_live` with expired lease returns
  `Err(code == "CALYX_READER_LEASE_EXPIRED")`.
- [x] Add proptest: for any `(start: u64, n: u64)` with `n <= 100`,
  `SeqAllocator::new(start)` + n allocations → all seqs in `(start, start+n]`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 800-thread concurrent allocation → all seqs distinct.
- [x] unit: expired lease → `CALYX_READER_LEASE_EXPIRED` code.
- [x] proptest: sequential monotonicity for all `(start, n)` in range.
- [x] edge (≥3): (1) `start = u64::MAX - 1` → wrap on second allocation (or
  overflow; document behavior, assert it does not panic); (2) `max_age_ms = 0`
  → lease immediately expired; (3) `max_age_ms = u64::MAX` → effectively never
  expires.
- [x] fail-closed: `ensure_live` on expired lease → structured error with
  `code == "CALYX_READER_LEASE_EXPIRED"`, not a panic.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-aster mvcc::tests -- --nocapture 2>&1` on aiwonder.
- **Readback:** Terminal output of the concurrent allocator test showing "800 seqs
  all distinct" assertion passing.
- **Prove:** The stress test prints the min and max allocated seq and the count of
  unique values; all three are consistent. Screenshot posted to PH08 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH08 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
