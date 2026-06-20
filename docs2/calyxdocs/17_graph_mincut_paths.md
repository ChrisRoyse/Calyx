# Graph Primitives — calyx-mincut + calyx-paths

**Source files covered:**

- `crates/calyx-mincut/src/lib.rs`
- `crates/calyx-mincut/src/error.rs`
- `crates/calyx-mincut/src/graph_builder.rs`
- `crates/calyx-mincut/src/scc.rs`
- `crates/calyx-mincut/src/betweenness.rs`
- `crates/calyx-mincut/src/lp_scaffold.rs`
- `crates/calyx-mincut/src/spectral.rs`
- `crates/calyx-mincut/src/spectral_linalg.rs`
- `crates/calyx-mincut/tests/ph31_mincut_tests.rs`
- `crates/calyx-mincut/tests/ph52_spectral_tests.rs`
- `crates/calyx-paths/src/lib.rs`
- `crates/calyx-paths/src/error.rs`
- `crates/calyx-paths/src/graph.rs`
- `crates/calyx-paths/src/traversal.rs`
- `crates/calyx-paths/src/attenuation.rs`
- `crates/calyx-paths/tests/ph31_paths_tests.rs`

These two crates supply the directed-graph data structure and graph algorithms consumed by the grounding kernel — see [12_lodestar_kernel.md](12_lodestar_kernel.md). `calyx-paths` owns the graph type (`AssocGraph`) and traversal; `calyx-mincut` depends on `calyx-paths` and adds SCC, centrality, spectral, and LP-scaffold algorithms. Both depend on `calyx-core` for `CxId` (a 16-byte content id with `from_bytes([u8;16])` and `as_bytes() -> &[u8;16]`, `crates/calyx-core/src/ids.rs`). Crate-level attribute `#![deny(warnings)]` is set on both (`lib.rs`).

Both crates depend only on `calyx-core`, `serde`, `thiserror` (dev: `proptest`, `serde_json`). No external linear-algebra or graph library is used — every algorithm is hand-rolled (`Cargo.toml` of each).

---

## Part A — calyx-mincut

Module-level doc (`lib.rs`): *"Directed graph primitives for Calyx grounding kernels."* Public re-exports:

- `betweenness::{betweenness, betweenness_top_k}`
- `error::{MincutError, Result}`
- `graph_builder::{AgreementEdge, CitationEdge, FrequencyEntry, build_assoc_graph}`
- `lp_scaffold::{ConstraintSense, LpConstraint, LpProblem, LpSolution, LpVariable, OptSense, SolveStatus, mfvs_lp_problem}`
- `scc::{CondensedEdge, CondensedGraph, SccResult, condensate, tarjan_scc}`
- `spectral::{EigenPair, SparseGraph, SpectralCache, SpectralCacheEntry, SpectralCacheKey, SpectralError, SpectralResult, eigenvector_centrality, gft_project, gft_reconstruct, laplacian_eigenmaps, laplacian_eigenmaps_with_max_iter, spectral_gap}`

Note: the crate is named "mincut" but **no min-cut / max-flow / feedback-vertex-set solver is implemented here.** `lp_scaffold` only *formulates* the minimum-feedback-vertex-set (MFVS) LP and provides serializable structs; it does not solve it. There is no Ford-Fulkerson / push-relabel / Stoer-Wagner code. See §A.4 and Gaps.

### 1. Error type (`error.rs`)

`pub type Result<T> = std::result::Result<T, MincutError>`.

`MincutError` (`#[derive(Clone, Debug, PartialEq, Error)]`):

| Variant | Fields | `code()` string |
|---|---|---|
| `SccGraphMismatch` | `detail: String` | `CALYX_SCC_GRAPH_MISMATCH` |
| `BetweennessEmptyGraph` | — | `CALYX_BETWEENNESS_EMPTY_GRAPH` |
| `LpInvalid` | `detail: String` | `CALYX_LP_INVALID` |
| `NodeNotFound` | `id: CxId` | `CALYX_MINCUT_NODE_NOT_FOUND` |

`MincutError::code(&self) -> &'static str` (const) maps each variant to its code string (the same code is the prefix of the `Display` message). `MincutError::lp_invalid(detail: impl Into<String>) -> Self` constructs `LpInvalid`. (`NodeNotFound` is defined but not produced by any function in this crate as read.)

