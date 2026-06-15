# PH55 — Cross-model transactions + universal query surface

**Stage:** S12 — Universal data layer  ·  **Crate:** `calyx-sextant` (+ `calyx-aster`)  ·
**PRD roadmap:** A19, `20 §4/§8`, `17 §7.3`  ·  **Axioms:** A15, A16, A17, A19

## Objective

One statement, one transaction, across all data model modes — what used to need
five systems. Two complementary pieces:

**Cross-model transactions (Aster):** a single `WriteBatch` can span rows from
relational, document, KV, time-series, blob, and constellation CFs in the same
vault, committed at one MVCC sequence number. Single-writer-per-vault
serialization guarantees no partial read and no deadlock. An unbounded
cross-model write (no declared isolation or cost cap) is rejected.

**Universal query surface (Sextant):** extend the existing Stage 4
`calyx-sextant` search/planner stack. A `UniversalQuery` struct expresses, in
one statement: typed relational predicates + document subtree filters + KV point
lookups + time-series range + graph traversal (association graph hop) + multi-lens
vector/FTS fusion (RRF, from PH24/PH25) + OLAP aggregate + `ASK` (multi-lens +
`kernel_answer` + Oracle, grounded + provenanced). The planner enforces a
`cost_cap_ms`; plans exceeding it are rejected with `CALYX_PLANNER_COST_CAP`.
One query pass returns one provenanced result set.

## Dependencies

- **Phases:** PH54 (secondary indexes — data + index keys; relational filter
  pushdown uses btree indexes), PH26 (query planner + intent + explain — the
  Sextant planner extended here), PH09 (Aster vault txn machinery), PH24 (RRF
  fusion), PH25 (sparse inverted), PH33 (kernel_answer), PH35 (Ledger — `ASK`
  provenance)
- **Provides for:** PH62 (CLI `calyx ask` command calls universal query), PH72
  (streaming ingest uses cross-model txn for atomic multi-collection writes)

## Current state (build off what exists)

`calyx-sextant` already contains Stage 4 dense/sparse search, RRF fusion,
freshness/provenance, and the PH26 intent/explain planner. `calyx-aster` has
the PH53 paradigm layers and PH54 secondary indexes by the time this phase
starts. PH33 adds `kernel_answer`. The universal query surface stitches these
together and adds the cross-model txn guarantee. Sextant is declared as
`calyx-aster`'s peer — it imports from `calyx-aster` (the storage layers) and
from `calyx-lodestar`, `calyx-registry`; no circular dep.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/query/mod.rs` | `UniversalQuery`, `QueryMode` enum, `QueryResult`, `ProvenancedResult` |
| `crates/calyx-sextant/src/query/planner.rs` | Extend PH26 planner: cross-model plan, cost cap, `explain`, unbounded rejection |
| `crates/calyx-sextant/src/query/executor.rs` | Execute each query mode segment; merge results; one-pass |
| `crates/calyx-sextant/src/query/ask.rs` | `ASK` handler: multi-lens + `kernel_answer` + Oracle; provenance tag |
| `crates/calyx-aster/src/txn/cross_model.rs` | `CrossModelTxn`: single `WriteBatch` spanning multiple CFs + declared isolation + cost cap |
| `crates/calyx-aster/src/txn/mod.rs` | `TxnHandle`, serialization lock (single-writer-per-vault), deadlock prevention |
| `crates/calyx-sextant/tests/ph55_fsv.rs` | Integration FSV: cross-model txn + universal query + ASK on aiwonder |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `CrossModelTxn`: single-writer serialization, declared isolation, cost cap | PH09 WAL |
| T02 | Universal query struct + planner extension: cross-model plan + reject unbounded | PH26 T04 |
| T03 | Query executor: relational filter → graph hop → vector fusion → aggregate → TS range | T02 |
| T04 | `ASK`: multi-lens + `kernel_answer` + Oracle + provenance tag | T03, PH33, PH35 |
| T05 | FSV: one txn spans modes atomically; cross-model query one provenanced pass; unbounded plan rejected | T01, T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

```
# Cross-model txn: one seq, relational + constellation + KV in one write:
calyx txn begin --vault /home/croyse/calyx/test-vault --isolation serializable --cost-cap 100ms
calyx txn put-record  --collection orders --pk 10 --data '{"qty":1}'
calyx txn kv-set      --ns 1 --key session --val active
calyx txn put-cx      --collection cxs --input "order confirmed"
calyx txn commit
# Read all three back at the same seq:
calyx readback --cf relational    --vault /home/croyse/calyx/test-vault --show-seq
calyx readback --cf kv            --vault /home/croyse/calyx/test-vault --show-seq
calyx readback --cf slot_00       --vault /home/croyse/calyx/test-vault --show-seq
# Must all show seq=N (same commit).

# Universal query: relational filter → graph hop → vector similarity → aggregate
calyx query --vault /home/croyse/calyx/test-vault \
  --filter 'orders.qty >= 1' \
  --hop   'cxs.related' \
  --vec   'sem-self:nearest:5' \
  --agg   'count' \
  --explain

# ASK: grounded, provenanced
calyx ask --vault /home/croyse/calyx/test-vault "What orders were placed recently?" --provenance

# Unbounded plan rejection:
calyx query --vault /home/croyse/calyx/test-vault --filter 'orders.qty >= 0' \
  --no-cost-cap 2>&1 | grep CALYX_PLANNER_COST_CAP
```

Evidence (seq numbers, query results, provenance tag, rejection error) posted to PH55 GitHub issue.

## Risks / landmines

- Single-writer-per-vault means no concurrent txn. If two calyx-cli processes
  race, the second must block (not error) until the first commits or times out.
  Use a per-vault mutex in `TxnHandle`; timeout → `CALYX_TXN_TIMEOUT`.
- The `ASK` path calls `kernel_answer` (PH33) which may be expensive; it must
  honour the `cost_cap_ms` from the query's `TxnPolicy`.
- Planner cost estimation for cross-model plans is approximate; use a
  conservative upper bound; fail closed if estimate exceeds cap rather than
  running and hoping.
- Sextant imports from `calyx-aster`; `calyx-aster` must NOT import from
  `calyx-sextant` — verify no circular dep in `Cargo.toml` deps.
- Graph traversal (association graph hop) delegates to Loom cross-term edges
  (PH27); PH55 stubs with a direct CF lookup of `xterm` keys if PH27 is not
  yet done (parallelism note: S12 can proceed in parallel with S5–S8, so
  stub gracefully).
