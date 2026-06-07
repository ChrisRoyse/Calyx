# PH54 · T04 — Atomic data+index write: maintenance hook in write path

| Field | Value |
|---|---|
| **Phase** | PH54 — Secondary indexes (btree/inverted) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/index/maintenance.rs` (≤500) |
| **Depends on** | T01, T02, T03, PH53 T02 (relational put_record write path) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/20 §1`, `dbprdplans/04 §2` |

## Goal

Inject an index-maintenance hook into every paradigm-layer write operation so
that the data key and **all** applicable index keys are written in exactly
**one** WAL group-commit batch at the same MVCC sequence number. This is the
FoundationDB atomicity invariant: there is no sequence number at which a data
key exists without its index key, or vice versa. A crash at any point leaves
both absent (old seq is durable) or both present (new seq is durable). No
half-indexed row is possible.

## Build (checklist of concrete, code-level steps)

- [ ] Define `IndexMaintenance` struct in `index/maintenance.rs`:
  ```rust
  pub struct IndexMaintenance {
      pub indexes: Vec<(IndexSpec, Box<dyn SecondaryIndex>)>,
  }
  ```
- [ ] Implement `IndexMaintenance::on_put(batch: &mut WriteBatch, col: &Collection, pk: &RecordKey, row: &Row) -> Result<()>`:
  - For each `(spec, index)` in `self.indexes`:
    - Extract the indexed field value from `row` for `spec.on_field`.
    - Call `index.encode_index_key(field_val, pk)`.
    - Append the index key (with empty value for btree; with weight for
      inverted) to `batch` — the **same** `WriteBatch` object that holds
      the data key.
  - Do NOT submit the batch; the caller submits once (one group-commit).
- [ ] Implement `IndexMaintenance::on_delete(batch: &mut WriteBatch, col: &Collection, pk: &RecordKey, old_row: &Row) -> Result<()>`:
  - For each index: append a tombstone for the old index key to `batch`.
- [ ] Wire `IndexMaintenance::on_put` into `relational::put_record`:
  - After encoding the data key into the `WriteBatch`, call
    `index_maintenance.on_put(batch, col, pk, row)`.
  - Submit the single batch.
- [ ] Wire into `document::put_doc`, `kv::kv_set`, `timeseries::ts_write` for
  collections that declare indexes (most TS/KV collections won't have
  inverted indexes; skip gracefully if `col.indexes` is empty).
- [ ] Add a read-path check in `get_record` / `kv_get`: if the index CF has an
  entry for a pk but the data CF does not (stale index), log a structured
  warning (`CALYX_INDEX_STALE_ENTRY`) and skip — do NOT panic.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `put_record` on a collection with a btree index on `qty` → after the
  call, both the data CF and the `index_btree` CF have entries at the **same**
  MVCC seq number (read seq from the vault's current sequence counter before
  and after; both keys appear at `seq_before + 1`).
- [ ] unit: `put_record` then check `WriteBatch` was submitted once (not twice).
- [ ] unit: delete a record → data key tombstoned + index key tombstoned at the
  same seq.
- [ ] proptest: for N random `put_record` calls, `btree_range(gte=MIN, lte=MAX)`
  returns exactly the N primary keys — no missing, no duplicates.
- [ ] edge (≥3): (1) collection with 0 indexes → `on_put` is a no-op, no extra
  CF writes; (2) field absent from row on a `SchemaFull` collection →
  `CALYX_SCHEMA_VIOLATION` before any write; (3) two indexes on same collection
  → both index keys in same batch.
- [ ] fail-closed: `WriteBatch` submission fails mid-way (injected `Err`) →
  neither data key nor index key is visible at the new seq; vault still readable
  at the old seq.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Both data CF (`cf/relational/`) and index CF (`cf/index_btree/`)
  show the same MVCC sequence number for a given write.
- **Readback:**
  ```
  calyx collection create --vault /home/croyse/calyx/test-vault --name atomic_test --mode records --index btree:qty:i64
  calyx record put --vault /home/croyse/calyx/test-vault --collection atomic_test --pk 7 --data '{"qty":42}'
  calyx readback --cf relational   --vault /home/croyse/calyx/test-vault --show-seq
  calyx readback --cf index_btree  --vault /home/croyse/calyx/test-vault --show-seq
  ```
- **Prove:** Both `readback --show-seq` outputs show the same `seq=N` for the
  write at `pk=7`. No seq gap between data write and index write.
  Evidence posted to PH54 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH54 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