### 2. Graph builder (`graph_builder.rs`)

Converts Loom-style cross-term inputs into a `calyx_paths::AssocGraph`. Three public input structs (all `Clone, Copy, Debug, PartialEq, Serialize, Deserialize`; `CitationEdge` additionally `Eq`):

| Struct | Fields |
|---|---|
| `AgreementEdge` | `src: CxId`, `dst: CxId`, `agreement: f32`, `directional_confidence: f32` |
| `FrequencyEntry` | `cx_id: CxId`, `frequency: f32` |
| `CitationEdge` | `src: CxId`, `dst: CxId` |

`pub fn build_assoc_graph(agreements: &[AgreementEdge], frequencies: &[FrequencyEntry], citations: &[CitationEdge]) -> calyx_paths::Result<AssocGraph>`

Steps:
1. For each `AgreementEdge`, validate `agreement` and `directional_confidence` are finite and in `[0,1]` (`validate_unit`; else `PathsError::GraphInvalidWeight`). Seed both `src` and `dst` into a `BTreeMap<CxId,f32>` node-weight map with default weight `1.0`.
2. For each `FrequencyEntry`: require `frequency` finite and `>= 1.0` (else `GraphInvalidWeight{field:"frequency"}`); set the node weight to `max(existing, frequency)`, inserting if absent.
3. For each `CitationEdge`: require both `src` and `dst` already present as nodes (else `GraphUnknownNode`).
4. Build via `AssocGraph::builder()`: add every node with its weight (sorted iteration of the BTreeMap), then add an edge per agreement with weight `agreement * directional_confidence`, then an edge per citation with weight `1.0`.

Complexity: O(A + F + C + N log N) for A agreements, F frequencies, C citations, N nodes (BTreeMap ordering + builder sort). Returns the built `AssocGraph`. Errors are `calyx_paths::PathsError` (this function returns the *paths* `Result`, not the mincut one).

`validate_unit(value, field) -> Result<()>` (private): finite and in `[0,1]`.

### 3. Strongly connected components (`scc.rs`)

**Algorithm: Tarjan's SCC.**

Public types (all `Clone, Debug, PartialEq, Serialize, Deserialize`):

| Type | Fields |
|---|---|
| `SccResult` | `components: Vec<Vec<CxId>>`, `component_of: BTreeMap<CxId, usize>` |
| `CondensedEdge` | `src_component: usize`, `dst_component: usize`, `weight: f32` |
| `CondensedGraph` | `component_nodes: Vec<Vec<CxId>>`, `edges: Vec<CondensedEdge>` |

`pub fn tarjan_scc(graph: &AssocGraph) -> SccResult`

Steps (`strong_connect`, recursive DFS):
1. Maintain `TarjanState` (private): `next_index`, `indices: Vec<Option<usize>>`, `lowlinks: Vec<usize>`, `stack: Vec<usize>`, `on_stack: Vec<bool>`, `components: Vec<Vec<CxId>>`. All indexed by node *index* (0..node_count).
2. For each unvisited node, call `strong_connect`: assign DFS index and lowlink = `next_index++`, push to stack, mark on-stack.
3. For each out-edge (`graph.out_edges_by_index`): if target unvisited, recurse and `lowlink = min(lowlink, child.lowlink)`; else if target on-stack, `lowlink = min(lowlink, target.index)`.
4. When `lowlink[node] == index[node]`, pop the stack down to `node`, forming a component; map each popped index to its `CxId` via `graph.node_id`. **The component vector is sorted (`component.sort()`).**
5. After all roots processed, build `component_of` by flat-mapping each component's nodes to its component index.

Complexity: O(V + E). Component order is DFS-finish order (Tarjan emits in reverse-topological order of the condensation). Recursion depth is bounded by V — deep graphs risk stack overflow (no iterative variant).

`pub fn condensate(graph: &AssocGraph, scc: &SccResult) -> Result<CondensedGraph>`
1. `validate_scc` (private): error `SccGraphMismatch` if `component_of.len() != node_count`, or if the SCC node set differs from the graph node set (compared via `BTreeSet`).
2. For every graph edge, look up `src`/`dst` component; skip intra-component edges; for inter-component pairs accumulate into `BTreeMap<(usize,usize), f32>` keeping `max(weight)` (parallel condensed edges merged by max).
3. Emit one `CondensedEdge` per `(src_component, dst_component)`. `component_nodes` = `scc.components.clone()`.

