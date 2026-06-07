# PH53 · T02 — Relational layer: `(table,pk)→row` key encoding + CRUD

| Field | Value |
|---|---|
| **Phase** | PH53 — Collections-as-any-model (relational/doc/KV/TS/blob) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/layers/relational.rs` (≤500), `crates/calyx-aster/src/layers/mod.rs` (≤500) |
| **Depends on** | T01 (Collection struct), PH07 (CF key encoding, big-endian range scans) |
| **Axioms** | A15, A16, A19 |
| **PRD** | `dbprdplans/04 §2`, `dbprdplans/20 §2` |

## Goal

Implement the relational paradigm layer: encode rows as `(table, pk) → row`
in an ordered keyspace so range scans, point reads, and joins-by-reference are
correct and fast. This is the FoundationDB-style "relational = a key-encoding
layer over an ordered transactional KV" pattern (`04 §2`). The layer exposes
`put_record`, `get_record`, `range`, and `join_by_ref` over a `Records`-mode
`Collection`. Every write goes through the WAL group-commit batch (A15).

## Build (checklist of concrete, code-level steps)

- [ ] Define the relational key schema (big-endian, discriminant `0x01`):
  ```
  key = 0x01 | collection_id (8B BE) | pk_bytes (variable, length-prefixed u16 BE)
  value = schema_version (u16 BE) | row_bytes (bincode-encoded field map)
  ```
  Range over all rows in collection: prefix scan on `0x01 | collection_id`.
- [ ] Implement `put_record(col: &Collection, pk: &RecordKey, row: &Row) -> Result<()>`:
  - Validate `row` fields against `SchemaFull` if applicable; mismatch →
    `CALYX_SCHEMA_VIOLATION`.
  - Encode key + value; write in group-commit WAL batch via `AsterVault`.
  - Write a Ledger stub entry in the same batch (A15, real hash-chain at PH35).
- [ ] Implement `get_record(col: &Collection, pk: &RecordKey) -> Result<Option<Row>>`:
  - Point-read from the ordered keyspace at the exact encoded key.
  - Decode value bytes; fail closed with `CALYX_ASTER_CORRUPT_SHARD` on bad decode.
- [ ] Implement `range(col: &Collection, start: &RecordKey, end: &RecordKey, limit: usize) -> Result<Vec<Row>>`:
  - Prefix + range scan; keys are big-endian so ordered range is correct.
  - Respect the MVCC snapshot seq (pin to the caller's snapshot); no torn reads.
  - Return empty `Vec` (not error) if no rows in range.
- [ ] Implement `join_by_ref(col_a: &Collection, pk_a: &RecordKey, col_b_name: &str, fk_field: &str) -> Result<Option<Row>>`:
  - Read `row_a`, extract `fk_field` as a `RecordKey`, look up `col_b`.
  - All within the same snapshot; no partial read across seqs.
- [ ] Define `Layer` trait in `layers/mod.rs`:
  ```rust
  pub trait Layer: Send + Sync {
      fn collection_mode() -> CollectionMode;
      fn put(&self, col: &Collection, key: &[u8], value: &[u8]) -> Result<()>;
      fn get(&self, col: &Collection, key: &[u8]) -> Result<Option<Vec<u8>>>;
      fn range(&self, col: &Collection, start: &[u8], end: &[u8], limit: usize) -> Result<Vec<Vec<u8>>>;
  }
  ```
  `RelationalLayer` implements `Layer`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `put_record` on `pk=1u64`, `row={item:"bolt",qty:5i64}` → encoded
  key starts with `0x01` discriminant followed by big-endian collection_id bytes;
  `get_record(pk=1)` returns the identical row.
- [ ] unit: `range(start=0, end=100, limit=10)` on 5 inserted rows (pks 1,3,5,7,9)
  returns all 5 in ascending pk order.
- [ ] proptest: `get_record(pk, put_record(pk, row)) == Some(row)` for arbitrary
  `pk` and `row` (all field types).
- [ ] edge (≥3): (1) `get_record` on absent pk → `None` (not error); (2)
  `put_record` on `SchemaFull` collection with missing required field →
  `CALYX_SCHEMA_VIOLATION`; (3) `range` with `start > end` → empty vec;
  (4) `put_record` then vault restart → `get_record` returns same row byte-exact.
- [ ] fail-closed: corrupt SST value bytes → `CALYX_ASTER_CORRUPT_SHARD`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cf/relational/` SST shard in the vault.
- **Readback:**
  ```
  calyx record put --vault /home/croyse/calyx/test-vault --collection orders --pk 1 --data '{"item":"bolt","qty":5}'
  calyx record get --vault /home/croyse/calyx/test-vault --collection orders --pk 1
  xxd /home/croyse/calyx/test-vault/cf/relational/000001.sst | head -8
  ```
- **Prove:** The `xxd` output contains the `0x01` discriminant at byte 0 of the
  key; `get_record` after vault restart returns `{"item":"bolt","qty":5}` exactly;
  `range(0,100)` returns the row in sorted order. Evidence posted to PH53 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH53 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
