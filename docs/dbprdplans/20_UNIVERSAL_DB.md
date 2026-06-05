# 20 — The Universal Database (First Principles)

Implements A19. Calyx is **the** database for any project. It serves the root purpose of every database paradigm on one core, with the Association Engine as the crown layer that subsumes the search-shaped paradigms. Leapable is one consumer; ContextGraph, Polis, ClipCannon, and any future project are others.

## 1. The architecture that makes universality possible

Validated by the proven multi-model pattern (FoundationDB "ordered transactional core + layers"; ArcadeDB/NodeDB/RedDB "one engine, every model"; HTAP row+column). Calyx is three layers:

```
┌──────────────────────────────────────────────────────────────┐
│  ASSOCIATION ENGINE (the crown — Calyx's reason to exist)     │
│  constellations · Loom(DDA) · Assay(bits) · Lodestar(kernel)  │
│  Ward(Gτ) · Sextant(search/nav) · Oracle(consequences) · Ledger│
├──────────────────────────────────────────────────────────────┤
│  GENERAL DATA LAYER (table stakes — be a real database)        │
│  collections-as-any-model: relational · document · KV ·        │
│  columnar · graph · time-series · full-text · blob · wide-col  │
│  (FoundationDB-style key-encoding layers + secondary indexes)  │
├──────────────────────────────────────────────────────────────┤
│  ASTER CORE (04): ordered, transactional, LSM + columnar       │
│  WAL · MVCC · column families · ACID-per-vault · Forge math    │
└──────────────────────────────────────────────────────────────┘
```

**Key insight (FoundationDB layer concept):** an ordered, transactional key-value/columnar core can host *any* data model by encoding it into keys, and ACID transactions make those layers correct. Indexing is just "store the data key **and** an index key in one transaction." Aster is that core; every paradigm below is a layer; the Association Engine is the layer that makes Calyx unique.

## 2. First principles: every paradigm's root purpose, and how Calyx serves it

The *irreducible* job each paradigm does, and Calyx's fulfilment (per Doctrine §3):

| Paradigm | Root purpose (first principles) | Calyx mechanism | Subsumed or served |
|---|---|---|---|
| **Relational / OLTP** | typed tuples with constraints, transactions, joins, point+range reads | `Collection` of typed records; secondary indexes via index-key layer; ACID-per-vault transactions; join-by-reference | served (general data layer) |
| **Document** | nested, schemaless records, retrieve sub-trees | tuple-encoded path keys (root→leaf), prefix range-read returns a subtree | served |
| **Key-Value** | O(1) keyed state, TTL | KV collection over the ordered keyspace; TTL via retention | served |
| **Columnar / OLAP** | scan/aggregate huge columns fast | Aster stores columns in Arrow layout (SIMD scan from mmap); HTAP row+column side-by-side | served |
| **Graph** | nodes, typed edges, traversal, graph algos | the **association graph is native**: cross-term agreement/delta edges (Loom), asymmetric edges, SCC/MFVS/paths (`08`); CSR adjacency projection | **subsumed** |
| **Time-series** | ordered events, retention, downsample, range/rollup | range keys on time + scalar columns + temporal lenses + retention policy + continuous rollups | served |
| **Full-text search** | inverted term→doc match, BM25 | a **sparse lexical lens** (SPLADE/keyword) with inverted lists (SPANN) — search is just a lens (`10`) | **subsumed** |
| **Vector** | ANN over one embedding | a **dense lens** + per-slot ANN; a vector DB is a 1-lens Calyx | **subsumed (a special case)** |
| **Object / blob** | store/stream large payloads | input store + cold-tier sidecars on ZFS archive | served |
| **Wide-column** | sparse, billions of columns | the slot/scalar map is already sparse-by-design | served |
| **"ASK" / RAG** | answer NL questions across all the above | Sextant multi-lens search + `kernel_answer` + Oracle — **native**, no external pipeline | **native** |

**The thesis of universality:** the same multi-lens intelligence that powers DDA and the kernel *is* a superior search/graph/vector engine. You don't run Postgres + Elasticsearch + Pinecone + Neo4j + Influx beside Calyx; the search-shaped ones collapse into the Association Engine, and the storage-shaped ones (relational/doc/KV/columnar/TS/blob) are the general data layer beneath it — one engine, one transaction, one source of truth.

## 3. Collections — one container, any model (progressive enhancement)

Following the proven "a collection behaves as the model your workload needs" pattern, the Calyx unit of organization is a **Collection**:

```
Collection {
  name, mode: Records | Documents | KV | TimeSeries | Blob | Constellations,
  schema: Option<Schema>,            // SchemaFull | SchemaLess
  panel: Option<PanelRef>,           // 0 lenses = plain store; ≥1 lens = intelligence
  indexes: Vec<SecondaryIndex>,      // btree / inverted / ANN / kernel
  retention, txn_policy, tenant,
}
```