Complexity: O(E log E + V).

`CondensedGraph::is_dag(&self) -> bool` — 3-color DFS (`has_cycle`, colors 0=unvisited,1=on-path,2=done) over component indices; returns true iff no back edge. Complexity O(C·E_c) as written (`has_cycle` re-filters all condensed edges per node). Used by lodestar to confirm acyclicity of the condensation (test `scc_planted_cycle_and_condensation_match_known_partition` asserts `is_dag()` after collapsing a 3-cycle to one component).

### 4. LP scaffold — MFVS formulation (`lp_scaffold.rs`)

This module is **formulation + serialization only; it does not solve.** It builds the LP relaxation of minimum feedback vertex set.

Enums (`Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize`, `#[serde(rename_all="snake_case")]`):

| Enum | Variants |
|---|---|
| `ConstraintSense` | `Leq`, `Geq`, `Eq` |
| `OptSense` | `Minimize`, `Maximize` |
| `SolveStatus` | `Optimal`, `Infeasible`, `Unbounded`, `NotSolved` |

Structs (`Clone, Debug, PartialEq, Serialize, Deserialize`):

| Struct | Fields |
|---|---|
| `LpVariable` | `id: usize`, `name: String`, `lb: f64`, `ub: f64` |
| `LpConstraint` | `coeffs: Vec<(usize, f64)>`, `sense: ConstraintSense`, `rhs: f64` |
| `LpProblem` | `vars: Vec<LpVariable>`, `constraints: Vec<LpConstraint>`, `objective: Vec<(usize, f64)>`, `sense: OptSense` |
| `LpSolution` | `values: Vec<f64>`, `objective_value: f64`, `status: SolveStatus` |

Methods / functions:
- `LpVariable::new(id, name: impl Into<String>, lb, ub) -> Result<Self>` — error `LpInvalid` if `lb`/`ub` non-finite or `lb > ub`.
- `LpProblem::validate(&self) -> Result<()>` — checks each var's `id` equals its dense index, bounds finite & `lb<=ub`; every objective and constraint variable reference is `< vars.len()` (`validate_var_ref`); every coefficient and rhs is finite (`validate_finite`). Errors `LpInvalid`.
- `pub fn mfvs_lp_problem(graph: &AssocGraph) -> Result<LpProblem>` — one variable `x_{id}` per node with bounds `[0,1]`; objective coefficient `1.0` per variable; `sense = Minimize`; **`constraints` is empty** (cycle-elimination constraints are NOT generated). Calls `validate()` before returning.

So `mfvs_lp_problem` produces the *objective* of "minimize sum of node indicators" with box constraints but no cycle constraints, and there is no solver. The lodestar doc confirms LP rounding is not wired to a solver and the kernel falls back to a heuristic DFVS (see [12_lodestar_kernel.md](12_lodestar_kernel.md) §2.4, Gaps). Test `lp_scaffold_roundtrips_and_triangle_problem_has_unit_bounds` checks JSON round-trip and that a 3-node triangle yields 3 unit-bound vars and objective `[(0,1),(1,1),(2,1)]`.

### 5. Betweenness centrality (`betweenness.rs`)

**Algorithm: Brandes' betweenness on a weighted directed graph**, using reciprocal edge weight as distance.

`pub fn betweenness(graph: &AssocGraph) -> Result<BTreeMap<CxId, f64>>`
- Constant `DIST_EPSILON: f64 = 1.0e-12`.
- Empty graph → `MincutError::BetweennessEmptyGraph`.

Steps:
1. For each source node index 0..n: run `shortest_paths_from` then `accumulate_dependencies`.
2. `shortest_paths_from` is a **Dijkstra** SSSP (not BFS): `dist[source]=0`, `sigma[source]=1`; repeatedly pick the unvisited finite-distance node of minimum distance (`min_unvisited`, ties broken by lower index). Edge with `weight <= 0` is skipped. Candidate distance = `dist[node] + 1.0/weight` (reciprocal weight ⇒ higher weight = shorter). Standard shortest-path-counting: on strictly-less distance, replace dist, set `sigma = sigma[node]`, reset predecessors to `[node]`; on `approx_eq` (within `DIST_EPSILON`) accumulate `sigma += sigma[node]` and append predecessor. Records visitation `stack`, `sigma`, `predecessors`.
3. `accumulate_dependencies` (Brandes back-propagation): walk `stack` in reverse, for each predecessor `delta[pred] += (sigma[pred]/sigma[node]) * (1 + delta[node])`; add `delta[node]` to `scores[node]` for `node != source`.
4. Normalize all scores by `(n-1)(n-2)` when `n>2`, else by `1.0`. Return `BTreeMap<CxId,f64>`.

