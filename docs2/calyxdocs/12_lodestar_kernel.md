# 12 — Lodestar: the Grounding-Kernel Engine

calyx-lodestar discovers the small **grounding kernel** of a corpus — the minimal
set of records (constellations) that carries the structure of the whole dataset —
scores candidates by centrality + groundedness, approximates a directed feedback
vertex set, and then turns the kernel into both a vector index and an answer path.
It is built on the graph primitives in `calyx-mincut` and `calyx-paths`
(see [17_graph_mincut_paths.md](17_graph_mincut_paths.md)).

**Source files covered:**

- `crates/calyx-lodestar/src/lib.rs`
- `crates/calyx-lodestar/src/error.rs`
- `crates/calyx-lodestar/src/kernel.rs`
- `crates/calyx-lodestar/src/kernel_graph.rs`
- `crates/calyx-lodestar/src/dfvs.rs`
- `crates/calyx-lodestar/src/grounding_gaps.rs`
- `crates/calyx-lodestar/src/recall_test.rs`
- `crates/calyx-lodestar/src/kernel_index.rs`
- `crates/calyx-lodestar/src/kernel_answer.rs`
- `crates/calyx-lodestar/src/incremental.rs`
- `crates/calyx-lodestar/src/temporal_kernel.rs`
- `crates/calyx-lodestar/src/scope.rs`
- `crates/calyx-lodestar/src/multi_scope.rs`
- `crates/calyx-lodestar/src/scope_cache.rs`
- `crates/calyx-lodestar/src/scope_report.rs`
- `crates/calyx-lodestar/src/hierarchical.rs`
- `crates/calyx-lodestar/src/kernel_health.rs`
- `crates/calyx-lodestar/src/label_propagation.rs`
- `crates/calyx-lodestar/src/loom_assoc.rs`
- `crates/calyx-lodestar/src/provenance.rs`
- `crates/calyx-lodestar/src/summarize.rs`
- `crates/calyx-lodestar/src/aster_bridge.rs`
- `crates/calyx-lodestar/Cargo.toml`

Cross-checked against the design plan `docs/dbprdplans/08_LODESTAR_KERNEL.md`.

---

## 1. What the "kernel" is in code

### 1.1 The `Kernel` struct

The kernel is the serializable struct `Kernel` (`src/kernel.rs`). It is the result
of running the discovery pipeline over a directed association graph
(`calyx_paths::AssocGraph`).

| Field | Type | Meaning |
|---|---|---|
| `kernel_id` | `CxId` | Content address over `panel_version`, `anchor_kind`, `corpus_shard_hash`, member ids, and kernel-graph ids (`kernel_id()` via `calyx_core::content_address`). |
| `panel_version` | `u64` | Version of the embedder/signal panel the kernel was built against. |
| `anchor_kind` | `Option<String>` | Domain label this kernel was grounded against (e.g. `"label:passed"`); `None` = ungrounded/global. |
| `corpus_shard_hash` | `[u8; 32]` | Identity of the corpus slice the kernel was computed over. |
| `members` | `Vec<CxId>` | The ≈1% — the grounding-kernel nodes (the approximate directed FVS). |
| `kernel_graph` | `Vec<CxId>` | The ≈10% intermediate "kernel graph" set (high-score candidates). |
| `groundedness` | `GroundednessReport` | Anchor-reach fraction + unanchored members. |
| `recall` | `RecallReport` | Trust metrics (kernel-only vs full recall, MFVS approx factor, τ\* estimate). |
| `built_at_millis` | `u64` | Build timestamp (caller-supplied via params). |
| `estimator_provenance` | `String` | Free-form provenance: DFVS method, approx factor, τ\* estimate/exactness, `trust=anchored|provisional`. |
| `warnings` | `Vec<String>` | Structured `CALYX_*` warnings (e.g. ungrounded, LP unavailable). |

`GroundednessReport` (`src/kernel.rs`): `reached_anchor: f32`, `unanchored_members: Vec<CxId>`.
The `unanchored_members` list is the actionable "cheapest grounding plan" — exactly
which members still need an outcome label.

`RecallReport` (`src/kernel.rs`): `kernel_only: f32`, `full: f32`, `ratio: f32`,
`approx_factor: f64`, `tau_star_estimate: usize`, `tau_star_exact: bool`,
`recall_test_params: Option<RecallTestParams>`, `corpus_name: Option<String>`,
`n_queries_tested: usize`, `held_out: Vec<CxId>`, `warning: Option<String>`.
`Default` sets `approx_factor = 1.0`, `tau_star_exact = true`, everything else 0/empty.

### 1.2 What it represents

The kernel encodes the claim operationalized in the plan (`docs/dbprdplans/08_…`):
the kernel ≈ a minimum feedback vertex set (MFVS) of the directed association graph.
Removing `members` makes the remainder acyclic ("bottoms out" at anchors), so the
rest of the corpus is reconstructable/answerable by association from the kernel.
The code does NOT assume any fixed 1%/10% ratio — `target_fraction` is a parameter
(default 0.10 for the kernel graph) and the MFVS size is whatever the approximation
yields (§2).

---

## 2. Kernel-discovery algorithm (exact steps)

The pipeline is `build_kernel_pipeline` (`src/kernel.rs`), which delegates to the
private `build_kernel_pipeline_with_adjustment`. Inputs: a `&AssocGraph`, a slice
of anchor `CxId`s, and `&KernelParams`. Steps:

