# 10 — Sextant: Multi-Lens Search & Navigation

> **Living-system role:** cognition / attention — navigating the web of associations and attending to what matters (A31 — DOCTRINE §1b)

The query engine. *"Optimized for being searched through by multi-embedder systems with full provenance."* Where the constellation architecture pays off: many lenses → many ways to search, navigate, and build hierarchical skills.

## 0. The universal query surface (A19) — one pass, all paradigms

Sextant is also Calyx's single query surface over **every** collection mode (`20`), so one statement can, in one transaction, do what used to need five systems:

| Sub-capability | Subsumes | Mechanism |
|---|---|---|
| typed predicates, joins-by-reference, aggregation | SQL/OLAP | btree/columnar scan over the general data layer |
| full-text term match + BM25 | Elasticsearch | a sparse lexical **lens** + inverted lists |
| vector ANN | Pinecone/pgvector | a dense **lens** + per-slot ANN |
| graph traversal / shortest-path / PageRank | Neo4j | the native association + cross-term graph (`06`/`08`) |
| time-range + rollup | InfluxDB/Timescale | range keys + temporal lenses |
| **`ASK`** (NL answer across all of the above) | a hand-built RAG stack | multi-lens fusion + `kernel_answer` + Oracle, grounded + provenanced |

The multi-lens intelligence makes the search/graph/vector capabilities **better** than the dedicated systems (bits-weighted fusion, kernel-first routing, `Gτ`-restricted, fully provenanced) — the universality win of `20 §2`. The rest of this doc details the constellation-specific power; it composes with plain-collection predicates in the same query.

**Retrieval efficiency is a facet of intelligence (A32, `27 §9`).** Fast, precise navigation is *usable intelligence per unit cost* — the same grounded structure that maximizes understanding (kernel-first routing `08`, differentiated lenses `07`, the association graph `06`) makes search fast. Anneal self-adjusts the fusion weights and index params online toward `J` (`27 §5`), so search gets sharper *and* faster as the system grows more intelligent — not as a separate optimization.

## 1. Query model