Complexity: O(V · (V² + E)) because `min_unvisited` is a linear scan rather than a heap (no priority queue). Memory O(V + E) per source.

`pub fn betweenness_top_k(graph: &AssocGraph, k: usize) -> Result<Vec<(CxId, f64)>>` — sorts descending by score, ties broken by ascending `CxId.as_bytes()`; truncates to `min(k, len)`. Tests: a 5-node path gives mid-node 1/3, neighbors 0.25, ends 0.0; a bidirectional star hub scores `>0.99`, leaves 0.0.

### 6. Spectral algorithms (`spectral.rs`, `spectral_linalg.rs`)

Type aliases: `pub type NodeId = CxId`, `pub type SparseGraph = AssocGraph`, `pub type SpectralResult<T> = std::result::Result<T, SpectralError>`. Constants: `EIGEN_EPS: f32 = 1e-6`, `DEFAULT_EIGEN_MAX_ITER: usize = 256` (and in `spectral_linalg.rs`: `JACOBI_TOL = 1e-6`, `JACOBI_MAX_ITER = 256`).

Public types:

| Type | Fields / kind |
|---|---|
| `EigenPair` (`Clone,Debug,PartialEq,Serialize,Deserialize`) | `eigenvalue: f32`, `eigenvector: Vec<f32>` |
| `SpectralCacheKey` (`+Eq,PartialOrd,Ord`) | `scope: String`, `panel_version: u64` |
| `SpectralCacheEntry` | `centrality: Vec<(NodeId,f32)>`, `eigenpairs: Vec<EigenPair>`, `refreshed_at_seq: u64` |
| `SpectralCache` (`+Default`) | private `entries: BTreeMap<SpectralCacheKey, SpectralCacheEntry>` |

`SpectralCache` methods: `insert(key, entry)`, `get(&key) -> Option<&Entry>`, `invalidate(&key) -> Option<Entry>` (remove), `invalidate_scope(scope: &str)` (retain entries with a different scope), `len()`, `is_empty()`.

`SpectralError` (`Clone,Debug,PartialEq,Error`):

| Variant | Fields | code |
|---|---|---|
| `NotConverged` | `iterations: usize` | `CALYX_SPECTRAL_NOT_CONVERGED` |
| `GraphTooSmall` | `n: usize`, `required: usize` | `CALYX_SPECTRAL_GRAPH_TOO_SMALL` |
| `SingularMatrix` | — | `CALYX_SPECTRAL_SINGULAR_MATRIX` |

All spectral routines first **symmetrize** the directed graph: `sym_adjacency` builds a dense `n×n` matrix where `A[i][j]=A[j][i]=max(weight over both directions)` (`spectral.rs`). So spectral analysis treats the association graph as undirected.

#### 6.1 `eigenvector_centrality(graph, max_iter, tol) -> SpectralResult<Vec<(NodeId, f32)>>`
**Algorithm: shifted power iteration on the (symmetrized) adjacency.**
1. Require ≥2 nodes (`ensure_min_nodes`, else `GraphTooSmall`); `max_iter==0` → `NotConverged{0}`.
2. Build symmetric adjacency; init vector `1/√n` uniformly.
3. Each step: `next = (A + I)·current` (`shifted_mat_vec` adds the diagonal-shift `+current[i]` to guarantee a dominant positive eigenvalue), then `normalize` (L2; non-finite or `<=EIGEN_EPS` norm → `SingularMatrix`). If `l2_distance(next, current) < tol`, converged.
4. On convergence, `ranked_scores`: take `|value|/max(|value|)` (0 if `max<=EIGEN_EPS`), sorted descending, ties by `CxId.as_bytes()`. Else after `max_iter` → `NotConverged`.

Complexity: O(max_iter · n²) (dense mat-vec). Tests: cycle graph → uniform scores; star → hub ranks first with score ≥ 2× next; scores normalized into `[0,1]`.