1. **Empty short-circuit** — if `graph.is_empty()`, return `empty_kernel(params)`
   (members/kernel_graph empty, `reached_anchor = 1.0`, provenance `ph32::empty; trust=anchored`).
2. **SCC condensation** — `scc = tarjan_scc(graph)` (`calyx_mincut`). `SccResult`
   has `components: Vec<Vec<CxId>>` and `component_of: BTreeMap<CxId, usize>`.
3. **Betweenness** — `bet = betweenness(graph)?` (`calyx_mincut`, Brandes' algorithm
   over reciprocal edge weights `1/weight`, normalized by `(n-1)(n-2)`).
4. **Kernel-graph selection (≈10%)** — `select_kernel_graph(graph, &scc, &bet, anchors, &params.kernel_graph)`.
5. **Optional frequency adjustment** — the closure `adjust_heuristic` runs here. For
   `build_kernel_pipeline` it is a no-op; for `build_kernel_pipeline_with_frequency`
   it calls `apply_frequency_bonuses` (§2.4).
6. **LP rounding** — `rounded = lp_round_kernel_graph(&heuristic, &params.lp_round)` (§2.3).
7. **DFVS approximation (≈1%)** — `dfvs = dfvs_approx(&rounded)?` (§2.5).
8. **Grounding gaps** — `grounding_gaps_for_members(&dfvs.members, graph, anchors, params.kernel_graph.max_groundedness_distance)?` (§2.6).
9. **Warnings + provenance** — assemble warnings and `estimator_provenance`.
10. **Assemble `Kernel`** — `members = dfvs.members`, `kernel_graph = rounded.selected`,
    `recall` carries `approx_factor`/`tau_star_estimate`/`tau_star_exact` from the DFVS
    step (recall ratios are NOT measured here; they default to 0 until a recall test runs — §3.3).

### 2.1 Candidate scoring (`score_nodes` in `src/kernel_graph.rs`)

Each graph node gets a `NodeScore`:

| Field | Type | Computation |
|---|---|---|
| `id` | `CxId` | node id |
| `degree_score` | `f64` | `(in_degree + out_degree) / max_degree` over the graph (max ≥ 1) |
| `betweenness_score` | `f64` | `betweenness.get(id)` (0.0 if absent) |
| `groundedness_distance` | `Option<usize>` | BFS hops from node to nearest anchor, capped at `max_groundedness_distance` (`None` = no anchor reachable) |
| `groundedness_score` | `f64` | `1.0 − min(dist, max)/max` if reachable, else `0.0` |
| `frequency_bonus` | `f32` | 0.0 until `apply_frequency_bonuses` runs (§2.4) |
| `total_score` | `f64` | weighted sum (below) |

**Total score formula** (`score_nodes`):

```
total = degree_score      * degree_weight        (default 0.40)
      + betweenness_score  * betweenness_weight   (default 0.40)
      + groundedness_score * groundedness_weight  (default 0.20)
```

`validate_params` requires `degree_weight + betweenness_weight + groundedness_weight == 1.0`
(within 1e-6) and `target_fraction ∈ (0, 1]`. Scores are sorted descending by
`total_score`, ties broken by ascending `id` bytes (`sort_node_scores`).

**Groundedness distance** (`groundedness_distance`, `src/kernel_graph.rs`): BFS over
**out-edges** from the node; returns `Some(0)` if the node is itself an anchor,
`Some(hops)` on first reaching any anchor within `max_hops`, `None` if no anchor in
the graph or none reached. This is the "distance to an outside-language grounding point."

### 2.2 Selection (greedy top-fraction, NOT min-cut here)

`select_kernel_graph` selects the kernel graph deterministically:

- `take = ceil(target_fraction * node_count)`, clamped to `[1, node_count]`.
- Take the top `take` nodes by `total_score`.
- `build_kernel_graph` induces the subgraph on the selected ids (only edges whose
  both endpoints are selected; node weights copied from source).
- Errors: `KernelEmptyGraph` if the input graph is empty; `KernelInvalidParams` if
  the SCC result does not cover the graph; `KernelEmptyResult` if selection is empty
  on a non-empty graph.

The selection of the **kernel graph** is therefore a greedy top-k by composite score.
The MFVS (members) within that kernel graph is computed separately in §2.5.

### 2.3 LP rounding (`lp_round_kernel_graph`)

The LP-relaxation rounding step is **not wired to an external solver**. Two paths:

- `lp_round_kernel_graph(kernel_graph, params)`: if `params.fallback_to_heuristic`
  (default `true`), it clones the kernel graph, pushes warning
  `CALYX_KERNEL_LP_UNAVAILABLE: external LP solver not configured`, sets
  `lp_fraction = source_fraction`, and returns it unchanged. If `fallback_to_heuristic == false`,
  it returns `Err(KernelLpUnavailable)`.
- `lp_round_kernel_graph_from_solution(kernel_graph, params, &LpSolution)`: the real
  rounding path. Requires `solution.status == SolveStatus::Optimal` (else
  `KernelLpInfeasible` on `Infeasible`, or falls back for other statuses). Selects
  node `i` iff `solution.values[i] >= params.threshold` (default 0.5). `solution.values`
  length must equal node count; empty selection on a non-empty graph → `KernelEmptyResult`.

