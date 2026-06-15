# 10. Graph Primitives & Lodestar Kernel (calyx-paths, calyx-mincut, calyx-lodestar)

This reference describes only what the source code does, traced to file paths and to specific functions/types. Where a property is not determinable from source it is marked "Not determined from source". Sibling references: see [09_loom_assay_dda.md](09_loom_assay_dda.md) for the Loom cross-term and agreement layer that feeds `calyx-lodestar::loom_assoc`, and the upstream Aster/Sextant references for the vault and HNSW index that the kernel index and Aster bridge build on.

## Source files covered

calyx-paths (`crates/calyx-paths/src/`):
- `lib.rs`, `graph.rs`, `traversal.rs`, `attenuation.rs`, `error.rs`

calyx-mincut (`crates/calyx-mincut/src/`):
- `lib.rs`, `graph_builder.rs`, `scc.rs`, `betweenness.rs`, `lp_scaffold.rs`, `spectral.rs`, `spectral_linalg.rs`, `error.rs`

calyx-lodestar (`crates/calyx-lodestar/src/`):
- `lib.rs`, `error.rs`, `kernel.rs`, `kernel_graph.rs`, `dfvs.rs`, `kernel_index.rs`, `kernel_answer.rs`, `kernel_health.rs`, `grounding_gaps.rs`, `scope.rs`, `scope_cache.rs`, `scope_report.rs`, `temporal_kernel.rs`, `recall_test.rs`, `multi_scope.rs`, `provenance.rs`, `incremental.rs`, `hierarchical.rs`, `label_propagation.rs`, `loom_assoc.rs`, `summarize.rs`, `aster_bridge.rs`

---

# Part A — calyx-paths (graph + traversal)

`lib.rs` declares the crate `#![deny(warnings)]` and re-exports `attenuate`/`deattenuate`, the error types, the graph types (`AssocGraph`, `AssocGraphBuilder`, `Edge`, `NodeEntry`), and traversal entry points (`bidirectional`, `reach`, `reach_scored`, `BidirectionalPath`).

## A.1 Association graph data structures (`graph.rs`)

| Type | Fields | Notes |
|------|--------|-------|
| `NodeEntry` | `id: CxId`, `frequency_weight: f32` | Copy; serde. |
| `Edge` | `src: usize`, `dst: usize`, `weight: f32` | Endpoints are dense node indices, not `CxId`. |
| `AssocGraph` | `nodes: Vec<NodeEntry>`, `edges: Vec<Edge>`, `adj: Vec<Range<usize>>`, `id_to_idx: HashMap<CxId, usize>` | Built form; `adj[i]` is the contiguous edge range for node `i` (CSR-style out-edge slices). |
| `AssocGraphBuilder` | `nodes`, `id_to_idx`, `edges` | Mutable accumulator; `Default`. |

This is a sparse, CSR-like directed graph. Edges are stored in one flat `Vec<Edge>` sorted so each node's out-edges are contiguous; `adj` holds the per-node `Range` into that vector (`out_edges_by_index` returns `&edges[adj[i]]`). `id_to_idx` maps `CxId` to dense index.

Builder validation:
- `validate_frequency_weight` requires finite and `> 0.0`; otherwise `GraphInvalidWeight { field: "frequency" }`.
- `validate_edge_weight` requires finite and within `0.0..=1.0`; otherwise `GraphInvalidWeight { field: "edge" }`.
- `add_node` rejects duplicate ids (`GraphDuplicateNode`); `add_edge` requires both endpoints already present (`GraphUnknownNode`).

`build()` (deterministic finalization):
1. Sort nodes by `id` (`node_order.sort_by_key(|(_, node)| node.id)`), producing an `old_to_new` index remap.
2. Deduplicate edges into a `BTreeMap<(src,dst), f32>`, keeping the **maximum** weight on collision (`current.max(edge.weight)`).
3. Materialize `edges` from the map (already ordered by `(src,dst)`), then compute `adj` via `build_ranges` (counting-sort prefix sums over `src`).
4. Rebuild `id_to_idx`.

Accessors of note: `out_neighbors`/`out_edges_by_index` (out-edges), `incoming_edges_by_index` (linear scan filtering `edge.dst == index`), `in_degree` (linear scan, O(E)), `out_degree`, `node_weight`, `edge_endpoints` (index→`CxId` pair), `require_node_index` (errors `GraphUnknownNode`).

## A.2 Hop attenuation (`attenuation.rs`)

The 0.9 hop-decay constant lives here and is the single source of truth for distance decay across both paths and the kernel answer scoring.

```rust
const HOP_DECAY: f32 = 0.9;
pub fn attenuate(base_score: f32, hops: u32) -> f32 { base_score * HOP_DECAY.powi(hops as i32) }
pub fn deattenuate(attenuated: f32, hops: u32) -> f32 { attenuated / HOP_DECAY.powi(hops as i32) }
```

So an attenuated score is `base * 0.9^hops`. `deattenuate` is the exact inverse.

## A.3 Traversal (`traversal.rs`)

| Function | Inputs | Output | Complexity |
|----------|--------|--------|-----------|
| `reach` | graph, src, dst, max_hops | `Option<Vec<CxId>>` (path) | Bidirectional BFS, ~O(V+E) |
| `bidirectional` | graph, question, answer, max_hops | `BidirectionalPath { forward, reverse }` | two `reach` calls |
| `reach_scored` | graph, src, max_hops | `Vec<(CxId, f32)>` best-score-per-node | BFS over edges bounded by `max_hops` |

`reach` algorithm (steps):
1. Empty graph → `NodeNotFound`. Resolve src/dst indices (`NodeNotFound` if absent). If `src == dst`, return `vec![src]`.
2. Run `shortest_path_indices` (bidirectional BFS): two `Frontier`s (forward over out-edges, backward over in-edges), always expanding the smaller frontier. When an expansion reaches a node already in the other side's `parents`, that node is the meeting point.
3. `reconstruct` walks forward parents from meet to src (reversed) then backward parents from meet to dst, yielding the index path.
4. Hops = `path.len() - 1`. If `hops > max_hops` → `MaxHops { required, max_hops }`. Otherwise map indices to `CxId`s.

Note: `reach` finds an unweighted shortest (fewest-hops) path; edge weights are ignored for path selection, then `max_hops` is enforced after the fact.

`reach_scored` algorithm: a BFS from `src` carrying a `ScoredReach { node, hops, raw_score }` where `raw_score` is the running **product of edge weights** (`current.raw_score * edge.weight`), seeded at `1.0`. A node is updated/re-enqueued only when the candidate's `ranked_score()` exceeds the known best, where `ranked_score = attenuate(raw_score, hops)` (i.e. `raw_score * 0.9^hops`). Traversal stops expanding once `hops == max_hops`. Final output excludes `src` and reports `(CxId, attenuate(raw_score, hops))` per reachable node, ordered by node index (`BTreeMap` keyed by index).

`BidirectionalPath { forward: Option<Vec<CxId>>, reverse: Option<Vec<CxId>> }`.

## A.4 Errors (`error.rs`)

`PathsError` variants and stable codes: `GraphDuplicateNode` (`CALYX_GRAPH_DUPLICATE_NODE`), `GraphUnknownNode` (`CALYX_GRAPH_UNKNOWN_NODE`), `GraphInvalidWeight` (`CALYX_GRAPH_INVALID_WEIGHT`), `MaxHops` (`CALYX_PATHS_MAX_HOPS`), `NodeNotFound` (`CALYX_PATHS_NODE_NOT_FOUND`). `code()` returns the constant string.

---

# Part B — calyx-mincut (graph primitives for grounding kernels)