#### 6.2 `laplacian_eigenmaps(graph, k)` / `laplacian_eigenmaps_with_max_iter(graph, k, max_iter) -> SpectralResult<Vec<EigenPair>>`
**Algorithm: Lanczos tridiagonalization + Jacobi eigensolver on the graph Laplacian** `L = D − A` (`laplacian_matrix`: diagonal = row degree sum of symmetric adjacency, off-diagonal = `−weight`).
1. `laplacian_eigenmaps` delegates with `DEFAULT_EIGEN_MAX_ITER=256`. Require ≥2 nodes; `k==0` → empty; `max_iter==0` → `NotConverged{0}`.
2. `lanczos_eigen` (`spectral_linalg.rs`): build an orthonormal Krylov `basis` of dimension n via `lanczos_basis` (seed with unit vectors, three-term recurrence `r = A·v − β·v_prev − α·v`, full reorthogonalization `orthogonalize_against` all prior basis vectors, `β = ‖r‖`; reseed with the next unit vector when β collapses, to handle disconnected/reducible graphs). If the basis cannot reach full dimension within `max_iter` → `NotConverged{max_iter}`.
3. `project_to_basis`: form the `m×m` matrix `Bᵀ A B`.
4. `jacobi_eigen`: cyclic-Jacobi rotations zeroing the largest off-diagonal (`max_offdiag`) each step; stop when largest off-diagonal `< JACOBI_TOL` (`1e-6`), re-orthonormalize the vector columns every 10th iteration; `JACOBI_MAX_ITER=256` exceeded → `NotConverged`. Returns eigenvalues (diagonal) and Ritz vectors.
5. `expand_ritz_vectors` maps Ritz vectors back to the n-dim node space.
6. Each pair: eigenvalue passed through `clean_zero` (snap `|v|<EIGEN_EPS` to 0); eigenvector sign-oriented (`orient_vector`: flip so the first non-trivial entry is positive — deterministic sign). Sort by ascending eigenvalue; truncate to `min(k, len)`.

Complexity: dominated by O(n³) (dense Lanczos mat-vecs n times + Jacobi sweeps). Eigenvector index 1 is the Fiedler vector. Test `planted_two_community_graph_bisects_by_second_eigenvector` confirms the Fiedler vector separates the two planted communities by sign.

#### 6.3 Graph Fourier transform
- `pub fn gft_project(signal: &[f32], eigenvectors: &[EigenPair]) -> Vec<f32>` — coefficient `c_i = dot(signal, v_i)` per eigenpair. **Panics** (`assert_eq!`) on signal/eigenvector dimension mismatch.
- `pub fn gft_reconstruct(coefficients: &[f32], eigenvectors: &[EigenPair]) -> Vec<f32>` — `signal = Σ c_i · v_i`. **Panics** on coefficient/eigenvector count or dimension mismatch; returns empty vec if no eigenvectors. Round-trip is exact when the basis is full (test asserts ≤1e-3 error; low-pass keeping 2 coefficients recovers the smooth component).

#### 6.4 `spectral_gap(eigenmaps: &[EigenPair]) -> f32`
Returns `max(0, λ₁ − λ₀)` (the algebraic-connectivity gap); `0.0` if fewer than 2 eigenpairs. A disconnected graph (two components) has gap `0.0` (test `spectral_edges_fail_closed_and_star_hub_ranks_highest`).

Private linalg helpers (`spectral_linalg.rs`, `pub(crate)`): `lanczos_eigen`, `column(matrix, index)`. Internal: `dense_mat_vec`, `dot`, `axpy`, `scale`, `normalize`, `vector_norm`, `orthogonalize_against`, `identity`, `diagonal`, `rotate`, `max_offdiag`, `jacobi_eigen`, `orthonormalize_columns`, `lanczos_basis`, `next_lanczos_seed`, `project_to_basis`, `expand_ritz_vectors`.

Comment in `spectral.rs` (lines 298–299): spectral centrality is "structure-only"; the MFVS kernel is outcome-anchored — centrality proposes candidates, grounding confirms them.

---

## Part B — calyx-paths

Module-level doc (`lib.rs`): *"Path and graph traversal over Calyx association networks."* Public re-exports:

- `attenuation::{attenuate, deattenuate}`
- `error::{PathsError, Result}`
- `graph::{AssocGraph, AssocGraphBuilder, Edge, NodeEntry}`
- `traversal::{BidirectionalPath, bidirectional, reach, reach_scored}`