`LpRoundParams`: `threshold: f64` (default 0.5, must be finite in `[0,1]`),
`fallback_to_heuristic: bool` (default `true`).

**Gap:** in the default `build_kernel_pipeline`, LP rounding is the no-op fallback;
the MFVS therefore runs on the full greedy-selected kernel graph. A real LP solution
must be supplied via `lp_round_kernel_graph_from_solution` to exercise relaxation rounding.

### 2.4 Frequency bonus (temporal_kernel.rs)

`apply_frequency_bonuses(kernel_graph, vault)` (used by `build_kernel_pipeline_with_frequency`):

- For each node score, read its recurrence frequency from the Aster vault
  (`FREQUENCY_SCALAR` scalar; missing → frequency 0 + a `CALYX_LODESTAR_MISSING_FREQUENCY` warning).
- `frequency_bonus = frequency_kernel_bonus(freq)` = `ln(min(freq, FREQ_BONUS_MAX) + 1) / ln(FREQ_BONUS_MAX + 1)`, capped at 1.0; 0 if freq == 0.
- Update score: `total_score += FREQ_WEIGHT * (new_bonus − old_bonus)` (`FREQ_WEIGHT = 0.15`).
- Re-sort and re-take the top `selected.len()` nodes.

Constants: `FREQ_BONUS_MAX = 10_000`, `FREQ_WEIGHT = 0.15`. Recurring (frequent)
constellations are reinforced as stronger kernel candidates.

### 2.5 DFVS approximation (`dfvs_approx`, `src/dfvs.rs`)

This is the MFVS step. `dfvs_approx(&kernel_graph)` dispatches by graph shape:

1. Empty graph → empty result, `ExactOrGreedyLocalSearch`, approx 1.0.
2. **Tournament** (`is_tournament`: every node pair has an edge in at least one
   direction) → `tournament_2approx` (method `Tournament2Approx`, theoretical bound 2.0).
3. **Bounded genus** — `genus_estimate = ceil(max(E − 3V + 6, 0) / 6)` (0 if V<3). If
   `genus <= 2` → `bounded_genus_approx(graph, genus)` (method `BoundedGenus`, bound
   `genus + 1`). `genus > 100` → `DfvsGenusTooLarge`.
4. Otherwise → `solve_with_method(graph, ExactOrGreedyLocalSearch, None)`.

`solve_with_method` performs the actual vertex-set computation:

- **Exact** if `node_count <= EXACT_SEARCH_MAX_NODES` (= **20**): `exact_min_fvs` does
  exhaustive subset search by increasing size (smallest acyclic-after-removal subset).
- **Greedy** otherwise: `greedy_fvs` repeatedly removes the highest total-degree node
  until the residual graph is acyclic.
- **Local-search shrink** (`local_search_shrink`): for each member, try removing it
  from the FVS; if the graph stays acyclic without it, drop it (minimization).
- **Verify**: `verify_feedback_vertex_set` confirms removing the members leaves a DAG
  (each SCC size 1, no self-loops); failure → `DfvsVerificationFailed`.

Acyclicity test (`is_acyclic_after_removing`): rebuild the graph minus removed nodes
and their incident edges, treat any surviving self-loop as a cycle, and check every
`tarjan_scc` component has size 1.

`DfvsResult`: `members: Vec<CxId>`, `approx_factor: f64`, `tau_star_estimate: usize`,
`tau_star_exact: bool`, `method: DfvsMethod` (`ExactOrGreedyLocalSearch` |
`Tournament2Approx` | `BoundedGenus`).

**Approximation report** (`approximation_report`): if exact search was used →
`(member_count, exact=true, approx=1.0)`. Otherwise the lower bound is the count of
cyclic SCCs (`cyclic_scc_lower_bound`: SCCs of size > 1, or singletons with a
self-loop). `tau_star_estimate` = that lower bound; `tau_star_exact` = whether
`member_count == lower_bound`; `approx_factor` = `1.0` if tight, else
`max(member_count / lower_bound, theoretical_bound)`.

### 2.6 Stopping criteria & parameters

There is no iterative "stop"; the pipeline is a fixed staged sequence. The size
controls are:

| Parameter (`KernelGraphParams`) | Default | Role |
|---|---|---|
| `target_fraction` | `0.10` | fraction of nodes taken into the kernel graph |
| `max_groundedness_distance` | `3` | BFS hop cap for anchor reachability + grounding-gap check |
| `degree_weight` | `0.40` | weight on degree score |
| `betweenness_weight` | `0.40` | weight on betweenness score |
| `groundedness_weight` | `0.20` | weight on groundedness score |

`KernelParams` (top-level): `panel_version` (default 1), `anchor_kind`
(default `Some("synthetic")`), `corpus_shard_hash` (`[0;32]`), `built_at_millis` (0),
`kernel_graph: KernelGraphParams`, `lp_round: LpRoundParams`.

The MFVS itself "stops" when the residual graph is acyclic (greedy/exact),
then is minimized by local search.

### 2.7 Grounding gaps (`src/grounding_gaps.rs`)

`grounding_gaps(kernel, graph, anchors, max_anchor_dist)` (and the internal
`grounding_gaps_for_members`) produce a `GroundingGapReport`:

