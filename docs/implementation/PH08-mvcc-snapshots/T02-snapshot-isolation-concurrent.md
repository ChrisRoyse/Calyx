# PH08 · T02 — Snapshot isolation: concurrent writer+reader no partial read

| Field | Value |
|---|---|
| **Phase** | PH08 — MVCC sequence numbers + snapshot reads |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/mvcc/store.rs` (≤500), `crates/calyx-aster/src/mvcc/tests.rs` (≤500) |
| **Depends on** | T01 (SeqAllocator) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §6`, `dbprdplans/03 §8` |

## Goal

Prove the cross-CF snapshot invariant: a reader that pins a seq before a
`commit_batch` completes will see either all rows from that batch or none; it will
never see a partial write (e.g., `base` CF row visible but the corresponding
`slot_00` row not yet visible). This is the MVCC correctness guarantee that
makes constellation reads consistent.

## Build (checklist of concrete, code-level steps)

- [x] Add a concurrent test using `std::sync::Barrier`:
  1. Shared `Arc<VersionedCfStore>`.
  2. Reader thread pins snapshot at `seq_before = store.current_seq()`.
  3. Barrier sync → writer thread calls `commit_batch([(CF::Base, k, v1),
     (CF::Slot(slot_0), k, v2)])`.
  4. Reader reads `Base` at `seq_before` → must be `None` (batch not yet visible
     at the pinned seq).
  5. Reader reads `Slot(slot_0)` at `seq_before` → must also be `None`.
  6. Pin a new snapshot at `seq_after`; both reads return `Some`.
  7. Repeat for 1000 iterations to stress-test.
- [x] Add test: `read_batch` returns all results atomically at one seq — all Some
  or all None across the CF reads in the batch.
- [x] Add test: after two `commit_batch` calls (seqs S1 and S2), pinning at S1
  shows only the first batch's data; pinning at S2 shows both.
- [x] Verify `commit_batch` allocates seq inside the write lock (current
  implementation: `let seq = self.seqs.allocate()` is called while holding
  `self.rows.write().lock()` — confirm this holds after any refactor).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: reader pinned before write → reads None for all CF rows in the batch.
- [x] unit: reader pinned after write → reads Some for all CF rows in the batch.
- [x] concurrent stress (1000 iterations): no partial constellation read observed.
- [x] edge (≥3): (1) empty `commit_batch` → seq unchanged; (2) single-CF batch
  is atomic; (3) 10-row batch spanning 5 CFs is atomic.
- [x] fail-closed: expired snapshot `ensure_live` → `CALYX_READER_LEASE_EXPIRED`
  even if the rows exist.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-aster mvcc::tests::snapshot_isolation` on aiwonder.
- **Readback:** `cargo test -p calyx-aster mvcc -- --nocapture 2>&1 | grep -E
  "(partial|isolation|PASS|FAIL)"`
- **Prove:** The stress test prints "1000/1000 iterations: no partial read" and
  exits with code 0. Screenshot of terminal output posted to PH08 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH08 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