**Progressive enhancement is the killer property:**
- A collection with **0 lenses** is a plain, fast store (relational/document/KV/TS) — Calyx is "just a database."
- **Add a lens** → its records become constellations; you instantly get multi-lens search, DDA, bits, kernel, `Gτ`, and Oracle on that collection. **Intelligence is opt-in per collection, one `add_lens` call** (Doctrine §5).
- A query can span collections and modes in **one transaction** (relational filter → graph hop → vector similarity → aggregate) — the multi-model "single query, single pass" win.

So a project can put *all* its data in Calyx: ordinary tables/documents/KV next to constellation collections, turning intelligence on exactly where it wants it.

## 4. The universal query surface

One query language/planner (Sextant, `10`) over all modes, so a single statement can:
- filter typed records (relational predicates),
- traverse the association/edge graph,
- run multi-lens vector + full-text fusion (RRF/pipeline),
- aggregate columns (OLAP),
- range-scan time-series,
- and `ASK` — answer in natural language across all of it, kernel-grounded and provenanced.

No chaining across services; no glue; one pass, one transaction (the multi-model latency win, plus Calyx's grounding/guard/provenance the others lack).

## 5. Deployment profiles (same core, any project)

| Profile | Shape | For |
|---|---|---|
| **Embedded library** (`libcalyx`) | one `vault.calyx` dir, in-process, no server, CPU SIMD + optional ONNX GPU | end-user Leapable Vaults; laptops; offline-first apps |
| **Single-node server** (`calyxd`) | loopback/Access-gated, GPU lenses, Aster on ZFS | aiwonder; a project's primary DB |
| **Multi-tenant server** | per-vault isolation, shared lens store by content-id | a hosted product serving many projects/users |
| **Edge / offline** | embedded + CRDT-style sync when online (future) | distributed clients |

A **Project** is a config object: a deployment profile, one or more collections, a panel (or none), lenses, a guard policy, anchor kinds. That's the entire setup — the multi-lens plumbing is the database's job (Doctrine §5).

## 6. Project catalog (Calyx is universal; these are profiles)

| Project | Collections / panel | Notes |
|---|---|---|
| **Leapable.ai** | per-user Vault = constellation collection (text/code/legal lenses) | replaces SQLite/`sqlite-vec` only; their PostgreSQL control plane untouched (`15`) — a per-project deployment choice, not a Calyx limit |
| **ContextGraph** | text/code panel (N=13), memory constellations | becomes a Calyx profile; its mincut/witness/mejepa logic seeds Calyx engines |
| **Polis / socialmedia2** | civic panel (21 slots), people-constellations, `Gτ` matching | a Calyx profile |
| **ClipCannon** | media panel (N=7), video-clip constellations | a Calyx profile |
| **A new project** | declare collections + panel + profile | one config; full intelligence stack, zero plumbing |

For a **greenfield** project, Calyx can be the *sole* database — control-plane tables (general data layer) **and** intelligence (association engine) in one engine. "Keep PostgreSQL" is specific to Leapable's mature production system, not a statement about Calyx's capability: a new project would put everything in Calyx.

## 7. What Calyx deliberately does *not* chase

Honesty (bound stated plainly, Doctrine §9): Calyx serves the *root purpose* of every paradigm at the scale most projects need; it does **not** chase full ANSI-SQL compliance, extreme-write-contention distributed OLTP parity with a tuned Postgres/CockroachDB, or being a message broker. It is a universal *intelligence* database with a competent general data layer — "handle everything all databases do" for real projects, plus the AGI layer none of them have.

## 8. Why one engine beats the polyglot stack (the win, restated)

- **One source of truth** — data exists once; no replication lag between models; cross-model queries are atomic.
- **No glue** — no ETL/sync between a vector DB, a graph DB, a search engine, and an RDBMS.
- **The intelligence is native** — search/graph/vector are the Association Engine, so they come with bits, kernel, `Gτ`, provenance, and the Oracle for free.
- **One operational surface** — one binary, one backup, one metric stream (`16`).

**One sentence:** Calyx is a universal database because an ordered transactional columnar core hosts every paradigm as a layer, the general data layer covers the storage-shaped ones, and the Association Engine *subsumes* the search-shaped ones while adding DDA, the multi-scope kernel, `Gτ`, and the Oracle — so one engine is every database a project needs, plus the one no other database is: the engine of intelligence.

Sources (engineering only; theory is strictly Royse per Doctrine §2): [FoundationDB layer concept](https://apple.github.io/foundationdb/layer-concept.html) · [ArcadeDB multi-model](https://docs.arcadedb.com/arcadedb/concepts/multi-model.html) · [HTAP survey](https://arxiv.org/pdf/2404.15670) · NodeDB/RedDB (collections-as-any-model).
