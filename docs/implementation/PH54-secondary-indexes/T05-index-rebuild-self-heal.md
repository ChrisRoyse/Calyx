# PH54 · T05 — Index rebuild (self-heal): scan-and-re-index

| Field | Value |
|---|---|
| **Phase** | PH54 — Secondary indexes (btree/inverted) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/index/rebuild.rs` (≤500) |
| **Depends on** | T04 (IndexMaintenance.on_put), T02, T03 |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/20 §2`, `dbprdplans/22 §PH54` |

## Goal

Implement `index_rebuild` — a background-safe, bounded-memory scan of the data
CF that re-emits all index keys for a given collection and index spec. This is
the self-heal path: if a crash left index keys missing (a bug in the atomicity
guarantee, or a deliberately injected fault for FSV), `index_rebuild` detects
and repairs the gap without data loss. It operates in bounded batches (≤10K
rows per batch) and is safe to run concurrently with live reads (uses an MVCC
snapshot). It does NOT run during normal writes; it is operator-invoked or
Anneal-scheduled (PH44).

## Build (checklist of concrete, code-level steps)

- [ ] Implement `index_rebuild(vault: &AsterVault, col: &Collection, spec: &IndexSpec, batch_size: usize) -> Result<RebuildStats>`:
  - Pin an MVCC read snapshot at `vault.current_seq()`.
  - Scan the data CF for the collection in batches of `batch_size` rows (default
    `batch_size = 10_000`).
  - For each row, call `IndexMaintenance::on_put` to compute the expected index
    keys; write only the **missing** ones (skip if already present at any seq
    ≤ snapshot). Use a `WriteBatch` per scan batch; submit each batch atomically.
  - Tombstone index keys that exist in the index CF but have no corresponding
    data CF entry (stale entries from deleted rows).
  - Return `RebuildStats { rows_scanned: u64, keys_added: u64, stale_removed: u64, elapsed_ms: u64 }`.
- [ ] Implement `index_verify(vault: &AsterVault, col: &Collection, spec: &IndexSpec) -> Result<IndexHealth>`:
  - Same scan without writes; counts missing and stale entries.
  - Returns `IndexHealth { missing: u64, stale: u64, healthy: bool }`.
  - `healthy = missing == 0 && stale == 0`.
- [ ] Expose `index_rebuild` and `index_verify` in the CLI surface as
  `calyx index rebuild` and `calyx index verify` (CLI wiring in `calyx-cli` at
  PH62; here just the library function).
- [ ] Ensure the rebuild is idempotent: running it twice on an already-healthy
  index → `keys_added=0`, `stale_removed=0`, no writes.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit healthy: insert 5 records with btree index → `index_verify` → `missing=0,
  stale=0, healthy=true`.
- [ ] unit after simulated gap: write 5 records with index; then manually delete
  2 index keys from the index CF (injected corruption); `index_verify` →
  `missing=2`; `index_rebuild` → `keys_added=2`; `index_verify` again →
  `healthy=true`.
- [ ] unit stale: insert 3 records; delete 1 record (data tombstoned); skip index
  tombstone (injected gap); `index_verify` → `stale=1`; `index_rebuild` →
  `stale_removed=1`; `index_verify` → `healthy=true`.
- [ ] unit idempotent: run `index_rebuild` twice → second run `keys_added=0`.
- [ ] edge (≥3): (1) empty collection → `rows_scanned=0`, `keys_added=0`;
  (2) `batch_size=1` (single-row batches) → correct and complete; (3) collection
  with no indexes declared → `index_rebuild` returns `Ok(stats)` with all zeros.
- [ ] fail-closed: data CF corrupt row → `CALYX_ASTER_CORRUPT_SHARD`; rebuild
  aborts and reports the seq of the corrupt row.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `index_btree` CF after rebuild; `RebuildStats` struct printed.
- **Readback:**
  ```
  # Inject a gap (delete one index key):
  calyx debug delete-index-key --vault /home/croyse/calyx/test-vault --collection inv_orders --pk 3 --index qty
  # Verify gap:
  calyx index verify  --vault /home/croyse/calyx/test-vault --collection inv_orders --index qty
  # Rebuild:
  calyx index rebuild --vault /home/croyse/calyx/test-vault --collection inv_orders --index qty
  # Verify healthy:
  calyx index verify  --vault /home/croyse/calyx/test-vault --collection inv_orders --index qty
  calyx index range   --vault /home/croyse/calyx/test-vault --collection inv_orders --index qty --gte 1 --lte 10
  ```
- **Prove:** First `verify` shows `missing=1`; `rebuild` shows `keys_added=1`;
  second `verify` shows `healthy=true`; `range(1,10)` returns all 5 pks
  including pk=3. Evidence posted to PH54 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH54 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
