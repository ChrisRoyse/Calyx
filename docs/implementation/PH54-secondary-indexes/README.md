# PH54 — Secondary indexes (btree/inverted)

**Stage:** S12 — Universal data layer  ·  **Crate:** `calyx-aster`  ·
**PRD roadmap:** `20 §1/§2`, A15  ·  **Axioms:** A15, A16

## Objective

Add secondary indexes to the general data layer collections. The core
invariant (FoundationDB pattern, `20 §1`): writing the data key and the
index key happen in **one transaction** — the same WAL group-commit batch,
the same MVCC sequence number. There is no window in which the data exists
without its index or the index exists without its data. A crash at any point
leaves either both present (at the new seq) or both absent (the old seq is
the durable state); a half-indexed row is impossible.

Two index types for PH54:
- **Btree (scalar/range):** for typed fields (`I64`, `F64`, `Timestamp`,
  `Text` prefix). Key encoding: `(idx_id, field_val_BE, pk) → ∅`. Supports
  point lookup and ordered range scan.
- **Inverted (term→doclist):** reuse the inverted-list machinery from PH25
  (sparse lens). Key encoding: `(idx_id, term_hash, pk) → weight`. Supports
  term-match and BM25 scoring.

ANN and kernel indexes already exist in `idx/` (PH23, PH33); PH54 adds only
btree and inverted. Index rebuild (self-heal) is the path for corrupted or
missing indexes: re-scan the data CF and re-write all index keys atomically
per batch.

## Dependencies

- **Phases:** PH53 (Collection + all paradigm layers — data key encoding
  must exist before index key encoding can mirror it), PH25 (inverted index
  posting lists — reuse, do not re-implement)
- **Provides for:** PH55 (cross-model query uses secondary indexes for
  relational filter pushdown and FTS), PH44 (self-heal rebuild reuses the
  index rebuild path)

## Current state (build off what exists)

`calyx-aster` has `cf/key.rs` (big-endian key encoding helpers), `cf/family.rs`
(`CfFamily` trait, CF routing), and `mvcc/` (seq-number management). PH25 built
inverted-list posting infrastructure in `calyx-sextant`. PH53 (T02–T07) will
have added paradigm-layer CF write paths. What does NOT yet exist: any
secondary-index infrastructure — no `index/btree.rs`, no `index/inverted.rs`,
no index-maintenance hooks in the write path, no `index_rebuild`.
`calyx-sextant` already contains Stage 4 search/query-planner surfaces; PH54
does not modify Sextant, but its Aster index outputs will be consumed by later
universal query work.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-aster/src/index/mod.rs` | `SecondaryIndex` trait + `IndexSpec`, dispatch |
| `crates/calyx-aster/src/index/btree.rs` | Btree index: key encoding `(idx_id,val_BE,pk)→∅`; range/point query |
| `crates/calyx-aster/src/index/inverted.rs` | Inverted index: `(idx_id,term_hash,pk)→weight`; term-match + BM25 |
| `crates/calyx-aster/src/index/maintenance.rs` | Atomic data+index write hook; inject into paradigm layer write path |
| `crates/calyx-aster/src/index/rebuild.rs` | Index rebuild: scan data CF → re-emit all index keys in batches |
| `crates/calyx-aster/tests/ph54_fsv.rs` | Integration test: atomic write + crash test + range/point + rebuild FSV |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `SecondaryIndex` trait, `IndexSpec`, btree key encoding | — |
| T02 | Btree index: range + point queries | T01 |
| T03 | Inverted index: term-match + BM25 (reuse PH25) | T01 |
| T04 | Atomic data+index write: maintenance hook in write path | T01, T02, T03 |
| T05 | Index rebuild (self-heal): scan-and-re-index | T04 |
| T06 | FSV: index key same txn as data key; crash = no half-indexed row | T04, T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

```
# Write a record with a btree index on field "qty":
calyx collection create --vault /home/croyse/calyx/test-vault --name inv_orders --mode records --index btree:qty:i64
calyx record put --vault /home/croyse/calyx/test-vault --collection inv_orders --pk 1 --data '{"qty":5}'

# Read the data key and the index key at the SAME seq:
calyx readback --cf relational  --vault /home/croyse/calyx/test-vault
calyx readback --cf index_btree --vault /home/croyse/calyx/test-vault
# Both must be present at the same MVCC seq; one seq for both writes.

# Range query via index:
calyx index range --vault /home/croyse/calyx/test-vault --collection inv_orders --index qty --gte 1 --lte 10

# Crash simulation: kill after data write, before index write (inject fault):
calyx debug fault-inject --after-data-write --before-index-write
# Re-open vault: index rebuild detects gap, re-indexes:
calyx index rebuild --vault /home/croyse/calyx/test-vault --collection inv_orders --index qty
calyx readback --cf index_btree --vault /home/croyse/calyx/test-vault
# Index now consistent; no half-indexed row.
```

Evidence (seq numbers + `xxd` bytes) posted to PH54 GitHub issue.

## Risks / landmines

- The group-commit batch must include both the data key and the index key
  in the same `WriteBatch`; if the WAL implementation commits batches
  sequentially (data batch, then index batch), the atomicity guarantee is
  broken — ensure both keys are appended to the **same** `WriteBatch` object.
- Inverted index from PH25 lives in `calyx-sextant`; PH54 either copies the
  posting-list write logic into `calyx-aster` or exposes a `write_posting`
  function callable from the aster write path. Avoid a cross-crate circular dep.
- Btree key encoding must be big-endian for all comparable types (I64 must use
  two's-complement sign-flip so negative values sort before positive).
- Index rebuild is O(n) over the data CF; it must operate in bounded-size
  batches (≤10K rows per batch) to avoid unbounded memory use before PH56.
