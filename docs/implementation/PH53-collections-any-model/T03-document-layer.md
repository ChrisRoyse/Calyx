# PH53 · T03 — Document layer: tuple-path keys + subtree prefix-read

| Field | Value |
|---|---|
| **Phase** | PH53 — Collections-as-any-model (relational/doc/KV/TS/blob) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/layers/document.rs` (≤500) |
| **Depends on** | T01, T02 (Layer trait) |
| **Axioms** | A15, A16, A19 |
| **PRD** | `dbprdplans/04 §2`, `dbprdplans/20 §2` |

## Goal

Implement the document paradigm layer using tuple-path key encoding
`(doc_id, p1, p2, …) → leaf` so that any path within a nested document maps to
exactly one ordered key, and a subtree prefix-read returns all keys under a
given path segment in one scan (`04 §2`). This is "nested docs + subtree
prefix-read" as a key-encoding layer; no separate document store is needed.

## Build (checklist of concrete, code-level steps)

- [ ] Define the document key schema (discriminant `0x02`):
  ```
  key = 0x02 | collection_id (8B BE) | doc_id (16B) | path_segments...
  ```
  Each path segment is length-prefixed `(u8 len | utf8 bytes)`. The leaf value
  is bincode-encoded `FieldValue`. A document root maps to the key with no
  path segments; child fields append one segment per level.
- [ ] Implement `put_doc(col: &Collection, doc_id: DocId, doc: &Value) -> Result<()>`:
  - Recursively flatten the nested document into `(path → leaf_value)` pairs.
  - Encode each path as a tuple-path key; write all pairs in **one** group-commit
    WAL batch (atomically, same seq).
  - Write Ledger stub entry in the same batch (A15).
- [ ] Implement `get_doc(col: &Collection, doc_id: DocId) -> Result<Option<Value>>`:
  - Prefix-scan all keys with `0x02 | collection_id | doc_id`.
  - Reconstruct the nested `Value` tree from path→leaf pairs.
  - Return `None` if no keys found (doc absent, not error).
- [ ] Implement `get_subtree(col: &Collection, doc_id: DocId, path: &[&str]) -> Result<Option<Value>>`:
  - Prefix-scan all keys starting with `0x02 | collection_id | doc_id | path...`.
  - Return the sub-tree rooted at `path`; `None` if absent.
- [ ] Implement `delete_doc(col: &Collection, doc_id: DocId) -> Result<()>`:
  - Write tombstones for all keys under the doc_id prefix in one WAL batch.
- [ ] `SchemaLess` is the natural mode; if `SchemaFull`, validate top-level field
  names against the schema on `put_doc` → `CALYX_SCHEMA_VIOLATION` on unknown
  required fields.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `put_doc(id="d1", {"a":{"b":42},"c":7})` → `get_doc("d1")` returns
  `{"a":{"b":42},"c":7}` with all fields; `get_subtree("d1", ["a"])` returns
  `{"b":42}` only.
- [ ] proptest: `get_doc(id, put_doc(id, v)) == Some(v)` for arbitrary nested JSON
  values (depth ≤5, 5 fields per level).
- [ ] edge (≥3): (1) single-field flat doc `{"x":1}` round-trips; (2) `get_doc`
  on absent `doc_id` → `None`; (3) `delete_doc` then `get_doc` → `None`; (4)
  `get_subtree` on absent path → `None`.
- [ ] fail-closed: corrupt leaf value bytes → `CALYX_ASTER_CORRUPT_SHARD`;
  path segment length overflows `u8` (> 255 bytes) → `CALYX_INVALID_ARGUMENT`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cf/document/` SST shard.
- **Readback:**
  ```
  calyx doc put --vault /home/croyse/calyx/test-vault --collection docs --id d1 --data '{"a":{"b":42},"c":7}'
  calyx doc get --vault /home/croyse/calyx/test-vault --collection docs --id d1
  calyx doc subtree --vault /home/croyse/calyx/test-vault --collection docs --id d1 --prefix a
  xxd /home/croyse/calyx/test-vault/cf/document/000001.sst | head -8
  ```
- **Prove:** The `xxd` output shows `0x02` discriminant; `get_subtree("d1",["a"])`
  returns `{"b":42}` and does NOT include `"c"`; after vault restart, full doc
  round-trips byte-exact. Evidence posted to PH53 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH53 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
