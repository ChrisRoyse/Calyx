# PH53 · T09 — Plain graph layer for 0-lens collections

| Field | Value |
|---|---|
| **Phase** | PH53 — Collections-as-any-model |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/plain_graph/` |
| **Depends on** | PH07/PH08 Aster CF keys, WAL, MVCC snapshots |
| **Axioms** | A15, A16, A19 |
| **PRD** | `04 §2`, `20 §2`, `19 §5` |

## Goal

Serve the graph paradigm for plain 0-lens collections: node properties, typed
directed edges, reverse edge lookup, CSR adjacency projection, and bounded
traversal over Aster's ordered transactional keyspace.

## Build

- Add a `graph` column family for byte-level FSV under `vault.calyx/cf/graph`.
- Encode rows with a leading graph discriminant plus collection id:
  - node: `(collection, node_id) -> props`
  - edge: `(collection, src, etype, dst) -> edge`
  - reverse: `(collection, dst, etype, src) -> forward_edge_key`
  - CSR: `(collection) -> derived projection`
- Write forward edge and reverse index row in one `write_cf_batch`.
- Traverse by graph CF prefix/range scans with hop and cost caps.
- Reuse `calyx-paths` graph validation where the CSR shape aligns.

## FSV

- Seed deterministic nodes `a,b,c,d` with a 3-hop `knows` chain plus a cycle
  and a second edge type.
- Read node, forward edge, reverse edge, and CSR rows back from `cf/graph`.
- Hand-compute 2-hop traversal from `a`: expected `[b,c]`.
- Edge cases: unknown edge type returns empty with no row change; max-hop fails
  closed with `CALYX_GRAPH_TRAVERSE_LIMIT`; empty graph traversal fails with
  `CALYX_GRAPH_NODE_NOT_FOUND`; injected WAL failure leaves neither forward nor
  reverse edge row.
- Save evidence under `/home/croyse/calyx/data/fsv-issue638-*` and independently
  read SST/WAL bytes after the test trigger.
