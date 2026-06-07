# PH54 · T01 — `SecondaryIndex` trait, `IndexSpec`, and btree key encoding

| Field | Value |
|---|---|
| **Phase** | PH54 — Secondary indexes (btree/inverted) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/index/mod.rs` (≤500), `crates/calyx-aster/src/index/btree.rs` (≤500) |
| **Depends on** | PH53 T01 (Collection + IndexSpec list), PH07 (big-endian key helpers) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/20 §1/§2`, `dbprdplans/04 §2` |

## Goal

Define the `SecondaryIndex` trait and `IndexSpec` type that every index
implementation satisfies, and implement the btree index key encoding
`(idx_id, field_val_BE, pk) → ∅` over the Aster ordered keyspace. The btree
index uses discriminant `0x10` in its key prefix. Big-endian encoding of the
`field_val` component is mandatory for correct range scans; negative I64 values
must use sign-flipped big-endian encoding so the natural key order matches
numeric order.

## Build (checklist of concrete, code-level steps)

- [ ] Define `IndexSpec` struct:
  ```rust
  pub struct IndexSpec {
      pub index_id: IndexId,   // u32 stable identifier within the collection
      pub name: String,
      pub kind: IndexKind,     // Btree | Inverted
      pub on_field: String,    // field name (relational) or path (document)
      pub field_type: FieldType,
  }
  ```
- [ ] Define `SecondaryIndex` trait in `index/mod.rs`:
  ```rust
  pub trait SecondaryIndex: Send + Sync {
      fn kind(&self) -> IndexKind;
      /// Encode the index key for a given field value + primary key.
      fn encode_index_key(&self, field_val: &FieldValue, pk: &RecordKey) -> Vec<u8>;
      /// Encode the index key prefix for scanning all entries with a given field value.
      fn encode_scan_prefix(&self, field_val: &FieldValue) -> Vec<u8>;
  }
  ```
- [ ] Implement `BtreeIndex` in `index/btree.rs`:
  - Key schema (discriminant `0x10`):
    ```
    key = 0x10 | collection_id (8B BE) | index_id (4B BE) | field_val_encoded | pk_bytes
    val = b"" (empty — existence is the signal)
    ```
  - `field_val_encoded` per type:
    - `FieldType::I64`: sign-flip XOR `0x8000_0000_0000_0000u64` then 8B BE.
    - `FieldType::F64`: IEEE 754 total-order encoding (flip sign bit + all bits
      if negative) then 8B BE.
    - `FieldType::Text`: first 64 bytes of UTF-8 (truncated, for prefix ordering),
      length-prefixed `u8`.
    - `FieldType::Timestamp`: u64 nanoseconds BE (already non-negative).
  - Implement `encode_index_key` and `encode_scan_prefix` using the above.
  - Implement `decode_index_key(key: &[u8]) -> Result<(FieldValue, RecordKey)>`:
    reverse the encoding; fail with `CALYX_ASTER_CORRUPT_SHARD` on bad bytes.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit I64: `encode_index_key(FieldValue::I64(-1), pk=0)` produces a key
  whose `field_val_encoded` component is `0x7fff_ffff_ffff_ffff` (sign-flip
  of -1); `encode_index_key(I64(0))` > `encode_index_key(I64(-1))` in byte order.
- [ ] unit I64 ordering: encode(-5), encode(-1), encode(0), encode(3) → byte-sort
  order equals numeric order.
- [ ] unit F64: `encode_index_key(F64(-1.0))` < `encode_index_key(F64(0.0))` in
  byte order; `encode_index_key(F64(0.0))` < `encode_index_key(F64(1.0))`.
- [ ] proptest: `decode_index_key(encode_index_key(v, pk)) == (v, pk)` for all
  `FieldType` variants.
- [ ] proptest ordering: for arbitrary `(a, b): (i64, i64)`, `a < b` iff
  `encode_index_key(I64(a), pk=0)` < `encode_index_key(I64(b), pk=0)` in
  lexicographic byte order.
- [ ] edge (≥3): (1) `I64::MIN` encodes and decodes correctly; (2) `I64::MAX`
  encodes and decodes correctly; (3) Text value = empty string → 0-byte
  `field_val_encoded`; (4) Text value truncated at 64 bytes has correct prefix.
- [ ] fail-closed: `decode_index_key` on a 3-byte slice → `CALYX_ASTER_CORRUPT_SHARD`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Unit test output on aiwonder.
- **Readback:**
  ```
  cargo test -p calyx-aster index::btree -- --nocapture 2>&1 | tail -20
  ```
- **Prove:** All ordering propty tests pass; golden bytes for I64(-1) printed and
  matched. Screenshot of test output posted to PH54 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH54 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
