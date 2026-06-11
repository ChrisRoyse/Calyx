# PH53 — Collections-as-any-model (relational/doc/KV/TS/blob/graph/OLAP)

**Stage:** S12 — Universal data layer  ·  **Crate:** `calyx-aster`  ·
**PRD roadmap:** A19, `20 §2/§3`, `04 §2`  ·  **Axioms:** A15, A19

## Objective

Implement the `Collection` container so Calyx behaves as any data model a
workload needs: relational records, nested documents, key-value state,
time-series events, blob payloads, graph edges, and columnar OLAP scans — all as key-encoding layers over the
Aster ordered-transactional core (FoundationDB-style). A collection with **0
lenses** is a plain fast store; adding ≥1 lens via `add_lens` upgrades it to a
Constellations-mode collection with the full Association Engine. Each paradigm's
root operation (`point / range / join-by-ref / aggregate / traverse / rollup`)
must round-trip by readback on aiwonder. `create_collection`,
`put_record`/`get_record`/`range`/`query` land here; schema enforcement
(`SchemaFull | SchemaLess`), dedup policy, temporal policy, retention, and
tenant isolation are set at creation and immutable thereafter.

## Dependencies

- **Phases:** PH09 (Aster vault CRUD, WAL, MVCC, CF key encoding — the ordered
  transactional core these layers sit on top of)
- **Provides for:** PH54 (secondary indexes need a live `Collection` with
  data-key encoding), PH55 (cross-model txn needs all paradigm layers), PH41
  (dedup policy wiring), PH40 (temporal policy wiring)

## Current state (build off what exists)

`calyx-aster` has: `wal/` (WAL + group-commit), `memtable.rs`, `sst/` (SSTable
r/w), `cf/` (`CfRouter`, `CfFamily`, key encoding), `mvcc/` (MVCC sequence
numbers + snapshot reads), `vault.rs` (`AsterVault<C>` implementing
`VaultStore`), `manifest/`, `compaction/`. The vault is wired end-to-end for
Constellation CRUD (PH09). `calyx-sextant` now has the Stage 4
search/navigation stack. What does **not** yet exist is the `Collection`
abstraction, the paradigm-specific key-encoding layers (relational, document,
KV, time-series, blob), and the collection query surface that sits above the CF
layer.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-aster/src/collection/mod.rs` | `Collection` struct + `CollectionMode` enum + `create_collection` |
| `crates/calyx-aster/src/collection/schema.rs` | `Schema`, `SchemaFull`, `SchemaLess`, field validation |
| `crates/calyx-aster/src/collection/policy.rs` | `DedupPolicy`, `TemporalPolicy`, `RetentionPolicy`, `TxnPolicy`, `TenantId` |
| `crates/calyx-aster/src/layers/relational.rs` | key-encoding `(table,pk)→row`; `put_record`/`get_record`/`range` |
| `crates/calyx-aster/src/layers/document.rs` | tuple-path keys `(doc_id,p1,p2,…)→leaf`; subtree prefix-read |
| `crates/calyx-aster/src/layers/kv.rs` | `(ns,key)→val` + TTL |
| `crates/calyx-aster/src/layers/timeseries.rs` | `(series,ts)→point` + rollups + retention |
| `crates/calyx-aster/src/layers/blob.rs` | chunked payload + manifest; cold-tier sidecar |
| `crates/calyx-aster/src/layers/mod.rs` | `Layer` trait; dispatch to paradigm |
| `crates/calyx-aster/src/plain_graph/` | plain 0-lens graph keys, reverse edge index, CSR projection, bounded traversal |
| `crates/calyx-aster/src/olap/` | columnar Arrow/SoA scan+aggregate root op over materialized slot chunks |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `Collection` struct, `CollectionMode`, schema, policies | — |
| T02 | Relational layer: `(table,pk)→row` key encoding + CRUD | T01 |
| T03 | Document layer: tuple-path keys + subtree prefix-read | T01 |
| T04 | KV layer: `(ns,key)→val` + TTL | T01 |
| T05 | Time-series layer: `(series,ts)→point` + rollups + retention | T01 |
| T06 | Blob layer: chunked payload + manifest | T01 |
| T07 | Progressive enhancement: 0-lens = plain store; `add_lens` upgrades to Constellations | T01, T02, T03, T04, T05, T06 |
| T08 | FSV: each paradigm's root op round-trips on aiwonder | T02, T03, T04, T05, T06, T07 |
| T09 | Plain graph layer: `(node)→props`, typed edge/reverse keys, CSR, traversal | T01 |
| T10 | Columnar/OLAP layer: Arrow chunk scan+aggregate with group-by | Slot column materialization |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Each paradigm's root operation must round-trip by byte-level readback on aiwonder:

```
# Relational: put a record, read it back
calyx collection create --vault /home/croyse/calyx/test-vault --name orders --mode records --schema full
calyx record put   --vault /home/croyse/calyx/test-vault --collection orders --pk 1 --data '{"item":"bolt","qty":5}'
calyx record get   --vault /home/croyse/calyx/test-vault --collection orders --pk 1
xxd /home/croyse/calyx/test-vault/cf/relational/000001.sst | head -4

# Document: put a nested doc, range a subtree
calyx collection create --vault /home/croyse/calyx/test-vault --name docs --mode documents
calyx doc put --vault /home/croyse/calyx/test-vault --collection docs --id d1 --data '{"a":{"b":42}}'
calyx doc subtree --vault /home/croyse/calyx/test-vault --collection docs --id d1 --prefix a
xxd /home/croyse/calyx/test-vault/cf/document/000001.sst | head -4

# KV: set + get + TTL expiry
calyx kv set --vault /home/croyse/calyx/test-vault --ns ns1 --key foo --val bar --ttl 60
calyx kv get --vault /home/croyse/calyx/test-vault --ns ns1 --key foo

# Time-series: write points, rollup
calyx ts write --vault /home/croyse/calyx/test-vault --series cpu --ts 1700000000 --val 0.42
calyx ts rollup --vault /home/croyse/calyx/test-vault --series cpu --window 1h

# Blob: put chunk manifest, read back
calyx blob put  --vault /home/croyse/calyx/test-vault --collection blobs --id b1 --file /tmp/testfile
calyx blob get  --vault /home/croyse/calyx/test-vault --collection blobs --id b1 --out /tmp/out
cmp /tmp/testfile /tmp/out && echo "blob round-trip OK"

# 0-lens = plain store; add_lens upgrades
calyx collection add-lens --vault /home/croyse/calyx/test-vault --collection orders --lens sem-self
```

Evidence (output + `xxd` bytes) posted to PH53 GitHub issue.

## Risks / landmines

- Key-space collisions between paradigm layers if key prefixes are not
  disjoint; use a leading 1-byte discriminant in every key per `04 §2`.
- TTL expiry for KV requires a background task or check-on-read; PH53 uses
  check-on-read only (full janitor in PH58).
- Blob chunking must handle partial-write failure; chunk manifest is written
  last and only when all chunk CFs are durable (WAL-fenced).
- Time-series rollup state must be stored in its own CF row (not recomputed on
  range-read) to avoid O(n) scan; rollup accumulator is written in the same
  group-commit txn as the point.
- `SchemaFull` enforcement: field-type mismatch → `CALYX_SCHEMA_VIOLATION`, not
  silent coercion (A16 / fail-closed).
- `add_lens` on an existing plain-mode collection triggers lazy backfill; PH53
  merely sets the `panel` reference in the `Collection` metadata; actual
  backfill is PH20.
