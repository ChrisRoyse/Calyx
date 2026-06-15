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

## Implementation Evidence (2026-06-14)

Commit `14f85db` hardens the write path beyond the original T04 hook by gating
synthetic index maintenance on declared maintained indexes, avoiding old-row
reads when a collection has no indexes, making KV and time-series pseudo-fields
schema-aware, and making btree query liveness checks inspect the owning column
family instead of assuming every primary key is relational.

aiwonder verification:

- Targeted gate `issue460_aiwonder_targeted_after_btree_fix_v2`: passed
  `cargo fmt --all -- --check`, the <=500-line gate, `cargo check -p
  calyx-aster`, `cargo clippy -p calyx-aster -- -D warnings`, `cargo test -p
  calyx-aster layers::kv -- --nocapture` (9 passed), `cargo test -p
  calyx-aster index::btree -- --nocapture` (16 passed), and `cargo test -p
  calyx-aster --test issue460_kv_unsigned_ns_index_fsv -- --nocapture` (2
  passed).
- Workspace gate `issue460_aiwonder_workspace_gates_after_btree_fix`: passed
  `cargo check --workspace`, `cargo clippy --workspace -- -D warnings`, and
  `cargo test --workspace`.

Manual FSV source-of-truth readback on aiwonder:

- Atomic root:
  `/home/croyse/calyx/data/fsv-issue460-atomic-index-20260614T081001Z`.
  Artifact
  `/home/croyse/calyx/data/fsv-issue460-atomic-index-20260614T081001Z/issue460-atomic-index-write-fsv-artifact.json`
  has BLAKE3
  `cb91fe138fc7e655cd3ec4fd5f4a21b312b037b635977353dab1494b798a6c64`.
- WAL BLAKE3:
  `14b8698710eba536d03c372d38deba84495e3514a4b7cc5f13e2be6abbb723db`.
- Seq 2 relational SST `00000000000000000002-0001.sst` BLAKE3:
  `8f0bd03021d3464402343fabe369b1469ddab2171c1efd640ed133637476f5b1`.
- Seq 2 index SST `00000000000000000002-0002.sst` BLAKE3:
  `48b4d319fa7a341dcb7b31b08095a8f5088576284a41c77b23406cc58acb4a9b`.
- Seq 6 index tombstone SST `00000000000000000006-0002.sst` BLAKE3:
  `b20f42ae316b60e4bb5f16087eb34f9cdbd0b66b699e4ba5bd9f148a41ece04b`.
- Seq 10 KV SST `00000000000000000010-0001.sst` BLAKE3:
  `c29b989f08bc57b1e99e52436fb4d6f9bfe5e4e74d7354d023889e4f7ca6ab99`.
- Seq 10 index SST `00000000000000000010-0002.sst` BLAKE3:
  `9e0fd93bf2e287a39e8e3112c953bfce6c3aae702071e08d4874baa1b63197ea`.

The artifact and direct byte reads show the happy path absent before seq 2 and
present after seq 2 with both relational data and btree index bytes in the same
WAL batch (`ledger`, `relational`, `index_btree`, `time_index`). Reopen-at-seq
readback preserved both entries. The update edge wrote the old qty 42 index key
as `CALYX_ASTER_TOMBSTONE_V1` and the new qty 50 key at the same update path.
The no-index edge held index rows stable, the missing-field edge failed with
`CALYX_SCHEMA_VIOLATION`, the two-index edge produced both keys, and the KV edge
wrote namespace `u64::MAX` as sortable big-endian bytes with direct
`IndexBtree` entries for both `ns` and `key`.

Unsigned namespace ordering FSV:

- Namespace root:
  `/home/croyse/calyx/data/fsv-issue460-ns-index-20260614T081211Z`.
- Artifact
  `/home/croyse/calyx/data/fsv-issue460-ns-index-20260614T081211Z/issue460-ns-index-fsv-artifact.json`
  BLAKE3:
  `59788ddbf85cdabca8129985d4e9c3233822bc0b9459ee2acffd3469c84cca36`.
- Index SST BLAKE3 values:
  `4dd5cd697876385509832a631bb9cde44e21be47c82c0ae0681c07fe4bafae95`,
  `2604bea06fe1f11feb286f627565fdc8acee59aedcc620421a4c84ccbb1a73a5`,
  `df786f8aabd198f1dd108aa96cb4c269edab62d97df439760387662c0ed6f621`,
  `0ad8388a06f80d87ad7794594c71b0367d5fbd13bcd4d3e7e2a52800ab0d812b`,
  `bf9ab12dc5d4952762865baef4d6d51efd4c646cf6b7079a4501d9bab7b05b4b`,
  and `155a7322293037c6d6647fdb0c5d8411a5ae653283f992bfdfbd9ba9d74e4bdc`.

The namespace FSV wrote namespaces out of order
`[u64::MAX, 0, i64::MAX + 1, i64::MAX, 1]`, reopened the durable vault, scanned
`IndexBtree`, decoded the physical keys, and read back unsigned order
`[0, 1, 9223372036854775807, 9223372036854775808, 18446744073709551615]`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH54 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
