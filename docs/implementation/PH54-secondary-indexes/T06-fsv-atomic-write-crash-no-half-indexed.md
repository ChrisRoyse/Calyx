# PH54 · T06 — FSV: index key in same txn as data key; crash = no half-indexed row

| Field | Value |
|---|---|
| **Phase** | PH54 — Secondary indexes (btree/inverted) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/tests/ph54_fsv.rs` (≤500) |
| **Depends on** | T04, T05 |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/20 §1`, `dbprdplans/04 §5/§7` |

## Goal

A seeded integration test proving the phase FSV gate: (1) an index key is
written at the **same** MVCC sequence number as its data key (read both at
one seq and verify); (2) a simulated crash-before-index-write leaves no
half-indexed row visible at any seq; (3) range and point queries return correct
results; (4) a corrupted index is repaired by `index_rebuild` and queries
succeed again. No harness assertion counts — the bytes on disk and the
sequence numbers are the proof.

## Build (checklist of concrete, code-level steps)

- [ ] Create `tests/ph54_fsv.rs`; use path `/tmp/calyx-ph54-fsv-test`; clean at start.
- [ ] **Same-seq test:**
  - Create `orders` collection with btree index on `qty`.
  - `put_record(pk=1, {qty:5})`.
  - Read seq of the data CF entry for `pk=1` (via `vault.seq_for_key`).
  - Read seq of the index CF entry for the btree key corresponding to `(qty=5, pk=1)`.
  - Assert: `data_seq == index_seq`. They are the same because they were written
    in one `WriteBatch` (one group-commit entry).
- [ ] **Crash / no-half-indexed-row test:**
  - Use `FaultInjector` (test-only) that returns `Err` after the data key is
    appended to the `WriteBatch` but before `submit_batch` is called — simulating
    that the batch never reaches the WAL.
  - Open a fresh vault; inject fault; call `put_record(pk=2, {qty:9})` → `Err`.
  - Restart vault (open same path); call `get_record(pk=2)` → `None` (data
    absent); `btree_range(gte=9, lte=9)` → empty (index absent).
  - Verify: no partial state at any seq. Both absent = correct.
- [ ] **Range + point correctness test:**
  - Insert 10 records with `qty` values 0–9.
  - `btree_range(gte=3, lte=7)` → 5 results, pks `{3,4,5,6,7}`, asc order.
  - `btree_point(5)` → `[pk=5]`.
- [ ] **Rebuild-then-query test:**
  - Insert 5 records; manually tombstone the index key for `pk=3` (test helper).
  - `btree_range(1,10)` → 4 results (pk=3 missing from index).
  - `index_rebuild(...)` → `keys_added=1`.
  - `btree_range(1,10)` → 5 results (all present).
- [ ] All assertions are `assert_eq!` with exact expected values; seed RNG with
  `42`; inject `Clock` with fixed timestamp.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] The test is the FSV proof; it is self-contained and deterministic.
- [ ] proptest not required here (covered per-component in T01–T05).
- [ ] edge: the `FaultInjector` must be a `#[cfg(test)]` hook — not reachable in
  production (assert `#[cfg(not(test))]` the non-fault path compiles without the hook).
- [ ] fail-closed: if any assertion fails, print the seq numbers and the `xxd`
  of both CF entries before panicking (for aiwonder diagnosis).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cf/relational/` and `cf/index_btree/` SST shards post-test;
  vault WAL for the crash scenario.
- **Readback:**
  ```
  cargo test -p calyx-aster ph54_fsv -- --nocapture 2>&1 | tail -30
  xxd /tmp/calyx-ph54-fsv-test/cf/relational/000001.sst   | head -4
  xxd /tmp/calyx-ph54-fsv-test/cf/index_btree/000001.sst  | head -4
  # After crash scenario:
  xxd /tmp/calyx-ph54-fsv-crash/cf/relational/000001.sst  | head -2
  # Must show NO entry for pk=2 (batch never committed)
  ```
- **Prove:** Test prints "ph54 FSV: same-seq PASS, no-half-indexed PASS,
  range-correct PASS, rebuild PASS" and exits 0; `xxd` of crash vault shows
  no entry for pk=2 at any offset; same-seq assertion logs the seq number.
  Evidence posted to PH54 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (test output + `xxd` screenshots) attached to the PH54 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