| Field | Type | Meaning |
|---|---|---|
| `gaps` | `Vec<CxId>` | members NOT within `max_anchor_dist` of any anchor (sorted) |
| `grounded_fraction` | `f32` | `grounded_count / member_count` (1.0 if no members) |
| `grounded_count` | `usize` | members reaching an anchor |
| `member_count` | `usize` | total members |
| `max_anchor_dist` | `usize` | the distance bound used |
| `warning` | `Option<String>` | `CALYX_KERNEL_UNGROUNDED` if grounded_fraction == 0 with members present |

A member is grounded iff `groundedness_distance` (out-edge BFS to an anchor) returns
`Some(_)` within the bound. If `anchors` is empty, every member is a gap.
Public constant: `CALYX_KERNEL_UNGROUNDED`.

---

## 3. The kernel as index and as answer path

### 3.1 Kernel index (`src/kernel_index.rs`)

`build_kernel_index(kernel, embeddings)` builds a `KernelIndex` over the kernel
members only:

- For each `member`, fetch its embedding via the `EmbeddingStore` trait
  (`fn embedding(&self, cx_id) -> Result<Option<Vec<f32>>>`; impl'd for
  `BTreeMap<CxId, Vec<f32>>`). Missing → `KernelEmbeddingMissing`.
- `KernelIndex::from_rows` validates rows (`validate_rows`: non-empty, consistent
  non-zero dim, no duplicate ids, all-finite) and builds an HNSW index
  (`calyx_sextant::HnswIndex`) under a fixed `KERNEL_SLOT = SlotId(u16::MAX)` and
  seed `HNSW_SEED = 0x4c4f444553544152` ("LODESTAR").

`KernelIndex` fields: `kernel_id: CxId`, `dim: usize`, private `rows: Vec<KernelVectorRow>`,
private `hnsw: HnswIndex`. `KernelVectorRow { cx_id, vector }`. Public methods:
`rows()`, `filter_to_nodes(allowed)` (rebuilds an index restricted to a node set).

`kernel_search(index, query_vec, top_k)` validates the query dim and finiteness,
wraps it as `SlotVector::Dense`, runs `hnsw.search`, and returns `Vec<(CxId, f32)>`.
This is the **kernel-first funnel**: the query hits the tiny kernel index first.

**Persistence:** `write_kernel_index` / `load_kernel_index` serialize a
`KernelIndexSnapshot { format_version = FORMAT_VERSION (=1), kernel_id, dim, rows }`
as pretty JSON through the `KernelStore` trait (`write_index_bytes` / `read_index_bytes`).
`FsKernelStore` writes atomically (tmp + rename) to
`<root>/idx/kernel/<kernel_id>/index.json` (and `kernel.json` for the artifact).

### 3.2 Answer path (`src/kernel_answer.rs`)

`kernel_answer(kernel_index, graph, query_cx, query_vec, anchored_kernel_nodes, max_hops)`
routes a query through the kernel then walks association edges. Steps:

1. **Find an answerable anchored entry point** (`nearest_answerable_anchored_path`):
   - Error `KernelNoAnchoredNode` if `anchored_kernel_nodes` is empty.
   - `kernel_search(index, query_vec, rows().len())` ranks all kernel nodes by
     similarity to the query.
   - Iterate candidates in similarity order, keeping only those in
     `anchored_kernel_nodes` AND present in the graph.
   - If a candidate equals `query_cx`, return it directly (`vec![anchor]`).
   - Else attempt `reach(graph, anchor, query_cx, max_hops)` (`calyx_paths`,
     bidirectional BFS). First successful path wins. `Ok(None)` and
     `CALYX_PATHS_MAX_HOPS` errors are remembered and retried with the next candidate;
     other path errors propagate immediately.
   - If no anchored candidate was even seen → `KernelNoAnchoredNode`; if seen but none
     reachable → the recorded `KernelAnswerNoPath` (or max-hops) error.
2. **Direct hit:** if the path is length 1 (anchor == query), return an `AnswerPath`
   with no hops and `total_score = 1.0`.
3. **Walk the path** (`answer_hops_with`): for each consecutive `(from, to)` pair,
   look up the directed `edge_weight`, set `hop_index`, and compute
   `hop_score = attenuate(edge_weight, hop_index)` = `edge_weight * 0.9^hop_index`
   (`calyx_paths::attenuate`, `HOP_DECAY = 0.9`). Each hop is score-validated
   (finite, ≥ 0) and stamped with a `LedgerRef`.
4. `total_score = sum(hop_score)`; assemble the `AnswerPath`.

`AnswerHop`: `from, to: CxId`, `edge_weight: f32`, `hop_index: u32`, `hop_score: f32`,
`ledger_ref: LedgerRef`. `AnswerPath`: `query_cx`, `anchor_kernel_node`, `hops: Vec<AnswerHop>`,
`total_score: f32`, `provenance: Vec<LedgerRef>` (collected from the hops).

`kernel_answer` uses a deterministic **stub** ledger ref (`stub_ledger_ref`: blake3
over `"ph33-kernel-answer-ledger-stub"`, from/to/hop). The provenanced variant
`kernel_answer_with_ledger` writes real entries: per-hop `append_answer_hop_entry`
plus a final `append_answer_complete_entry` (see §6).

### 3.3 Recall test / trust gate (`src/recall_test.rs`)