### 1. Error type (`error.rs`)

`pub type Result<T> = std::result::Result<T, PathsError>`.

`PathsError` (`Clone, Debug, PartialEq, Error`):

| Variant | Fields | `code()` |
|---|---|---|
| `GraphDuplicateNode` | `id: CxId` | `CALYX_GRAPH_DUPLICATE_NODE` |
| `GraphUnknownNode` | `id: CxId` | `CALYX_GRAPH_UNKNOWN_NODE` |
| `GraphInvalidWeight` | `field: &'static str`, `value: f32` | `CALYX_GRAPH_INVALID_WEIGHT` |
| `MaxHops` | `required: usize`, `max_hops: usize` | `CALYX_PATHS_MAX_HOPS` |
| `NodeNotFound` | `id: CxId` | `CALYX_PATHS_NODE_NOT_FOUND` |

`PathsError::code(&self) -> &'static str` (const). (`graph_builder` in calyx-mincut produces `GraphInvalidWeight`/`GraphUnknownNode` from this enum.)

### 2. Graph representation (`graph.rs`)

The directed weighted graph is a **CSR-style adjacency** built once, then immutable.

Public data structs (`Clone, Copy, Debug, PartialEq, Serialize, Deserialize`):

| Struct | Fields |
|---|---|
| `NodeEntry` | `id: CxId`, `frequency_weight: f32` |
| `Edge` | `src: usize`, `dst: usize`, `weight: f32` (endpoints are node *indices*, not CxIds) |

`AssocGraph` (`Clone, Debug`; **not** Serialize) — private fields: `nodes: Vec<NodeEntry>`, `edges: Vec<Edge>`, `adj: Vec<Range<usize>>` (per-node slice into `edges`, CSR), `id_to_idx: HashMap<CxId, usize>`.

`AssocGraphBuilder` (`Clone, Debug, Default`) — private `nodes`, `id_to_idx`, `edges`.

#### 2.1 Builder
- `AssocGraph::builder() -> AssocGraphBuilder`.
- `add_node(&mut self, id, frequency_weight: f32) -> Result<&mut Self>` — `validate_frequency_weight`: finite **and `> 0.0`** (else `GraphInvalidWeight{field:"frequency"}`); duplicate id → `GraphDuplicateNode`. Assigns insertion index.
- `add_edge(&mut self, src: CxId, dst: CxId, weight: f32) -> Result<&mut Self>` — `validate_edge_weight`: finite and in `[0.0,1.0]` (else `GraphInvalidWeight{field:"edge"}`); both endpoints must already exist (`GraphUnknownNode`). Stored as index pair. Returns `&mut Self` for chaining.
- `build(self) -> AssocGraph` — (1) **re-sort nodes ascending by `CxId`**, building `old_to_new` index remap; (2) remap edge endpoints and **deduplicate parallel edges by `(src,dst)` keeping `max(weight)`** via a `BTreeMap` (self-loops are kept — only true parallels merge); (3) `build_ranges` constructs CSR offsets by counting out-edges per src and prefix-summing (edges are grouped contiguously per source because the dedup BTreeMap is `(src,dst)`-ordered); (4) rebuild `id_to_idx`. Test `graph_parallel_self_loop_and_invalid_weights_are_handled` confirms two parallel a→b edges (0.3, 0.7) collapse to weight 0.7 and a self-loop a→a (0.4) is retained ⇒ edge_count 2.

Complexity: build is O(N log N + E log E).

#### 2.2 Queries (read-only accessors on `AssocGraph`)

| Method | Signature | Notes / complexity |
|---|---|---|
| `node_count` | `(&self) -> usize` | |
| `edge_count` | `(&self) -> usize` | |
| `is_empty` | `(&self) -> bool` | nodes empty |
| `nodes` | `(&self) -> &[NodeEntry]` | |
| `edges` | `(&self) -> &[Edge]` | |
| `node_ids` | `(&self) -> impl Iterator<Item=CxId>` | ascending CxId order |
| `node_index` | `(&self, id) -> Option<usize>` | hash lookup O(1) |
| `require_node_index` | `(&self, id) -> Result<usize>` | else `GraphUnknownNode` |
| `node_id` | `(&self, index) -> Option<CxId>` | |
| `edge_endpoints` | `(&self, edge: Edge) -> (CxId, CxId)` | |
| `out_edges_by_index` | `(&self, index) -> &[Edge]` | CSR slice, O(1) |
| `out_neighbors` | `(&self, id) -> Result<&[Edge]>` | |
| `incoming_edges_by_index` | `(&self, index) -> impl Iterator<Item=Edge>` | **linear scan of all edges**, O(E) |
| `out_degree` | `(&self, id) -> Result<usize>` | |
| `in_degree` | `(&self, id) -> Result<usize>` | O(E) scan |
| `node_weight` | `(&self, id) -> Result<f32>` | frequency_weight |