`lib.rs` (`#![deny(warnings)]`) re-exports betweenness, the graph builder, the LP scaffold, SCC/condensation, and the spectral module.

## B.1 AssocGraph construction from grounding inputs (`graph_builder.rs`)

`build_assoc_graph(agreements, frequencies, citations) -> Result<AssocGraph>` assembles a `calyx_paths::AssocGraph` (returns `calyx_paths::Result`, i.e. `PathsError`).

| Input type | Fields | Role |
|------------|--------|------|
| `AgreementEdge` | `src`, `dst: CxId`, `agreement: f32`, `directional_confidence: f32` | Directed edge; weight = `agreement * directional_confidence`. |
| `FrequencyEntry` | `cx_id: CxId`, `frequency: f32` | Node weight (must be finite and `>= 1.0`). |
| `CitationEdge` | `src`, `dst: CxId` | Directed edge with fixed weight `1.0`. |

Steps: node weights collected in a `BTreeMap<CxId, f32>` (default `1.0` for agreement endpoints; frequency entries override via `.max(frequency)` and must be `>= 1.0` else `GraphInvalidWeight`). Citation endpoints must already exist (`GraphUnknownNode`). `agreement` and `directional_confidence` are validated to `[0,1]` (`validate_unit`). Then nodes are added, agreement edges added with the product weight, and citation edges added with weight `1.0`. Determinism comes from the `BTreeMap` ordering plus `AssocGraph::build`'s own node sort + edge dedup (max-weight).

## B.2 Tarjan SCC and condensation (`scc.rs`)

| Type | Fields |
|------|--------|
| `SccResult` | `components: Vec<Vec<CxId>>`, `component_of: BTreeMap<CxId, usize>` |
| `CondensedEdge` | `src_component`, `dst_component: usize`, `weight: f32` |
| `CondensedGraph` | `component_nodes: Vec<Vec<CxId>>`, `edges: Vec<CondensedEdge>` |

`tarjan_scc(graph) -> SccResult`: classic Tarjan via recursive `strong_connect`. `TarjanState` carries `next_index`, `indices: Vec<Option<usize>>`, `lowlinks`, `stack`, `on_stack`, `components`. Steps per node:
1. Assign `index`/`lowlink = next_index`, push to stack, mark `on_stack`.
2. For each out-edge: if dst unvisited, recurse and `lowlink = min(lowlink, lowlink[dst])`; else if dst `on_stack`, `lowlink = min(lowlink, index[dst])`.
3. If `lowlink[node] == index[node]`, pop the stack into a component until `node` is reached; the component's `CxId`s are sorted before being pushed.

`component_of` is derived by flattening components to `(CxId, component_index)`. Complexity O(V+E). (Recursive — stack depth is bounded by graph size; not determined from source whether there is an explicit overflow guard.)

`condensate(graph, scc) -> Result<CondensedGraph>`: validates the SCC covers exactly the graph node set (`validate_scc`, else `SccGraphMismatch`), then collapses each cross-component edge into a `BTreeMap<(src_comp,dst_comp), f32>` keeping **max** weight; intra-component edges are dropped. `CondensedGraph::is_dag()` runs a 3-color DFS (`has_cycle`) over components.

## B.3 Brandes betweenness (`betweenness.rs`)

| Function | Inputs | Output | Complexity |
|----------|--------|--------|-----------|
| `betweenness` | graph | `BTreeMap<CxId, f64>` | Brandes with Dijkstra inner loop, ~O(V·(V²+E)) (dense `min_unvisited` scan) |
| `betweenness_top_k` | graph, k | `Vec<(CxId, f64)>` sorted desc | wraps `betweenness`, truncates to `k` |

Empty graph → `BetweennessEmptyGraph`. This is **weighted** Brandes betweenness: edge "distance" is `1.0 / edge.weight` (edges with `weight <= 0.0` are skipped). `DIST_EPSILON = 1e-12` (`approx_eq`).

Steps (`shortest_paths_from` then `accumulate_dependencies` per source):
1. Dijkstra-style single-source shortest paths with `dist`, path-count `sigma` (seeded `sigma[source]=1.0`), and predecessor lists. `min_unvisited` linearly picks the unvisited finite-min node (ties by index). Shorter candidate resets `sigma` and predecessors; equal-within-epsilon candidate accumulates `sigma` and appends predecessor.
2. Dependency accumulation in reverse `stack` order: `delta[pred] += (sigma[pred]/sigma[node])*(1+delta[node])`; non-source nodes add `delta[node]` to global `scores`.
3. Normalization: divide each score by `(n-1)*(n-2)` when `n > 2`, else by `1.0` (directed normalization).

`betweenness_top_k` sorts by score descending, breaking ties by `CxId` bytes ascending, and truncates.

## B.4 LP scaffold (`lp_scaffold.rs`)

A serializable LP model, but **no solver is implemented here**. Enums: `ConstraintSense {Leq, Geq, Eq}`, `OptSense {Minimize, Maximize}`, `SolveStatus {Optimal, Infeasible, Unbounded, NotSolved}`.

| Type | Fields |
|------|--------|
| `LpVariable` | `id: usize`, `name: String`, `lb`, `ub: f64` |
| `LpConstraint` | `coeffs: Vec<(usize, f64)>`, `sense`, `rhs: f64` |
| `LpProblem` | `vars`, `constraints`, `objective: Vec<(usize,f64)>`, `sense` |
| `LpSolution` | `values: Vec<f64>`, `objective_value: f64`, `status` |

`LpVariable::new` rejects non-finite or `lb > ub` bounds (`LpInvalid`). `LpProblem::validate` checks dense variable ids (`var.id == index`), finite bounds, in-range objective/constraint variable references, and finite coefficients.

`mfvs_lp_problem(graph) -> LpProblem`: builds the **minimum feedback vertex set LP relaxation skeleton** — one variable `x_<id>` per node with bounds `[0,1]`, objective `sum(x_i)` to **minimize**, and **no constraints** (the cycle-cover constraints are not generated here). This is scaffolding consumed downstream by `calyx-lodestar::lp_round_kernel_graph_from_solution`.

## B.5 Spectral module (`spectral.rs`, `spectral_linalg.rs`)

Type aliases: `NodeId = CxId`, `SparseGraph = AssocGraph`. Constants: `EIGEN_EPS = 1e-6`, `DEFAULT_EIGEN_MAX_ITER = 256`.

| Type | Fields / role |
|------|---------------|
| `EigenPair` | `eigenvalue: f32`, `eigenvector: Vec<f32>` |
| `SpectralCacheKey` | `scope: String`, `panel_version: u64` (Ord) |
| `SpectralCacheEntry` | `centrality: Vec<(NodeId,f32)>`, `eigenpairs: Vec<EigenPair>`, `refreshed_at_seq: u64` |
| `SpectralCache` | `entries: BTreeMap<SpectralCacheKey, SpectralCacheEntry>` with insert/get/invalidate/`invalidate_scope` |

`SpectralError` variants/codes: `NotConverged` (`CALYX_SPECTRAL_NOT_CONVERGED`), `GraphTooSmall` (`CALYX_SPECTRAL_GRAPH_TOO_SMALL`), `SingularMatrix` (`CALYX_SPECTRAL_SINGULAR_MATRIX`).