The recall figures in `RecallReport` are not produced by the build pipeline; they are
measured by `kernel_recall_test`:

- `RecallTestParams`: `held_out_fraction` (default 0.1), `top_k` (default 10),
  `rng_seed` (default 42; 0 → use clock), `min_recall_ratio` (default **0.95**).
- Deterministically sample held-out queries: `sample_key = blake3(seed ‖ ordinal ‖ cx_id)`,
  sort, take `ceil(len * held_out_fraction)`.
- For each held-out query, run the **full** index (`AnnIndex` trait — impl'd for
  `HnswIndex` and `InMemoryAnnIndex` via cosine) and the **kernel** index at `top_k`;
  `recall@k = |kernel_hits ∩ full_hits| / |full_hits|`.
- `kernel_only = mean(recall@k)`, `full = 1.0`, `ratio = kernel_only / full`.
- `kernel_recall_gate` / `enforce_recall_gate`: `ratio < min_recall_ratio` →
  `RecallBelowGate { ratio, min }` (code `CALYX_KERNEL_RECALL_BELOW_GATE`). This is
  the A10 trust gate (default ≥ 0.95).

Public constant `CALYX_KERNEL_RECALL_BELOW_GATE`. `RecallTestReport` is a type alias
of `RecallReport`.

---

## 4. Maintenance / incremental update (`src/incremental.rs`)

`IncrementalKernelEval` holds a kernel + its graph + anchors + params + a set of
dirty SCC indices + a `stale` flag. It re-evaluates rather than recomputing from
scratch each event:

| Operation | Behavior | Result |
|---|---|---|
| `apply_edge_weight_change(src, dst, w)` | rebuild the graph with the one edge weight changed | `Unchanged` if no edge matched; else `Dirty { affected_sccs }` (SCCs of src/dst) |
| `apply_node_add(id, freq, edges)` | add node + its in/out edges; recompute SCCs | if the new node fell into an SCC of size > 1 → `FullRebuildRequired` and `stale = true`; else `Dirty { the new component }` |
| `apply_node_remove(id)` | rebuild graph without the node/edges, set `stale = true` | `KernelMemberRemoved { id }` if it was a member, else `FullRebuildRequired` |
| `rebuild_dirty()` | if any dirty SCCs or `stale`, re-run `build_kernel_pipeline`, then clear flags | `()` |

`NodeAddEdge` enum: `Out { dst, weight }` | `In { src, weight }`.
`IncrementalResult` (marked `#[must_use]`): `Dirty { affected_sccs: BTreeSet<usize> }`
| `FullRebuildRequired { reason: String }` | `KernelMemberRemoved { id: CxId }` |
`Unchanged`. Note: `rebuild_dirty` always re-runs the **full** pipeline over the whole
graph; the dirty-SCC tracking marks *whether* to rebuild, not a partial recompute.

The scope cache (`ScopeCache`, §5.4) is the other half of incrementality: re-asking a
scope at the same `(scope_hash, panel_version, anchor_identity, corpus_identity)` is a
cache hit; bumping `panel_version` invalidates via `invalidate_panel_version`.

---

## 5. Scopes, multi-scope, hierarchical, temporal

### 5.1 Scope (`src/scope.rs`)

`Scope` enum (matches plan §4b): `AllAssociations`, `Collection { id: CollectionId }`,
`Domain { anchor_kind: AnchorKind }`, `Subgraph { query: CxId, radius: usize }`,
`TimeWindow { t0: Ts, t1: Ts }`, `Tenant { id: TenantId }`, `Filter { expr: FilterExpr }`,
`Union { left, right }`, `Intersect { left, right }`.

`FilterExpr`: `Named { name }` | `MetadataEq { key, value }`.
`CollectionId(pub String)`, `TenantId(pub String)`.

`materialize_scope(scope, store)` resolves a scope to an `AssocGraph` via the
`AssocStore` trait (`full_graph`, `collection_nodes`, `domain_anchors`,
`time_window_nodes`, `tenant_nodes`, `filter_nodes`). `Domain` = nodes reachable from
the domain's anchors (forward BFS); `Subgraph` = out-edge BFS within `radius`;
`Union` = node+edge union; `Intersect` = node intersection induced over the full graph.
Recursion is capped at `MAX_SCOPE_DEPTH = 5` (`ScopeDepthExceeded`). `scope_hash` =
blake3 over `"calyx-lodestar-scope-v1"` + the scope JSON.

### 5.2 Multi-scope build + bridges (`src/multi_scope.rs`)

`build_kernel(store, scope, anchor_kind, params, cache)`: materialize the scope graph,
gather anchors for the scope's anchor kinds, build a `ScopeCacheKey`, return a cached
kernel on hit, otherwise run `build_kernel_pipeline` and cache it. **A Union kernel is
NOT `members_a ∪ members_b`** — the union scope materializes one graph and the same MFVS
pipeline runs over it (documented in the source comment). `mark_ungrounded_scope` tags a
scoped kernel `provisional` when `reached_anchor < UNGROUNDED_EPSILON (0.01)`.

`bridges(store, scope_a, scope_b, anchor_kind, params, cache)`: build both kernels, then
return members in **both** kernels (`members_a ∩ members_b`), ranked by descending
full-graph node weight (`bridge_members_by_frequency`). These are the constellations
that ground two domains at once.