A Calyx query is **lens-aware**. The agent (or Sextant's planner) chooses which slots to search, how to fuse, what to return:

```
Query {
  vault, text/vector/cx_anchor,        // a query can be raw input, a produced vector, or an existing cx
  lenses: Auto | Explicit(Vec<SlotId>),// which slots participate
  fusion: RRF | WeightedRRF(profile) | KernelFirst | SingleLens(slot) | Pipeline,
  filters: scalar/anchor/metadata predicates,
  guard: Off | InRegionOnly(GuardProfile),  // restrict to Gτ-passing regions
  guard_vectors: Map<SlotId, SlotVector>,    // required for multi-slot InRegionOnly
  freshness: FreshDerived | StaleOk(seq_lag),
  k, rerank: Option<RerankSpec>,
  explain: bool,                       // attach per-lens + provenance breakdown
}
```

Multi-slot `InRegionOnly` is fail-closed without slot-aware `guard_vectors`.
The legacy top-level dense query vector is accepted only for single-slot guard
profiles.

## 2. Fusion strategies (batteries included)

| Strategy | Mechanism | Best for |
|---|---|---|
| `SingleLens(slot)` | one slot's ANN/inverted | lowest latency, specialized (code-only, speaker-only) |
| `RRF` | Reciprocal Rank Fusion across chosen slots: `Σ weight_i/(rank_i+60)` | general multi-lens (ContextGraph default) |
| `WeightedRRF(profile)` | named weight profile per intent (causal, code, entity, temporal…) | intent-tuned (14 ContextGraph profiles ship as defaults) |
| `KernelFirst` | Lodestar funnel: kernel ANN → expand by association | precision + sublinear on huge vaults (`08`) |
| `Pipeline` | sparse recall (SPANN/SPLADE) → multi-lens score → late-interaction rerank (ColBERT MaxSim) | maximum precision (ContextGraph E13→E1→E12 pattern) |

**Intent → strategy** auto-selected by a small classifier (absorbed from ContextGraph intent detection + Leapable `query-classifier`), overridable explicitly (A17).

## 3. Per-slot indexes (the substrate)

| Slot kind | Index | Embedded | Server (billion-scale) |
|---|---|---|---|
| Dense | HNSW (usearch-class) | in-RAM HNSW | **DiskANN** on-disk graph |
| Dense, asymmetric | dual HNSW (a/b) with directional boost | both in-RAM | dual DiskANN |
| Sparse (SPLADE/keyword) | inverted lists | in-RAM | **SPANN** (centroids RAM, lists on NVMe) |
| Multi (ColBERT) | token index + MaxSim | token HNSW | token DiskANN + segmented MaxSim rerank from raw token sidecars |
| Concat cross-term | HNSW on the joint key | RAM | DiskANN over materialized `xterm` Concat keys |
| Kernel | small dedicated ANN | RAM | RAM (tiny set) |
| Scalars | b-tree | b-tree | b-tree |

Each slot owns its index + quant config (Qdrant-style per-vector config), so search cost is paid only on participating slots.


## 4. Navigation modes (beyond top-k) — the "many ways with TCTs"

The constellation graph enables navigation primitives no vector DB offers:

| Mode | What it does | Built on |
|---|---|---|
| **Cross-lens agreement search** | find cx where lenses *agree* (high-confidence) or *disagree* (anomaly/blind-spot) | Loom agreement graph |
| **Asymmetric/causal traversal** | "what caused X?" vs "what did X cause?" — different results from dual lenses + hop-attenuation | asymmetric slots (`03 §4`) |
| **Hierarchical skills** | cluster constellations into named skills/regions (HDBSCAN), build a skill tree, search skill→sub-skill→cx; replay skill sequences | ContextGraph topic/skill discovery, `mejepa-train` skill linkage |
| **Constellation neighbors per lens** | K-NN in a chosen single lens space (compare how neighborhoods differ across lenses) | per-slot ANN |
| **Define / name** | given an index in one lens (point, region, or query), return the constellation the *other* lenses form there — that constellation **is** the term's grounded definition (Gärdenfors object-category, `02`) | cross-lens projection at an index |
| **Kernel walk** | answer by traversing the grounded kernel skeleton | Lodestar (`08`) |
| **Bridge search** | find cross-domain bridge constellations | Lodestar bridges |
| **Guarded search** | restrict results to in-`τ` (trusted) regions | Ward (`09`) |
| **Time/sequence nav** | session timelines, sequence traversal, freshness-decayed recall | temporal lenses, post-retrieval boosts |

**Hierarchical skills** deserves emphasis: because every datum is a constellation, Calyx can cluster them into named regions, name the regions (Lodestar/Compose), build a skill hierarchy, and let an agent search "by skill" then drill to specifics — the user's "hierarchical skills and all kinds of things for search and navigation." Generalized from ContextGraph's `skill_linkage`/`skill_sequence_discovery`.

## 5. Provenance on every hit (A15, `11`)

Every result row carries:
```
Hit {
  cx_id, fused_score,
  per_lens: [(slot, rank, raw_score, weight, contribution)],   // why it ranked
  cross_terms_used: [...],
  guard: Option<{pass, per_slot_cos}>,
  provenance: LedgerRef,                                        // input→lens→vector→answer
  freshness: { built_at_seq, stale_by },
}
```
`explain=true` makes the full breakdown queryable — an agent sees *which lens found this and how grounded it is*, not just a number.

## 6. Post-retrieval boosts & gates (not during retrieval)

Following ContextGraph's "temporal awareness without temporal bias" (**AP-60**, binding): temporal lenses **E2 recency / E3 periodic / E4 sequence** are **search/retrieval-only**, applied as **post-retrieval boosts** (weighted 50/35/15), **never dominant** and never during ANN retrieval, so recent/periodic items don't drown relevant ones (`25`). Causal gate: high-confidence causal hits ×1.10, low-confidence ×0.85 (tunable, calibrated). Database-level temporal *understanding* (recurrence, next-occurrence, time-travel) is separate from this retrieval boost (`25 §4b`).

## 7. Reranking

Optional cross-encoder rerank (reuse the resident GTE reranker `:8089` on aiwonder; ONNX cross-encoder embedded) as a final stage, candidate text request-scoped and never persisted (Leapable privacy rule). ColBERT MaxSim is the in-engine late-interaction reranker for the `Pipeline` strategy.

## 8. Performance

| Op | Target |
|---|---|
| SingleLens ANN @1e6 cx | p99 < 5 ms |
| RRF 6-lens @1e6 cx | p99 < 15 ms |
| KernelFirst @1e8 cx | p99 < 25 ms |
| Pipeline (recall→score→rerank) | p99 < 60 ms |
| All with `explain` | + ≤ 3 ms |

GPU-batched distance for multi-lens fan-out (Forge); CPU SIMD path for embedded vaults.

## 9. Sextant API (summary; full in `18`)

```
search(Query) -> [Hit]
neighbors(cx_id, slot, k) -> [Hit]               // per-lens neighborhood
agree / disagree(cx_id | query) -> [Hit]         // cross-lens consistency / anomaly
traverse(anchor_cx, direction, hops) -> path     // asymmetric/causal walk
skills(vault) -> SkillTree ; search_skill(skill, query) -> [Hit]
define(vault, index_in_lens) -> Constellation     // the term = the constellation other lenses form at this index
compare_lenses(query, [slots]) -> side-by-side rankings
```

**One sentence:** Sextant turns the panel into a navigable space — fuse lenses, walk causes, drill skills, route through the kernel, restrict to trusted regions — with every hit's provenance attached.

Sources: [DiskANN](https://www.microsoft.com/en-us/research/publication/diskann-fast-accurate-billion-point-nearest-neighbor-search-on-a-single-node/) · [SPANN](https://arxiv.org/abs/2111.08566) · [Qdrant per-vector index/quant](https://qdrant.tech/documentation/manage-data/collections/) · RRF (Cormack et al.).