| Function | What it does |
|----------|--------------|
| `eigenvector_centrality(graph, max_iter, tol)` | Power iteration on a shifted symmetric adjacency. Requires `>= 2` nodes. |
| `laplacian_eigenmaps(graph, k)` / `_with_max_iter` | Lanczos+Jacobi eigensolve of the graph Laplacian, returns smallest-`k` `EigenPair`s. |
| `gft_project(signal, eigenvectors)` | Graph Fourier transform: per-eigenvector dot products (asserts dimension match). |
| `gft_reconstruct(coefficients, eigenvectors)` | Inverse GFT: weighted sum of eigenvectors. |
| `spectral_gap(eigenmaps)` | `(eigenmaps[1].eigenvalue - eigenmaps[0].eigenvalue).max(0.0)`; `0.0` if fewer than 2. |

`sym_adjacency` symmetrizes by taking `max(weight)` over both directions. `laplacian_matrix` = `D - A` (degree diagonal minus symmetric adjacency). `eigenvector_centrality` iterates `shifted_mat_vec` (`A·v + v`, an implicit `(A+I)` shift to keep the dominant eigenvalue positive), normalizes, and stops when L2 step `< tol`; results are abs-normalized to the max and ranked desc (`ranked_scores`). On non-finite/near-zero norm → `SingularMatrix`; never converging → `NotConverged`.

`spectral_linalg.rs` (`JACOBI_TOL = 1e-6`, `JACOBI_MAX_ITER = 256`): `lanczos_eigen` builds an orthonormal Lanczos `basis` (with reseeding and re-orthogonalization), projects the matrix into that basis (`project_to_basis`), runs the **cyclic Jacobi** eigensolver (`jacobi_eigen` — rotate the max off-diagonal, periodic column re-orthonormalization every 10th step), and expands Ritz vectors back to full dimension. Eigenpairs are sorted ascending by eigenvalue, near-zero eigenvalues cleaned to `0.0` (`clean_zero`), and eigenvectors sign-oriented (`orient_vector` makes the first significant component positive). A trailing comment in `spectral.rs` states the design intent: "spectral centrality is structure-only; the MFVS kernel is outcome-anchored (A2). Centrality proposes candidates; grounding through oracle anchors confirms them."

## B.6 Errors (`error.rs`)

`MincutError`: `SccGraphMismatch` (`CALYX_SCC_GRAPH_MISMATCH`), `BetweennessEmptyGraph` (`CALYX_BETWEENNESS_EMPTY_GRAPH`), `LpInvalid` (`CALYX_LP_INVALID`), `NodeNotFound` (`CALYX_MINCUT_NODE_NOT_FOUND`).

---

# Part C — calyx-lodestar (kernel discovery & maintenance)

`lib.rs` (`#![deny(warnings)]`) declares the kernel-discovery modules and re-exports their public surface.

## C.1 Kernel-graph scoring (`kernel_graph.rs`)

`select_kernel_graph(graph, scc, betweenness, anchors, params)` produces the heuristic kernel candidate. Steps:
1. `validate_params` (below). Empty graph → `KernelEmptyGraph`. SCC must cover the graph (else `KernelInvalidParams`).
2. `score_nodes` computes a per-node `NodeScore` (below) and sorts via `sort_node_scores`.
3. `take = ceil(target_fraction * node_count)` clamped to `[1, node_count]`; the top-`take` ids become `selected`.
4. `build_kernel_graph` induces the subgraph over `selected` (only edges with both endpoints selected, weights preserved), computes `source_fraction = selected.len()/node_count`.

### Scoring parameters (`KernelGraphParams`, defaults)

| Field | Default | Meaning |
|-------|---------|---------|
| `target_fraction` | `0.10` | fraction of nodes kept as kernel graph |
| `max_groundedness_distance` | `3` | max hops searched to an anchor |
| `degree_weight` | `0.40` | weight of normalized degree |
| `betweenness_weight` | `0.40` | weight of betweenness |
| `groundedness_weight` | `0.20` | weight of groundedness proximity |

`validate_params`: `target_fraction` must be in `(0,1]` and the three weights must sum to `1.0` within `1e-6`, else `KernelInvalidParams`.

### NodeScore fields (`NodeScore` = `KernelNodeScore`)

| Field | Type | Computation |
|-------|------|-------------|
| `id` | `CxId` | node id |
| `degree_score` | `f64` | `(in_degree + out_degree) / max_degree` (max over graph, floored at 1) |
| `betweenness_score` | `f64` | Brandes betweenness for the node (default `0.0`) |
| `groundedness_distance` | `Option<usize>` | hops to nearest anchor within `max_groundedness_distance`, else `None` |
| `groundedness_score` | `f64` | `1 - (min(dist, max)/max)`; `0.0` when no anchor reached |
| `frequency_bonus` | `f32` | `0.0` here; set later by `apply_frequency_bonuses` |
| `total_score` | `f64` | see formula below |

Total score formula (`score_nodes`):

```rust
total = degree * degree_weight
      + bet    * betweenness_weight
      + gnd_score * groundedness_weight
```

`sort_node_scores` orders by `total_score` descending, breaking ties by `CxId` bytes ascending (deterministic).

`groundedness_distance(graph, node, anchors, max_hops)`: BFS over out-edges. Returns `Some(0)` if the node is itself an anchor; `None` if no anchors are present in the graph; otherwise the hop count to the first anchor reached within `max_hops`, else `None`.

### LP rounding (`LpRoundParams`, defaults `threshold = 0.5`, `fallback_to_heuristic = true`)

- `lp_round_kernel_graph(kernel_graph, params)`: **no external solver is wired**. With `fallback_to_heuristic = true` (default) it clones the heuristic, pushes warning `CALYX_KERNEL_LP_UNAVAILABLE: external LP solver not configured`, sets `lp_fraction = source_fraction`, and returns it. With the flag false it errors `KernelLpUnavailable`.
- `lp_round_kernel_graph_from_solution(kernel_graph, params, solution)`: the real rounding path given an externally produced `LpSolution`. `Optimal` → keep nodes whose value `>= threshold`; `Infeasible` → `KernelLpInfeasible`; any other status → fall back to the heuristic path. Value count must match node count (`KernelInvalidParams`). Empty selection over a non-empty graph → `KernelEmptyResult`. Sets `lp_fraction = selected/total`.

`validate_lp_params`: `threshold` must be finite and within `[0,1]`.

`KernelGraph` struct fields: `graph: AssocGraph`, `selected: Vec<CxId>`, `source_fraction: f32`, `lp_fraction: Option<f32>`, `params`, `scores: Vec<NodeScore>`, `warnings: Vec<String>`.

## C.2 DFVS approximation (`dfvs.rs`)

`DfvsMethod {ExactOrGreedyLocalSearch, Tournament2Approx, BoundedGenus}`. `EXACT_SEARCH_MAX_NODES = 20`.

`DfvsResult` fields: `members: Vec<CxId>` (the feedback vertex set kept as kernel members), `approx_factor: f64`, `tau_star_estimate: usize`, `tau_star_exact: bool`, `method: DfvsMethod`.

`dfvs_approx(kernel_graph)` dispatch:
1. Empty graph → empty result.
2. `is_tournament(graph)` (every unordered node pair has an edge one way) → `tournament_2approx` (theoretical bound `2.0`).
3. Else `genus = genus_estimate(graph)`; if `genus <= 2` → `bounded_genus_approx(graph, genus)` (bound `(genus+1).max(1)`; `genus > 100` → `DfvsGenusTooLarge`).
4. Else `ExactOrGreedyLocalSearch` (no theoretical bound passed).

`genus_estimate`: `0` when `V < 3`; otherwise `ceil(max(E - 3V + 6, 0) / 6)` (Euler-formula upper bound on graph genus).