`kernel_answer_scoped(...)`: restrict the kernel index and graph to the scope's nodes
(`filter_to_nodes`; empty → `KernelNoAnchoredNode`), filter anchored nodes to the scope,
then call `kernel_answer`.

### 5.3 Hierarchical (kernel-of-regions) (`src/hierarchical.rs`)

`build_hierarchical_kernel(store, scope, params, cache)` for huge scopes: build a
**region graph** (one node per `RegionDescriptor`, inter-region edge weight =
`Σ edge.weight / (|left|·|right|)`, capped at 1.0), run the kernel pipeline on it to
pick kernel regions, then drill into each selected region via a `Subgraph` kernel
around its `centroid_cx` (radius `drill_radius`). If no regions, fall back to a flat
`build_kernel`.

`HierarchicalKernelParams`: `max_regions` (64), `drill_radius` (2), `min_region_size`
(1), `anchor_kind`, `kernel_params`. `HierarchicalKernel`: `region_kernel: Kernel`,
`region_drilldowns: Vec<(RegionId, Kernel)>`; `all_members()` unions the drilldown
members. `RegionDescriptor`: `id: RegionId`, `centroid_cx: CxId`, `members: BTreeSet<CxId>`.
`RegionStore: AssocStore` adds `regions_for_scope`. Region node ids are content
addresses over `"calyx-lodestar-region-v1"` + region id.

### 5.4 Scope cache (`src/scope_cache.rs`)

`ScopeCache`: LRU `BTreeMap<ScopeCacheKey, Kernel>` (default `DEFAULT_MAX_ENTRIES = 128`).
`ScopeCacheKey`: `scope_hash`, `panel_version`, `anchor_identity` (blake3 of anchor
kinds + anchor ids, framed), `corpus_identity` (= corpus_shard_hash).
`get`/`insert` track hits/misses; eviction logs `CALYX_SCOPE_CACHE_EVICT` to stderr.
`invalidate_panel_version(old)` drops all entries of a version. `CacheStats`:
`hits, misses, current_size, max_entries`.

### 5.5 Temporal kernel (`src/temporal_kernel.rs`)