In-edge access is O(E) (no reverse CSR); this matters for the bidirectional backward expansion (§4).

### 3. Attenuation (`attenuation.rs`)

Constant `HOP_DECAY: f32 = 0.9`.
- `pub fn attenuate(base_score: f32, hops: u32) -> f32` = `base_score * 0.9^hops`.
- `pub fn deattenuate(attenuated: f32, hops: u32) -> f32` = `attenuated / 0.9^hops` (inverse).

Test: `attenuate(1.0,0)=1.0`, `(1.0,1)=0.9`, `(1.0,10)≈0.34867844`; `deattenuate(attenuate(0.42,7),7)=0.42`.

### 4. Traversal (`traversal.rs`)

`BidirectionalPath` (`Clone, Debug, PartialEq, Eq`): `forward: Option<Vec<CxId>>`, `reverse: Option<Vec<CxId>>`.

#### 4.1 `reach(graph, src, dst, max_hops) -> Result<Option<Vec<CxId>>>`
**Algorithm: bidirectional BFS shortest (fewest-hops) path.**
1. Empty graph → `NodeNotFound{src}`. Resolve `src`/`dst` indices (`require_present`, else `NodeNotFound`).
2. `src==dst` → `Ok(Some(vec![src]))`.
3. `shortest_path_indices`: two `Frontier`s (forward from src using out-edges, backward from dst using in-edges), each a `VecDeque` frontier plus `parents: HashMap<usize, Option<usize>>`. Each round expands the **smaller** frontier one BFS layer (`expand_forward`/`expand_backward`); when an expansion touches a node already in the other side's `parents`, that node is the meeting point. `reconstruct` walks forward parents src→meet (reversed) then backward parents meet→dst, producing the full index path.
4. `hops = path.len()−1`; if `hops > max_hops` → `MaxHops{required:hops, max_hops}`. Else map indices to CxIds.
5. No path found → `Ok(None)`.

Complexity: O(V + E) worst case; backward expansion uses the O(E) in-edge iterator so each backward layer is O(E). Returns the unweighted shortest path (BFS — edge weights ignored for `reach`). Tests: linear chain 1→4 returns `[1,2,3,4]`; `reach(self,self,0)=Some([self])`; `reach(1,2,0)` on a chain → `MaxHops`; disconnected → `Ok(None)`; empty graph → `NodeNotFound`.

#### 4.2 `bidirectional(graph, question, answer, max_hops) -> Result<BidirectionalPath>`
Calls `reach` twice: `forward = reach(question, answer)`, `reverse = reach(answer, question)`. Test confirms both directions reported independently.

#### 4.3 `reach_scored(graph, src, max_hops) -> Result<Vec<(CxId, f32)>>`
**Algorithm: weighted best-first BFS with multiplicative score and hop attenuation.**
1. Empty graph → `NodeNotFound`. Resolve src.
2. `best: BTreeMap<usize, ScoredReach>` and a `VecDeque` queue seeded with `{node:src, hops:0, raw_score:1.0}`. `ScoredReach{node,hops,raw_score}`; `ranked_score() = attenuate(raw_score, hops)`.
3. Pop front; skip if `hops == max_hops`. For each out-edge: `hops+1`, `raw_score = current.raw_score * edge.weight` (product of edge weights along the path). Update `best[dst]` and re-enqueue **only if** the new node's `ranked_score()` exceeds the known one (`is_none_or`). This is a relaxation that re-expands when a better-scoring route is found.
4. Result: all reached nodes except src, mapped to `(CxId, attenuate(raw_score, hops))` — final score = product-of-edge-weights × `0.9^hops`.