`solve_with_method` (the core solver), steps:
1. If `node_count <= EXACT_SEARCH_MAX_NODES` (20), run `exact_min_fvs` (increasing-size subset enumeration: try size 0,1,2,… and test acyclicity after removal). Otherwise start from `greedy_fvs`.
2. `greedy_fvs`: repeatedly remove the not-yet-removed node with the **largest total degree** until the remainder is acyclic.
3. `local_search_shrink`: try removing each member from the set; if the graph stays acyclic without it, drop it (minimality pass). Members are then sorted.
4. `verify_feedback_vertex_set` re-checks acyclicity after removal; failure → `DfvsVerificationFailed`.
5. `approximation_report` derives `(tau_star_estimate, tau_star_exact, approx_factor)`.

Acyclicity test (`is_acyclic_after_removing`): rebuild the graph without removed nodes/edges (self-loops count as a cycle → not acyclic), then require every Tarjan SCC to be a singleton.

`approximation_report` logic: empty set → `(0, true, 1.0)`. Exact search used → `(member_count, true, 1.0)` (tau\* is exact). Otherwise compute a lower bound `cyclic_scc_lower_bound` (count of SCCs that are non-trivial — size `> 1` or containing a self-loop node, floored at 1); `observed_bound = member_count / lower_bound`; if `member_count == lower_bound` the bound is tight (`approx_factor = 1.0`), else `approx_factor = max(observed_bound, theoretical_bound)` (or `observed_bound` if no theoretical bound). `tau_star_estimate` is set to the lower bound, `tau_star_exact` to whether it was tight.

## C.3 Kernel pipeline (`kernel.rs`)

`Kernel` struct fields:

| Field | Type | Source |
|-------|------|--------|
| `kernel_id` | `CxId` | content address of params + members + kernel_graph |
| `panel_version` | `u64` | from params |
| `anchor_kind` | `Option<String>` | from params |
| `corpus_shard_hash` | `[u8;32]` | from params |
| `members` | `Vec<CxId>` | DFVS members |
| `kernel_graph` | `Vec<CxId>` | rounded `selected` node ids |
| `groundedness` | `GroundednessReport` | `reached_anchor: f32`, `unanchored_members: Vec<CxId>` |
| `recall` | `RecallReport` | mostly defaulted at build time (see below) |
| `built_at_millis` | `u64` | from params |
| `estimator_provenance` | `String` | `ph32::<method>; approx_factor=…; tau_star_estimate=…; tau_star_exact=…; trust=anchored|provisional` |
| `warnings` | `Vec<String>` | rounding + grounding warnings |

`build_kernel_pipeline(graph, anchors, params)` steps (via `build_kernel_pipeline_with_adjustment`):
1. Empty graph → `empty_kernel` (grounded fraction `1.0`, provenance `ph32::empty; trust=anchored`).
2. `scc = tarjan_scc(graph)`; `bet = betweenness(graph)`.
3. `heuristic = select_kernel_graph(...)`; apply optional adjustment closure (frequency bonuses for the `_with_frequency` variant).
4. `rounded = lp_round_kernel_graph(heuristic, lp_round_params)` (fallback path by default).
5. `dfvs = dfvs_approx(rounded)` → kernel `members`.
6. `grounding_gaps_for_members(members, graph, anchors, max_groundedness_distance)` → unanchored members.
7. Assemble `Kernel`: `groundedness_report` sets `reached_anchor = reached/members` (or `1.0` if empty). The only `RecallReport` fields populated at build time are `approx_factor`, `tau_star_estimate`, `tau_star_exact`; the rest default (recall is measured later, A10).
8. `kernel_id = CxId::from_bytes(content_address([panel_version_be, anchor_kind, corpus_shard_hash, members…, kernel_graph…]))`.

`build_kernel_pipeline_with_frequency(graph, anchors, params, vault)` injects `apply_frequency_bonuses` as the adjustment step (re-scores and re-selects with the vault frequency bonus before rounding).

Warnings/trust: if all members are unanchored, warning `CALYX_KERNEL_UNGROUNDED: all kernel members are provisional` is added and `trust=provisional` is recorded in provenance; otherwise `trust=anchored`.

`KernelParams` defaults: `panel_version: 1`, `anchor_kind: Some("synthetic")`, `corpus_shard_hash: [0;32]`, `built_at_millis: 0`, default `KernelGraphParams` and `LpRoundParams`.

## C.4 Grounding gaps (`grounding_gaps.rs`)

`grounding_gaps(kernel, graph, anchors, max_anchor_dist)` (and `grounding_gaps_for_members`) returns `GroundingGapReport { gaps, grounded_fraction, grounded_count, member_count, max_anchor_dist, warning }`. A member is "grounded" iff `groundedness_distance` finds an anchor within `max_anchor_dist`; ungrounded members become `gaps` (sorted). `grounded_fraction = grounded/members` (`1.0` if empty). If the fraction is exactly `0.0` for a non-empty kernel, `warning = "CALYX_KERNEL_UNGROUNDED: all kernel members are provisional"`. Constant `CALYX_KERNEL_UNGROUNDED`.

## C.5 Kernel index (`kernel_index.rs`)

`FORMAT_VERSION = 1`, `KERNEL_SLOT = SlotId::new(u16::MAX)`, `HNSW_SEED = 0x4c4f444553544152` ("LODESTAR" in ASCII).