A separate, frequency-weighted kernel over a time window (used for drift / "what
mattered then"). `kernel_for_window(vault, window, k)`: find `active_cxids_in_window`
(recurrence occurrences whose `t_k` is in the half-open window), build a recurrence-only
graph, score all nodes (`target_fraction = 1.0`), apply frequency bonuses, and emit the
top-`k` `KernelWeight` rows.

`TimeWindow { start_secs, end_secs }` (half-open `[start, end)`; `start <= end` enforced,
`CALYX_LODESTAR_INVALID_WINDOW`). `KernelResult`: `scope: KernelScope::TimeWindow`,
`nodes: Vec<KernelWeight>`, `active_node_count`, `source_node_count`, `warnings`.
`KernelWeight`: `cx_id, rank, degree_score, betweenness_score, groundedness_score,
frequency, frequency_bonus, total_score`. `FrequencyRead`: `cx_id, frequency, missing`.
Constants/codes: `FREQ_BONUS_MAX`, `FREQ_WEIGHT`, `CALYX_LODESTAR_MISSING_FREQUENCY`,
`CALYX_LODESTAR_INVALID_FREQUENCY`, `CALYX_LODESTAR_INVALID_WINDOW`. (`lib.rs` also
re-exports `CALYX_LODESTAR_INVALID_FREQUENCY` as a frequency-error code.)

---

## 6. Provenance, label propagation, Loom + Aster integration

### 6.1 Ledger provenance (`src/provenance.rs`)

Writers append `calyx_ledger` entries (every payload checked by `RedactionPolicy`):

- `append_kernel_build_entry` → `EntryKind::Kernel`, subject `SubjectId::Kernel(kernel_id)`,
  payload: `kernel_id`, `members_hash`, `graph_seq`, `mfvs_approx_factor`,
  `mfvs_tau_star_estimate`, `mfvs_tau_star_exact`, `recall_ratio`. Actor service `"calyx-lodestar"`.
- `build_kernel_pipeline_with_ledger` builds + appends, returning `KernelBuildReceipt { kernel, ledger_ref }`.
- `append_answer_hop_entry` / `append_answer_complete_entry` → `EntryKind::Answer`,
  subject `SubjectId::Query(query_cx)`.
- `kernel_members_hash(kernel)` = blake3 over `"calyx-lodestar-kernel-members-v1"` + member bytes.

`AnswerHopEvidence` / `AnswerCompleteHopEvidence` carry per-hop from/to/edge_weight/
hop_index/hop_score (+ ledger_ref for the complete form).

### 6.2 Label propagation (`src/label_propagation.rs`)

`propagate_labels(graph, kernel_labels, max_iter, tol)` / `..._with_decay(..., decay_lambda)`:
grounded harmonic extension. Kernel nodes are clamped to their labels; non-kernel node
values iterate to the weighted average of symmetric neighbors until `max_delta <= tol`
(non-convergence → `CALYX_PROP_NOT_CONVERGED`). Output `PropagatedLabel`:
`node_id, label, confidence, hop_distance, provisional`; non-kernel `confidence` =
`label * exp(-decay_lambda * hop_distance)`. `DEFAULT_PROPAGATION_DECAY_LAMBDA = 0.5`.
Error type `PropagationError` with codes `CALYX_PROP_GRAPH_EMPTY`,
`CALYX_PROP_NO_KERNEL_NODES`, `CALYX_PROP_NOT_CONVERGED`, `CALYX_PROP_INVALID_INPUT`.
`propagate_labels_with_ledger` writes an `EntryKind::Kernel` provenance entry.
Type aliases: `NodeId = CxId`, `SparseGraph = AssocGraph`.

### 6.3 Loom → association graph (`src/loom_assoc.rs`)

`build_assoc_graph_from_loom(store, slot_nodes, directional_confidences)` derives the
directed association graph from Loom cross-terms: for each directional confidence it
finds the matching **agreement** cross-term (`canonical_pair`), maps slots to `CxId`s,
and emits `calyx_mincut::AgreementEdge { src, dst, agreement, directional_confidence }`.
The **edge weight is `agreement * directional_confidence`** (clamped agreement in `[0,1]`),
then `calyx_mincut::build_assoc_graph` builds the graph. Missing mappings/agreements/
confidences fail closed (`CALYX_KERNEL_LOOM_*` errors). Types: `LoomSlotNode`,
`LoomDirectionalConfidence`, `LoomAssocGraphInput`, `LoomAssocEdgeProvenance`.

### 6.4 Aster bridge + summarize (`src/aster_bridge.rs`, `src/summarize.rs`)

`AsterAssocSnapshot` implements `AssocStore` over a `PlainGraph` view of an `AsterVault`
collection (node props carry `embedding`, `ts`, `anchors`, `tenant`, `named_filters`,
`metadata`). `summarize(store, scope, params, ctx)` makes the kernel the universal
**structural** summarizer: it builds (or reuses) the scope kernel and returns
`SummarizeResult { scope_hash, kernel_ids, kernel_size, kernel_only_recall,
grounded_fraction, approx_factor, ledger_ref }`, appending a `SUMMARIZE_INVOKED`
ledger entry. Fail-closed codes: `CALYX_SCOPE_INVALID_TIME_WINDOW`,
`CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING` (require_grounded with grounded_fraction < 0.5),
`CALYX_TIMETRAVEL_BEFORE_HORIZON`. `summarize_with_recall` measures `kernel_only_recall`
against a supplied corpus/index. `summarize_vault_latest` / `summarize_vault_as_of`
drive it from a vault.

### 6.5 Kernel health (`src/kernel_health.rs`)

`kernel_health(kernel_id, store)` reads the persisted `kernel.json` artifact (never
recomputes) and assembles `KernelHealth`: `kernel_id, size, kernel_graph_size, recall
(KernelRecallHealth), grounded_fraction, unanchored_count, approx_factor,
tau_star_estimate, tau_star_exact, built_at_millis, panel_version, anchor_kind,
corpus_shard_hash (hex), trust (KernelTrust::Anchored|Provisional), warnings`.
`KernelRecallHealth`: `raw, tuned, ratio, min_recall_ratio, n_queries_tested,
pass_mode (RecallPassMode::Untested|Passed|BelowGate)`. Artifact I/O via
`KernelArtifactStore` (impl'd for `FsKernelStore`); `KERNEL_ARTIFACT_FORMAT_VERSION = 1`.
A missing/stale artifact fails closed (`KernelNotFound` / `KernelArtifactCodec`).

---

## 7. Error taxonomy (`src/error.rs`)

`LodestarError` (`thiserror`) with `Result<T> = std::result::Result<T, LodestarError>`
and a stable `code()` method. Roots all start `CALYX_*`:

| Variant | Code |
|---|---|
| `KernelEmptyGraph` | `CALYX_KERNEL_EMPTY_GRAPH` |
| `KernelInvalidParams { detail }` | `CALYX_KERNEL_INVALID_PARAMS` |
| `KernelLpUnavailable { detail }` | `CALYX_KERNEL_LP_UNAVAILABLE` |
| `KernelLpInfeasible { detail }` | `CALYX_KERNEL_LP_INFEASIBLE` |
| `KernelEmptyResult` | `CALYX_KERNEL_EMPTY_RESULT` |
| `KernelIndexNotFound { kernel_id }` | `CALYX_KERNEL_INDEX_NOT_FOUND` |
| `KernelNotFound { kernel_id }` | `CALYX_KERNEL_NOT_FOUND` |
| `KernelArtifactCodec { detail }` | `CALYX_KERNEL_ARTIFACT_CODEC` |
| `KernelDimMismatch { expected, actual }` | `CALYX_KERNEL_DIM_MISMATCH` |
| `KernelEmbeddingMissing { cx_id }` | `CALYX_KERNEL_EMBEDDING_MISSING` |
| `KernelIndexIo { detail }` | `CALYX_KERNEL_INDEX_IO` |
| `KernelIndexCodec { detail }` | `CALYX_KERNEL_INDEX_CODEC` |
| `KernelIndexBuild { detail }` | `CALYX_KERNEL_INDEX_BUILD` |
| `KernelNoAnchoredNode` | `CALYX_KERNEL_NO_ANCHORED_NODE` |
| `KernelAnswerNoPath { from, to }` | `CALYX_KERNEL_ANSWER_NO_PATH` |
| `KernelScoreInvalid { detail }` | `CALYX_KERNEL_SCORE_INVALID` |
| `KernelLoomSlotMappingMissing { … }` | `CALYX_KERNEL_LOOM_SLOT_MAPPING_MISSING` |
| `KernelLoomDirectionalConfidenceMissing { … }` | `CALYX_KERNEL_LOOM_DIRECTIONAL_CONFIDENCE_MISSING` |
| `KernelLoomAgreementMissing { … }` | `CALYX_KERNEL_LOOM_AGREEMENT_MISSING` |
| `KernelLoomAgreementInvalid { detail }` | `CALYX_KERNEL_LOOM_AGREEMENT_INVALID` |
| `RecallEmptyCorpus` | `CALYX_RECALL_EMPTY_CORPUS` |
| `RecallInvalidParams { detail }` | `CALYX_RECALL_INVALID_PARAMS` |
| `RecallBelowGate { ratio, min }` | `CALYX_KERNEL_RECALL_BELOW_GATE` |
| `CollectionNotFound { id }` | `CALYX_COLLECTION_NOT_FOUND` |
| `ScopeTemporalNotReady` | `CALYX_SCOPE_TEMPORAL_NOT_READY` |
| `ScopeDepthExceeded { depth, max }` | `CALYX_SCOPE_DEPTH_EXCEEDED` |
| `ScopeTenantNotFound { id }` | `CALYX_SCOPE_TENANT_NOT_FOUND` |
| `DfvsVerificationFailed { detail }` | `CALYX_DFVS_VERIFICATION_FAILED` |
| `DfvsGenusTooLarge { genus }` | `CALYX_DFVS_GENUS_TOO_LARGE` |
| `TemporalKernel { code, message }` | (carries its own code) |
| `Ledger { code, message }` | (from `CalyxError`) |
| `Graph { code, message }` | (from `PathsError` / `MincutError`) |

`From` conversions wrap `calyx_core::CalyxError` (→ `Ledger`),
`calyx_paths::PathsError` and `calyx_mincut::MincutError` (→ `Graph`, keeping the
upstream code, e.g. `CALYX_PATHS_MAX_HOPS`).

---

## 8. Integration points with mincut / paths

| Used symbol | Crate | Role in lodestar |
|---|---|---|
| `AssocGraph` / `AssocGraphBuilder` | `calyx-paths` | the directed association graph; node `frequency_weight` ∈ (0,∞), edge `weight` ∈ [0,1] |
| `tarjan_scc → SccResult { components, component_of }` | `calyx-mincut` | SCC condensation (pipeline step 2), incremental SCC tracking, acyclicity test, cyclic-SCC lower bound |
| `betweenness → BTreeMap<CxId, f64>` | `calyx-mincut` | Brandes centrality (reciprocal weights, `(n-1)(n-2)` norm) for candidate scoring |
| `build_assoc_graph`, `AgreementEdge` | `calyx-mincut` | build the graph from Loom cross-terms |
| `LpSolution`, `SolveStatus` | `calyx-mincut` | LP-rounding input for `lp_round_kernel_graph_from_solution` |
| `reach(graph, from, to, max_hops) → Option<Vec<CxId>>` | `calyx-paths` | bidirectional-BFS path from anchor to query in `kernel_answer` |
| `attenuate(score, hops) = score * 0.9^hops` | `calyx-paths` | per-hop answer score decay (`HOP_DECAY = 0.9`) |
| `HnswIndex` / `SextantIndex` | `calyx-sextant` | the kernel vector index and the recall-test full index |
| `AsterVault`, `PlainGraph`, recurrence CF | `calyx-aster` | frequency reads, the Aster→AssocStore bridge, summarize |
| `LedgerAppender`, `EntryKind`, `RedactionPolicy` | `calyx-ledger` | build/answer/summarize/propagation provenance |

---

## 9. Gaps / not covered

- **LP relaxation is not wired to a solver.** Default `build_kernel_pipeline` always
  takes the heuristic fallback in `lp_round_kernel_graph` (warns
  `CALYX_KERNEL_LP_UNAVAILABLE`) and runs the MFVS on the full greedy kernel graph.
  Real LP rounding requires the caller to compute an `LpSolution` and use
  `lp_round_kernel_graph_from_solution`. The plan's "LP-relaxation `O(log τ* log log τ*)`"
  approximation is therefore **not implemented** as an automatic step.
- **The build pipeline does not measure recall.** `Kernel.recall.kernel_only/full/ratio`
  are 0 until a separate `kernel_recall_test` runs (and is fed back, e.g. via `summarize_with_recall`).
- **`kernel_answer`'s default ledger ref is a stub** (`stub_ledger_ref`); only
  `kernel_answer_with_ledger` writes genuine ledger entries.
- **Incremental rebuild is whole-graph.** `rebuild_dirty` re-runs the full pipeline;
  dirty-SCC tracking decides *whether* to rebuild, not a localized recompute.
- **Aster time-travel summarization is partial.** Per the `summarize.rs` module docs, a
  production vault-embedded retention horizon / time-travel snapshot store is not fully
  wired; `summarize_as_of` emulates "as of t" by intersecting with `TimeWindow { 0, t }`.
- `frequency_bonus` is only applied through `build_kernel_pipeline_with_frequency` /
  the temporal kernel; the plain pipeline leaves it at 0.
