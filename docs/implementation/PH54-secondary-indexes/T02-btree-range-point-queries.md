# PH54 · T02 — Btree index: range + point queries

| Field | Value |
|---|---|
| **Phase** | PH54 — Secondary indexes (btree/inverted) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/index/btree.rs` (amend T01 file, ≤500 total) |
| **Depends on** | T01 (BtreeIndex key encoding) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/20 §1/§2` |

## Goal

Implement range and point queries over a btree secondary index. A range query
scans the ordered index keyspace between two `field_val_encoded` bounds and
returns the matching primary keys (the values are empty; existence is the
signal). Queries run within a pinned MVCC snapshot so there are no dirty reads.
Results include only rows whose data key is also present (an index key without
a matching data key is a stale index entry — handle gracefully by skipping).

## Build (checklist of concrete, code-level steps)

- [ ] Implement `btree_range(vault: &AsterVault, col: &Collection, spec: &IndexSpec, gte: Option<&FieldValue>, lte: Option<&FieldValue>, limit: usize) -> Result<Vec<RecordKey>>`:
  - Encode `start_key = encode_scan_prefix(gte)` (or the collection-id prefix
    if `None`).
  - Encode `end_key = encode_scan_prefix_inclusive(lte)` (or the next
    collection-id if `None`).
  - Range-scan the `index_btree` CF between `[start_key, end_key]` at the
    caller's snapshot seq.
  - For each index key found, decode the `RecordKey`; do a point-read on the
    data CF to verify presence (skip stale index entries silently).
  - Return `Vec<RecordKey>`, up to `limit`; `limit=0` → no limit (caller must
    bound).
- [ ] Implement `btree_point(vault: &AsterVault, col: &Collection, spec: &IndexSpec, val: &FieldValue) -> Result<Vec<RecordKey>>`:
  - Calls `btree_range(gte=Some(val), lte=Some(val), limit=0)`.
- [ ] Implement `btree_count(vault: &AsterVault, col: &Collection, spec: &IndexSpec, gte: Option<&FieldValue>, lte: Option<&FieldValue>) -> Result<u64>`:
  - Counts entries in range without materializing all PKs (iterator count only).
- [ ] Add `CF_INDEX_BTREE: &str = "index_btree"` to the CF registry; the key
  schema uses the `0x10` discriminant from T01.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: insert 5 records with `qty` values `{1, 3, 5, 7, 9}` (via relational
  layer + maintenance hook from T04 — stub here with direct index writes);
  `btree_range(gte=3, lte=7)` → `[pk_3, pk_5, pk_7]` in ascending order.
- [ ] unit: `btree_point(val=5)` → `[pk_5]`.
- [ ] unit: `btree_count(gte=1, lte=9)` → `5`.
- [ ] proptest: insert N records with random `I64` field values; `btree_range`
  returns exactly the records whose field value is in `[gte, lte]`, in sorted
  order, no duplicates.
- [ ] edge (≥3): (1) `btree_range` where no records match → empty vec;
  (2) `btree_range` with `limit=2` on 5 matching records → returns 2;
  (3) stale index key (data row deleted but index not yet compacted) → skipped,
  not returned; (4) `btree_point` on absent value → empty vec.
- [ ] fail-closed: `limit=usize::MAX` on a million-row index → bounded by
  calling code; no OOM assertion in the function itself (caller must set limit).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `index_btree` CF in the vault.
- **Readback:**
  ```
  calyx collection create --vault /home/croyse/calyx/test-vault --name rng_test --mode records --index btree:qty:i64
  for i in 1 3 5 7 9; do
    calyx record put --vault /home/croyse/calyx/test-vault --collection rng_test --pk $i --data "{\"qty\":$i}"
  done
  calyx index range --vault /home/croyse/calyx/test-vault --collection rng_test --index qty --gte 3 --lte 7
  xxd /home/croyse/calyx/test-vault/cf/index_btree/000001.sst | head -8
  ```
- **Prove:** `range(gte=3,lte=7)` returns pks `{3,5,7}` and no others; `xxd`
  shows `0x10` discriminant; big-endian field values sort in numeric order.
  Evidence posted to PH54 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH54 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