Traits: `EmbeddingStore::embedding(cx_id) -> Option<Vec<f32>>` (blanket-impl'd for `BTreeMap<CxId, Vec<f32>>`); `KernelStore::{write_index_bytes, read_index_bytes}`. `FsKernelStore` lays out `<root>/idx/kernel/<kernel_id>/index.json` (and `kernel.json` for the artifact, see C.6) and writes atomically via a `.tmp` file + rename.

`KernelIndex { kernel_id, dim, rows: Vec<KernelVectorRow{cx_id, vector}>, hnsw: HnswIndex }`.
- `build_kernel_index(kernel, embeddings)`: one row per kernel member (missing embedding → `KernelEmbeddingMissing`); empty members → `KernelEmptyResult`.
- `KernelIndex::from_rows` validates rows (`validate_rows`: non-empty, non-zero dim, no duplicate `cx_id`, uniform dim, all-finite) and builds an HNSW (`build_hnsw`: inserts each row as a dense `SlotVector`, then `rebuild()`).
- `kernel_search(index, query_vec, top_k)`: dimension/finite checks then HNSW `search`, returning `Vec<(CxId, f32)>` (cx_id, score).
- `filter_to_nodes(allowed)` rebuilds the index restricted to an allowed node set.
- `write_kernel_index`/`load_kernel_index`: JSON `KernelIndexSnapshot { format_version, kernel_id, dim, rows }`, with version, kernel-id, and dim-consistency checks on load.

## C.6 Kernel health & artifact (`kernel_health.rs`)

`KERNEL_ARTIFACT_FORMAT_VERSION = 1`. `KernelArtifactStore::{write_kernel_bytes, read_kernel_bytes}` (impl'd for `FsKernelStore`, sibling `kernel.json`). The module doc states health is assembled by **reading** the persisted artifact, never recomputing recall/groundedness; a missing/stale artifact fails closed.

`write_kernel_artifact`/`read_kernel_artifact` wrap a `KernelArtifactSnapshot { format_version, kernel }`; mismatched version or stored kernel id → `KernelArtifactCodec`; absent → `KernelNotFound`.

`KernelTrust {Anchored, Provisional}`; `RecallPassMode {Untested, Passed, BelowGate}`.

`kernel_health(kernel_id, store)` reads the artifact then `kernel_health_from_kernel` (pure assembler). `KernelHealth` carries `size`, `kernel_graph_size`, `recall: KernelRecallHealth`, `grounded_fraction`, `unanchored_count`, `approx_factor`, `tau_star_estimate/exact`, `built_at_millis`, `panel_version`, `anchor_kind`, `corpus_shard_hash` (hex), `trust`, `warnings`.

`recall_health`: `min_recall_ratio` from the kernel's recall params (default `0.95`). `pass_mode`: `Untested` when `n_queries_tested == 0`; `Passed` when no recall warning and `ratio >= min_recall_ratio`; else `BelowGate`. `KernelRecallHealth { raw = recall.kernel_only, tuned = ratio, ratio, min_recall_ratio, n_queries_tested, pass_mode }`.

`trust_tag`: `Provisional` if a member carries a `CALYX_KERNEL_UNGROUNDED` warning or provenance contains `trust=provisional`; else `Anchored`.

## C.7 Kernel answer (`kernel_answer.rs`)

`AnswerPath { query_cx, anchor_kernel_node, hops: Vec<AnswerHop>, total_score: f32, provenance: Vec<LedgerRef> }`. `AnswerHop { from, to, edge_weight, hop_index: u32, hop_score: f32, ledger_ref }`.

`kernel_answer(index, graph, query_cx, query_vec, anchored_kernel_nodes, max_hops)`:
1. `nearest_answerable_anchored_path`: HNSW-search the kernel index for all rows, keep candidates that are in `anchored_kernel_nodes`, and for each (best-scored first) attempt `reach(graph, anchor, query_cx, max_hops)`. The first anchor with a path wins. No anchored candidate at all → `KernelNoAnchoredNode`; anchored but unreachable → first recorded error (`KernelAnswerNoPath` or a propagated `CALYX_PATHS_MAX_HOPS`).
2. If the path is just the anchor itself (`len == 1`), return an empty-hop path with `total_score = 1.0`.
3. Otherwise `answer_hops_with` builds one `AnswerHop` per consecutive pair: `edge_weight` looked up from the graph, `hop_index = idx`, and `hop_score = attenuate(edge_weight, hop_index)` (i.e. `edge_weight * 0.9^hop_index`, reusing the paths constant). `total_score = sum(hop_score)`. Scores validated finite and `>= 0`.

`kernel_answer_with_ledger` does the same but appends a per-hop ledger entry (`append_answer_hop_entry`) and a final complete-answer entry (`append_answer_complete_entry`). `kernel_answer` (no ledger) uses `stub_ledger_ref` — a BLAKE3 hash of `"ph33-kernel-answer-ledger-stub"` + from/to/hop_index — as a placeholder `LedgerRef`.

## C.8 Scope materialization (`scope.rs`)

`MAX_SCOPE_DEPTH = 5`. Newtypes `CollectionId(String)`, `TenantId(String)`. `FilterExpr { Named{name}, MetadataEq{key,value} }`.

`Scope` variants: `AllAssociations`, `Collection{id}`, `Domain{anchor_kind}`, `Subgraph{query, radius}`, `TimeWindow{t0, t1}`, `Tenant{id}`, `Filter{expr}`, `Union{left, right}`, `Intersect{left, right}`.

`AssocStore` trait methods: `full_graph`, `collection_nodes`, `domain_anchors`, `time_window_nodes`, `tenant_nodes`, `filter_nodes`.

`scope_hash(scope)`: BLAKE3 over domain tag `"calyx-lodestar-scope-v1"` + the JSON serialization of the scope → `[u8;32]`.

`materialize_scope` → `materialize_scope_at(scope, store, depth=1)`. Steps by variant:
- `AllAssociations` → `store.full_graph()`.
- `Collection` → `collection_nodes` (else `CollectionNotFound`) then `subgraph_from_nodes`.
- `Domain` → BFS-`reachable_from` the domain anchors over the full graph, then induced subgraph.
- `Subgraph{query, radius}` → `nodes_within_radius` (out-edge BFS bounded by `radius`) then induced subgraph.
- `TimeWindow` → requires `t0 <= t1` (else `KernelInvalidParams`); `time_window_nodes` (else `ScopeTemporalNotReady`) then subgraph.
- `Tenant` → `tenant_nodes` (else `ScopeTenantNotFound`) then subgraph.
- `Filter` → `filter_nodes` then subgraph.
- `Union` → materialize both sides (depth+1), then `union_graphs` (de-duplicated node/edge merge).
- `Intersect` → materialize both sides, intersect node sets, then subgraph from the full graph over the intersection.

`depth > MAX_SCOPE_DEPTH` → `ScopeDepthExceeded`. `subgraph_from_nodes` keeps node weights and only edges with both endpoints in the set.

## C.9 Scope cache (`scope_cache.rs`)

`DEFAULT_MAX_ENTRIES = 128`. LRU cache of `Kernel`s keyed by content identity.

### ScopeCacheKey fields

| Field | Type | Meaning |
|-------|------|---------|
| `scope_hash` | `[u8;32]` | `scope_hash(scope)` |
| `panel_version` | `u64` | panel version |
| `anchor_identity` | `[u8;32]` | `scope_cache_anchor_identity(anchor_kinds, anchors)` |
| `corpus_identity` | `[u8;32]` | `params.corpus_shard_hash` |

`ScopeCache { entries: BTreeMap<ScopeCacheKey, Kernel>, lru: VecDeque<ScopeCacheKey>, max_entries, hits, misses }`. `get` bumps `hits`/`misses` and touches LRU; `insert` (zero-capacity caches log an eviction and discard; otherwise insert + evict front of LRU until within `max_entries`). `invalidate_panel_version(old)` removes all entries with that panel version. `stats()` → `CacheStats { hits, misses, current_size, max_entries }`. Evictions are logged to stderr as `CALYX_SCOPE_CACHE_EVICT reason=… …`. `scope_cache_anchor_identity` is a framed BLAKE3 over the domain tag, each serialized anchor kind, and each anchor's bytes.

## C.10 Scope report (`scope_report.rs`)

`ScopeKernelReport { scope_name (Debug string), scope_hash, kernel_size, kernel_graph_size, kernel_only_recall, grounded_fraction, approx_factor, tau_star_estimate, tau_star_exact, bridge_count }`. `report_all_scopes(kernels)` computes `bridge_count` per scope: a member is a "bridge" if it appears in `> 1` distinct scope's kernel members (computed over de-duplicated member sets).

## C.11 Temporal kernel (`temporal_kernel.rs`)

Constants: `FREQ_BONUS_MAX = 10_000`, `FREQ_WEIGHT = 0.15`. Error code strings `CALYX_LODESTAR_MISSING_FREQUENCY`, `CALYX_LODESTAR_INVALID_FREQUENCY`, `CALYX_LODESTAR_INVALID_WINDOW`. (Note: re-exported `FREQ_WEIGHT` here is `0.15`, distinct from the kernel-graph score weights.)

`TimeWindow { start_secs, end_secs: i64 }` with `contains(t) = start <= t < end` (half-open). `KernelScope::TimeWindow{window}`. `KernelResult { scope, nodes: Vec<KernelWeight>, active_node_count, source_node_count, warnings }`.

`KernelWeight` fields: `cx_id`, `rank`, `degree_score`, `betweenness_score`, `groundedness_score`, `frequency: u64`, `frequency_bonus: f32`, `total_score`.

Frequency bonus formula (`frequency_kernel_bonus`):

```rust
if frequency == 0 { 0.0 } else {
    let capped = min(frequency, FREQ_BONUS_MAX) as f32;       // cap at 10_000
    let denom  = ln(FREQ_BONUS_MAX as f32 + 1.0);             // ln(10001)
    min(ln(capped + 1.0) / denom, 1.0)                         // log-scaled to [0,1]
}
```

`apply_frequency_bonuses(kernel_graph, vault)`: for each node score, read its frequency from the vault, recompute `frequency_bonus`, and adjust `total_score` by `-FREQ_WEIGHT*previous_bonus + FREQ_WEIGHT*new_bonus` (`0.15`-weighted). Then re-sort and re-select the top-`selected.len()` nodes; returns `Vec<FrequencyRead{cx_id, frequency, missing}>`.

`kernel_for_window(vault, window, k)`: `active_cxids_in_window` (scans the Recurrence CF for occurrences whose `t_k` is in the window), builds a `recurrence_only_graph` (node weight = `frequency_kernel_bonus(...).max(f32::EPSILON)`), then `kernel_for_window_from_graph`: scopes the graph to active nodes, full-scores it with `target_fraction = 1.0` (`full_score_graph`), applies frequency bonuses, and emits the top-`k` `KernelWeight` rows.

`read_frequency` reads the `FREQUENCY_SCALAR`; a missing base row or scalar pushes a `CALYX_LODESTAR_MISSING_FREQUENCY` warning and returns `frequency=0, missing=true`; a non-finite/negative/non-integer value errors `CALYX_LODESTAR_INVALID_FREQUENCY`. `validate_window` requires `start_secs <= end_secs` else `CALYX_LODESTAR_INVALID_WINDOW`.

## C.12 Recall test (`recall_test.rs`)

Defaults: `DEFAULT_HELD_OUT_FRACTION = 0.1`, `DEFAULT_TOP_K = 10`, `DEFAULT_RNG_SEED = 42`, `DEFAULT_MIN_RECALL_RATIO = 0.95`. Constant `CALYX_KERNEL_RECALL_BELOW_GATE`.

Traits: `AnnIndex::search(query_vec, top_k)` (impl'd for `InMemoryAnnIndex` via cosine and for `HnswIndex`); `CorpusReader::{name, len, query, is_empty}`. `RecallTestParams { held_out_fraction, top_k, rng_seed, min_recall_ratio }`. `RecallQuery { cx_id, vector }`. `RecallTestReport = RecallReport`.

`kernel_recall_test_with_clock(kernel_index, full_index, corpus, params, clock)` steps:
1. Validate params (fractions/ratios finite in `[0,1]`, `top_k > 0`). Empty corpus → `RecallEmptyCorpus`.
2. Seed = `params.rng_seed` (or `clock.now()` if `0`). `held_out_ordinals` deterministically samples `ceil(len*held_out_fraction)` queries by sorting on `sample_key = BLAKE3(seed‖ordinal‖cx_id)` and taking the smallest keys (a reproducible pseudo-random hold-out).
3. For each held-out query: `full_index.search` (top_k), `kernel_search` on the kernel index, accumulate `recall_at_k` = `|kernel∩full| / |full|`.
4. `kernel_only = mean recall`, `full = 1.0`, `ratio = kernel_only/full`. Report carries `recall_test_params`, `corpus_name`, `n_queries_tested`, `held_out`, and a `warning` if `ratio < min_recall_ratio`.

`enforce_recall_gate(report, min)` → `RecallBelowGate{ratio, min}` if `ratio < min`. `kernel_recall_gate`/`_with_clock` run the test then enforce. `kernel_recall_test`/`kernel_recall_gate` default to `SystemClock`. `cosine` returns `0.0` when either vector has zero norm.

## C.13 Multi-scope kernel (`multi_scope.rs`)

`UNGROUNDED_EPSILON = 0.01`.

`build_kernel(store, scope, anchor_kind, params, cache)`: materialize the scope graph, derive anchor kinds for the scope (`anchor_kinds_for_scope` — explicit kind, else the union of `Domain` kinds collected through `Union`/`Intersect`), resolve `anchors_for_graph` (domain anchors present in the graph), build the `ScopeCacheKey`, return a cache hit if present. Otherwise run `build_kernel_pipeline`, `mark_ungrounded_scope`, cache, and return. Comment in source: "Union kernel != members_a ∪ members_b" — union scopes materialize a graph and run the same MFVS pipeline.

`mark_ungrounded_scope`: if `groundedness.reached_anchor < UNGROUNDED_EPSILON` (0.01), append a `CALYX_KERNEL_UNGROUNDED: scoped kernel is provisional` warning and append `; CALYX_KERNEL_UNGROUNDED` / `; trust=provisional` to provenance (idempotently).

`bridges(store, scope_a, scope_b, …)`: build both kernels and return their shared members, ordered by full-graph node weight descending (`bridge_members_by_frequency`). `kernel_answer_scoped`: materialize the scope, restrict anchors and the kernel index to scoped nodes (`filter_to_nodes`; empty → `KernelNoAnchoredNode`), then `kernel_answer`. `anchors_for_scope` exposes anchor resolution standalone.

## C.14 Provenance writers (`provenance.rs`)

Ledger-backed writers (actor `Service("calyx-lodestar")`). `KernelBuildReceipt { kernel, ledger_ref }`.

- `build_kernel_pipeline_with_ledger(graph, anchors, params, graph_seq, ledger)` builds the kernel then `append_kernel_build_entry` (`EntryKind::Kernel`, subject `Kernel(kernel_id)`, payload with `kernel_id`, `members_hash`, `graph_seq`, `mfvs_approx_factor`, `mfvs_tau_star_estimate/exact`, `recall_ratio`).
- `append_answer_hop_entry` / `append_answer_complete_entry` emit `EntryKind::Answer` entries keyed by `Query(query_cx)` with per-hop and complete-path JSON payloads.
- All payloads pass `RedactionPolicy::check_payload`. `kernel_members_hash` = BLAKE3 over `"calyx-lodestar-kernel-members-v1"` + each member's bytes. `AnswerHopEvidence` / `AnswerCompleteHopEvidence` are the payload structs.

## C.15 Incremental kernel maintenance (`incremental.rs`)

`NodeAddEdge { Out{dst, weight} | In{src, weight} }`. `IncrementalResult` (`#[must_use]`): `Dirty{affected_sccs}`, `FullRebuildRequired{reason}`, `KernelMemberRemoved{id}`, `Unchanged`.

`IncrementalKernelEval { kernel, graph, anchors, dirty_sccs: BTreeSet<usize>, params, stale }`. Operations:
- `apply_edge_weight_change(src, dst, w)`: rebuild graph with the one edge's weight replaced; if unchanged → `Unchanged`; else mark the SCCs of `src`/`dst` dirty → `Dirty`.
- `apply_node_add(id, freq, edges)`: rebuild with the new node/edges; if the new node lands in an SCC of size `> 1`, mark `stale` and return `FullRebuildRequired{"node addition merged an SCC"}`; else mark that singleton component dirty → `Dirty`.
- `apply_node_remove(id)`: rebuild without the node; mark `stale`. If the node was a kernel member → `KernelMemberRemoved{id}`; else `FullRebuildRequired{"node removal can split or reindex SCCs"}`.
- `rebuild_dirty()`: if dirty or stale, re-run `build_kernel_pipeline` over the current graph/anchors/params and clear the flags.

## C.16 Hierarchical kernel (`hierarchical.rs`)

`RegionId(String)`, `RegionDescriptor { id, centroid_cx, members: BTreeSet<CxId> }`, `RegionStore: AssocStore + regions_for_scope`. `HierarchicalKernelParams { max_regions: 64, drill_radius: 2, min_region_size: 1, anchor_kind: None, kernel_params }`. `HierarchicalKernel { region_kernel, region_drilldowns: Vec<(RegionId, Kernel)> }`.

`build_hierarchical_kernel` steps:
1. `bounded_regions`: drop regions below `min_region_size`, sort by id, truncate to `max_regions`. No regions → fall back to a single flat `build_kernel`.
2. `build_region_graph`: a quotient graph with one synthetic node per region (`region_node_id = CxId::from_bytes(content_address("calyx-lodestar-region-v1" ‖ id))`, weight = region member count). Inter-region edge weight accumulates `edge.weight / (|left|*|right|)` and is clamped to `<= 1.0`.
3. Resolve region-level anchors (`region_anchor_nodes`), run `build_kernel_pipeline` over the region graph → `region_kernel`.
4. For each selected region node, drill down via `build_kernel(Scope::Subgraph{query: centroid_cx, radius: drill_radius}, …)`; collect `(RegionId, Kernel)` drilldowns.

`HierarchicalKernel::all_members` is the de-duplicated union of all drilldown members.

## C.17 Label propagation (`label_propagation.rs`)

Harmonic-extension label propagation over the symmetrized graph. `DEFAULT_PROPAGATION_DECAY_LAMBDA = 0.5`. `PropagationError` codes: `CALYX_PROP_GRAPH_EMPTY`, `CALYX_PROP_NO_KERNEL_NODES`, `CALYX_PROP_NOT_CONVERGED`, `CALYX_PROP_INVALID_INPUT`.

`PropagatedLabel { node_id, label: f32, confidence: f32, hop_distance: u32, provisional: bool }`.

`propagate_labels_with_decay(graph, kernel_labels, max_iter, tol, decay_lambda)` steps:
1. Validate (non-empty graph, ≥1 kernel label, `max_iter>0`, positive `tol`, non-negative `decay_lambda`). Kernel labels must be finite in `[0,1]` and reference present, non-duplicate nodes.
2. `sym_neighbors`: undirected neighbor lists with max-weight per pair (self-loops dropped). `hop_distances`: multi-source BFS from kernel nodes (`u32::MAX` = unreachable). `initial_values`: kernel nodes pinned to their label, all others seeded to the mean kernel label.
3. Jacobi/harmonic iteration: each non-kernel node becomes the weight-weighted average of its neighbors' current values; kernel nodes are clamped (Dirichlet boundary); isolated nodes → `0.0`. Convergence when `max_delta <= tol`; not converging by `max_iter` → `NotConverged`.
4. `label_rows`: `label = value.clamp(0,1)`; `confidence` = the kernel label for kernel nodes, `0.0` for unreachable, else `value.clamp(0,1) * exp(-decay_lambda * hop)`; `provisional = (not a kernel node)`.

`propagate_labels` uses the default lambda `0.5`. `propagate_labels_with_ledger` appends a `EntryKind::Kernel` ledger entry (`propagation_payload` with `propagation_id`/`kernel_hash`, `graph_version`, `node_count`, `n_propagated`, `max_hop_distance`). `kernel_labels_hash` = BLAKE3 over sorted `(node_id, label.to_bits())` pairs.

## C.18 Loom association bridge (`loom_assoc.rs`)

Builds an `AssocGraph` from Loom cross-terms (see [09_loom_assay_dda.md](09_loom_assay_dda.md)). Types: `LoomSlotNode { xterm_cx, slot, node }`, `LoomDirectionalConfidence { xterm_cx, src_slot, dst_slot, confidence }`, `LoomAssocGraphInput { agreements: Vec<AgreementEdge>, provenance: Vec<LoomAssocEdgeProvenance> }`, `LoomAssocEdgeProvenance` (full per-edge trace: slots, cx ids, `raw_agreement`, `agreement`, `directional_confidence`, `edge_weight`).

`loom_assoc_graph_input(store, slot_nodes, directional_confidences)`: reads `Agreement` cross-terms from the `LoomStore` (`agreement_rows`, scalar-only, clamped to `[0,1]` via `agreement_weight`), maps slots to `CxId`s (`slot_node_map`; missing → `KernelLoomSlotMappingMissing`), and for each directional confidence forms an `AgreementEdge` with `edge_weight = agreement * confidence`. Missing agreement → `KernelLoomAgreementMissing`; an agreement with no matching directional confidence → `KernelLoomDirectionalConfidenceMissing`; duplicate/invalid confidence → `KernelLoomAgreementInvalid`. `build_assoc_graph_from_loom` feeds the result into `calyx_mincut::build_assoc_graph` and returns the graph plus provenance.

## C.19 Summarization (`summarize.rs`)

`MIN_GROUNDED_FRACTION = 0.5`. Marker `SUMMARIZE_INVOKED`. Codes: `CALYX_SCOPE_INVALID_TIME_WINDOW`, `CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING`, `CALYX_TIMETRAVEL_BEFORE_HORIZON`. Doctrine in the module header: the kernel is the universal, **structural** summarization primitive — the summary *is* the kernel node ids, never generated text — and every call is ledger-provenanced.

`SummarizeParams { max_kernel_size: Option<usize>, require_grounded: bool, cache_ttl_secs: Option<u64> (default Some(3600)), anchor_kind }`. `SummarizeResult { scope_hash, kernel_ids, kernel_size, kernel_only_recall, grounded_fraction, approx_factor, ledger_ref }`. `SummarizeRecall { embeddings, full_index, corpus, params }`. `SummarizeCtx { cache, clock, ledger }`.

`summarize_with_ledger` steps:
1. `validate_scope` rejects any inverted `TimeWindow` anywhere in the tree (`CALYX_SCOPE_INVALID_TIME_WINDOW`).
2. Build `KernelParams { built_at_millis = clock.now() }`. If `max_kernel_size` is set, `target_fraction_for_cap` derives `target_fraction = clamp(max / node_count, MIN_POSITIVE, 1.0)` so the kernel is honestly capped (metrics computed on the capped kernel).
3. `cache_ttl_secs == Some(0)` drives the build with a throwaway 1-entry cache (forces recompute + re-provenance); else the caller's cache.
4. `build_kernel`. If `require_grounded` and `grounded_fraction < 0.5` → `CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING`.
5. `apply_measured_recall` (optional): build a kernel index from supplied embeddings, run `kernel_recall_test_with_clock`, and overwrite `kernel.recall` (preserving the DFVS `approx_factor`/`tau_star`).
6. Append the `SUMMARIZE_INVOKED` ledger entry (scope hash + metrics) and return the result.

`summarize_as_of(store, scope, t, retention_horizon, …)`: before `retention_horizon` → `CALYX_TIMETRAVEL_BEFORE_HORIZON`; otherwise expresses "as of t" as `Intersect(scope, TimeWindow{0, t})`. `kernel_only_recall` is `0.0` unless a `SummarizeRecall` is supplied (per the source comment, the `AssocStore` graph carries edge weights only).

## C.20 Aster bridge (`aster_bridge.rs`)

`DEFAULT_ASTER_ASSOC_COLLECTION = "default"`, `ASTER_ASSOC_METADATA_KEY = "lodestar_assoc_v1"`. Wires the lodestar `AssocStore` abstraction onto an Aster vault `PlainGraph` snapshot.

`AsterAssocMetadata { retention_horizon: Option<Ts> }`. `AsterAssocNodeProps { embedding: Option<Vec<f32>>, ts: Option<Ts>, anchors: Vec<AnchorKind>, tenant: Option<TenantId>, named_filters: Vec<String>, metadata: BTreeMap<String,String> }`. `AsterAssocSnapshot<'a, C>` pins a vault, collection, snapshot `Seq`, metadata, and an optional time-travel lease.

`AsterAssocSnapshot::{latest, as_of}` (as_of fails closed before the retention horizon). It implements `AssocStore` by reading node props from the Aster graph: `full_graph` → `PlainGraph::assoc_graph`; `domain_anchors` filters nodes whose props contain the anchor kind; `time_window_nodes` filters by `props.ts` in `[t0,t1]` (returns `None` if no node carried a ts → maps to `ScopeTemporalNotReady`); `tenant_nodes`/`filter_nodes` by props. `recall_inputs` materializes embeddings + an `InMemoryAnnIndex`/`InMemoryCorpus` from per-node embeddings (missing embedding → `CALYX_SUMMARIZE_RECALL_MISSING_EMBEDDING`).

`summarize_vault_latest`/`summarize_vault_as_of(vault, request, …)` snapshot the vault and call `summarize_with_ledger`, appending the `SUMMARIZE_INVOKED` entry directly to the vault ledger (`append_invoked_to_vault`). `write_assoc_metadata` / `encode_assoc_node_props` persist the metadata and node-props codecs.

---

## Appendix — Algorithm catalog (name → complexity → inputs/outputs)

| Algorithm | Crate · file · fn | Complexity | Inputs → Output |
|-----------|-------------------|------------|------------------|
| Bidirectional BFS shortest path | paths · traversal.rs · `reach` | ~O(V+E) | graph, src, dst, max_hops → `Option<path>` |
| Weight-product scored reach | paths · traversal.rs · `reach_scored` | ~O(V+E) bounded by max_hops | graph, src, max_hops → `[(CxId, score)]` |
| Hop attenuation `base·0.9^h` | paths · attenuation.rs · `attenuate` | O(1) | score, hops → score |
| Build assoc graph | mincut · graph_builder.rs · `build_assoc_graph` | O(N log N + E) | agreements, freqs, citations → `AssocGraph` |
| Tarjan SCC | mincut · scc.rs · `tarjan_scc` | O(V+E) | graph → components + `component_of` |
| SCC condensation | mincut · scc.rs · `condensate` | O(V+E) | graph, scc → `CondensedGraph` |
| Weighted Brandes betweenness | mincut · betweenness.rs · `betweenness` | ~O(V·(V²+E)) dense | graph → `{CxId: f64}` |
| MFVS LP relaxation skeleton (no constraints) | mincut · lp_scaffold.rs · `mfvs_lp_problem` | O(V) | graph → `LpProblem` |
| Eigenvector centrality (power iter, shifted) | mincut · spectral.rs · `eigenvector_centrality` | O(iter·V²) | graph, max_iter, tol → ranked `[(CxId,f32)]` |
| Laplacian eigenmaps (Lanczos + cyclic Jacobi) | mincut · spectral_linalg.rs · `lanczos_eigen` | O(iter·V²)/Jacobi | graph, k → `[EigenPair]` |
| Kernel-graph scoring + select | lodestar · kernel_graph.rs · `select_kernel_graph` | O(V·(V+E)) (per-node groundedness BFS) | graph, scc, bet, anchors, params → `KernelGraph` |
| Groundedness distance (anchor BFS) | lodestar · kernel_graph.rs · `groundedness_distance` | O(V+E) | graph, node, anchors, max_hops → `Option<hops>` |
| LP rounding from solution | lodestar · kernel_graph.rs · `lp_round_kernel_graph_from_solution` | O(V+E) | kernel_graph, params, `LpSolution` → `KernelGraph` |
| DFVS approximation (dispatch) | lodestar · dfvs.rs · `dfvs_approx` | varies (see below) | `KernelGraph` → `DfvsResult` |
| Exact min FVS (subset enumeration) | lodestar · dfvs.rs · `exact_min_fvs` | exponential, gated at ≤20 nodes | graph → `Vec<CxId>` |
| Greedy max-degree FVS + local-search shrink | lodestar · dfvs.rs · `greedy_fvs`/`local_search_shrink` | O(V·(V+E)) | graph → `Vec<CxId>` |
| Kernel pipeline | lodestar · kernel.rs · `build_kernel_pipeline` | dominated by betweenness + DFVS | graph, anchors, params → `Kernel` |
| HNSW kernel build/search | lodestar · kernel_index.rs · `build_kernel_index`/`kernel_search` | HNSW (delegated to sextant) | kernel + embeddings → `KernelIndex`; query → hits |
| Kernel answer path | lodestar · kernel_answer.rs · `kernel_answer` | search + per-anchor `reach` | index, graph, query → `AnswerPath` |
| Scope materialization (recursive, depth ≤5) | lodestar · scope.rs · `materialize_scope` | O(V+E) per node-set op | scope, store → `AssocGraph` |
| Deterministic recall hold-out + recall@k | lodestar · recall_test.rs · `kernel_recall_test_with_clock` | O(corpus · top_k) | indices, corpus, params → `RecallReport` |
| Harmonic label propagation | lodestar · label_propagation.rs · `propagate_labels_with_decay` | O(iter·E) | graph, kernel_labels → `[PropagatedLabel]` |
| Hierarchical (quotient) kernel | lodestar · hierarchical.rs · `build_hierarchical_kernel` | per-region pipelines | store, scope, params → `HierarchicalKernel` |

## Appendix — Key constants

| Constant | Value | Location |
|----------|-------|----------|
| `HOP_DECAY` | `0.9` | paths · attenuation.rs |
| `DIST_EPSILON` (betweenness) | `1e-12` | mincut · betweenness.rs |
| `EIGEN_EPS` | `1e-6` | mincut · spectral.rs, spectral_linalg.rs |
| `DEFAULT_EIGEN_MAX_ITER` / `JACOBI_MAX_ITER` | `256` | mincut · spectral·spectral_linalg |
| `JACOBI_TOL` | `1e-6` | mincut · spectral_linalg.rs |
| `target_fraction` (kernel graph) | `0.10` | lodestar · kernel_graph.rs |
| `max_groundedness_distance` | `3` | lodestar · kernel_graph.rs |
| degree/betweenness/groundedness weights | `0.40 / 0.40 / 0.20` (sum 1.0) | lodestar · kernel_graph.rs |
| `LpRoundParams.threshold` | `0.5` | lodestar · kernel_graph.rs |
| `EXACT_SEARCH_MAX_NODES` | `20` | lodestar · dfvs.rs |
| DFVS genus cap | `100` | lodestar · dfvs.rs |
| `FREQ_BONUS_MAX` / `FREQ_WEIGHT` | `10_000` / `0.15` | lodestar · temporal_kernel.rs |
| `MAX_SCOPE_DEPTH` | `5` | lodestar · scope.rs |
| `DEFAULT_MAX_ENTRIES` (scope cache) | `128` | lodestar · scope_cache.rs |
| `UNGROUNDED_EPSILON` | `0.01` | lodestar · multi_scope.rs |
| `MIN_GROUNDED_FRACTION` (summarize) | `0.5` | lodestar · summarize.rs |
| `DEFAULT_PROPAGATION_DECAY_LAMBDA` | `0.5` | lodestar · label_propagation.rs |
| recall defaults: held-out / top_k / seed / min_ratio | `0.1 / 10 / 42 / 0.95` | lodestar · recall_test.rs |
| `KERNEL_SLOT` / `HNSW_SEED` | `u16::MAX` / `0x4c4f444553544152` | lodestar · kernel_index.rs |
| `FORMAT_VERSION` / `KERNEL_ARTIFACT_FORMAT_VERSION` | `1` / `1` | lodestar · kernel_index·kernel_health |
| `default cache_ttl_secs` (summarize) | `3600` | lodestar · summarize.rs |