Complexity: bounded by O((V+E) · re-relaxations) — not a strict Dijkstra; can re-enqueue. Tests: linear unit-weight chain from node 1 yields scores `0.9, 0.81, 0.729` at hops 1/2/3; scores strictly decrease with hops (proptest).

Private helpers: `require_present`, `shortest_path_indices`, `expand_forward`, `expand_backward`, `reconstruct`, `path_to_ids`; private structs `ScoredReach`, `Frontier`.

---

## 5. Integration points (lodestar)

From [12_lodestar_kernel.md](12_lodestar_kernel.md) §1, §2, §3, §6, §8 — which primitive each lodestar operation uses:

| lodestar step / field | Primitive used | Crate |
|---|---|---|
| Graph construction from Loom cross-terms | `build_assoc_graph`, `AgreementEdge` (+ `FrequencyEntry`, `CitationEdge`) | calyx-mincut |
| Underlying graph type / builder | `AssocGraph` / `AssocGraphBuilder` (node `frequency_weight ∈ (0,∞)`, edge `weight ∈ [0,1]`) | calyx-paths |
| Pipeline step 2: SCC condensation; incremental SCC tracking; acyclicity test; cyclic-SCC lower bound | `tarjan_scc → SccResult{components, component_of}`, `condensate`, `CondensedGraph::is_dag` | calyx-mincut |
| Pipeline step 3: candidate centrality scoring (`betweenness_score`) | `betweenness → BTreeMap<CxId,f64>` (Brandes, reciprocal weights, `(n-1)(n-2)` norm) | calyx-mincut |
| MFVS LP rounding input (`lp_round_kernel_graph_from_solution`) | `LpSolution`, `SolveStatus` (LP scaffold) — **solver not wired; heuristic fallback used by default** | calyx-mincut |
| `kernel_answer`: anchor→query path | `reach(graph, from, to, max_hops) → Option<Vec<CxId>>` (bidirectional BFS) | calyx-paths |
| Per-hop answer score decay | `attenuate(score, hops) = score * 0.9^hops` (`HOP_DECAY=0.9`) | calyx-paths |
| `groundedness_distance` / `groundedness_score` | BFS hops to nearest anchor (uses `reach`/graph hop search), capped at `max_groundedness_distance` | calyx-paths |
| Error surfacing | `PathsError` / `MincutError` mapped into lodestar's `Graph{code,message}`, preserving upstream code (e.g. `CALYX_PATHS_MAX_HOPS`) | both |

The spectral module (`eigenvector_centrality`, `laplacian_eigenmaps`, GFT, `SpectralCache`) is exercised by `tests/ph52_spectral_tests.rs` but is **not referenced** in the lodestar pipeline integration table; it is a standalone structural-analysis facility (the in-source comment positions spectral centrality as a candidate proposer, with grounding as the confirmer).

---

## 6. Gaps / not covered

- **No min-cut / max-flow / feedback-vertex-set solver in calyx-mincut** despite the crate name. `lp_scaffold` only *formulates* the MFVS LP (box-bounded node indicators, minimize-sum objective) and is missing cycle-elimination constraints; there is no solver. `LpSolution`/`SolveStatus` are inert data carriers. Per the lodestar doc the kernel falls back to a heuristic DFVS approximation and emits `CALYX_KERNEL_LP_UNAVAILABLE`.
- **`MincutError::NodeNotFound`** (`CALYX_MINCUT_NODE_NOT_FOUND`) is defined but not constructed anywhere in this crate as read.
- **`reach` ignores edge weights** (pure hop-count BFS); only `reach_scored` uses weights.
- **In-edge access is O(E)** (no reverse adjacency), affecting `in_degree`, `incoming_edges_by_index`, and the backward half of bidirectional `reach`.
- **`gft_project` / `gft_reconstruct` panic** (via `assert_eq!`) on dimension/count mismatch rather than returning an error.
- **Spectral routines treat the directed graph as undirected** (symmetrized by `max` over both directions) and use dense O(n²)/O(n³) matrices — no sparse path.
- **Tarjan SCC is recursive**, so very deep graphs can overflow the stack; no iterative fallback.
- `betweenness` uses a linear `min_unvisited` scan rather than a heap → O(V·(V²+E)).
- `AssocGraph` itself is **not `Serialize`/`Deserialize`** (only its `NodeEntry`/`Edge` components are); it must be rebuilt via the builder rather than deserialized directly.
