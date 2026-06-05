# 08 — Lodestar: the Grounding-Kernel Engine

> **Living-system role:** identity — the grounding kernel is the core that defines what this intelligence is (A31 — DOCTRINE §1b)

Implements A10/A11. *"Autonomously identifies the kernel of any dataset or domain"* and lets you *"use the kernel to find answers."* The most novel database capability in Calyx — no existing DB has it.

## 1. The claim it operationalizes

Vincent-Lamarre et al.: model a dictionary as a directed definitional graph; ≈99% is recoverable, by association alone, from a **grounding kernel** of ≈1% (a minimum feedback vertex set) that must be **anchored outside language**. The mental lexicon has the same small-world, scale-free shape. Calyx generalizes this from dictionaries to *any* corpus of constellations.

> The grounding kernel is the smallest set of constellations such that, once they are anchored to real outcomes, the rest of the corpus is reconstructable / answerable by association.

## 2. Building the association graph

Lodestar runs on a directed **definition/association graph** `G` over constellations (or over regions/named clusters for scale):

- **Nodes:** constellations (or named regions from `09`/`10` for billion-scale corpora — kernel-of-regions, then drill down).
- **Edges:** `a → b` ("a is defined/explained with reference to b") derived from:
  - high cross-lens **agreement** + directional **asymmetric** lenses (cause→effect, defines→defined) from Loom,
  - retrieval edges (b is consistently in a's neighborhood across lenses),
  - explicit citation/provenance edges (Ledger), entity links (entity lens).
- **Anchored nodes:** constellations carrying real `Anchor`s (A2) are the only legitimate "outside-language" grounding points.
- **Frequency = importance (A29, `25 §4c`):** a constellation's **recurrence frequency** (how often the same action recurred over time) raises its in-degree/weight — recurring events are reinforced and are strong **kernel candidates** (the lexicon grows by differentiation; frequent associations strengthen). The non-recurring outlier is high-information but not kernel. Frequency feeds node weighting in stages 2–3.

Edge weights = agreement × directional confidence; the graph is sparse (scale-free), which the algorithms below exploit.

## 3. Kernel discovery (directed MFVS, autonomous)

The grounding kernel ≈ a **minimum feedback vertex set (MFVS)** of `G` (remove it → the rest is a DAG that "bottoms out" at anchors). Directed MFVS is NP-hard, so Lodestar uses a staged, approximate, scale-aware pipeline:

| Stage | Method | Output |
|---|---|---|
| 1. Condense | Tarjan SCC condensation | collapse strongly-connected blobs; acyclic part is already groundable |
| 2. Kernel-graph (~10%) | high-in/out-degree + high-betweenness + low-groundedness-distance nodes; LP-relaxation rounding | the ≈10% "kernel graph" (paper's intermediate set) |
| 3. MFVS (~1%) | approximate directed FVS on the kernel-graph: LP-relaxation `O(log τ* log log τ*)`-approx, then local search; on near-tournament/bounded-genus subgraphs use the 2-approx / `O(g)`-approx specializations | the ≈1% **grounding kernel** (minimum feedback vertex set) |
| 4. Anchor check | verify each kernel node reaches an `Anchor` (A2); unanchored kernel nodes are flagged "needs grounding" | grounded kernel + grounding gaps |
| 5. Recall test | reconstruct/answer held-out nodes from kernel-only; measure **kernel-only recall vs full recall** | the trust metric (A10 gate: ≥0.95·full) |

Stages 1–3 are graph algorithms in `calyx-mincut`/`calyx-paths` (ContextGraph already ships `context-graph-mincut`/`-paths`/`-solver`; Calyx absorbs them). **Incremental**: as constellations arrive, Anneal re-evaluates the kernel, not recomputed from scratch.

## 4. The kernel is an index and an answer-path (A11)

Two production uses, not just a diagnostic:

### 4.1 Kernel-anchored retrieval
The kernel constellations get a dedicated `idx/kernel/` index. A query routes **kernel-first**: find the nearest kernel nodes (tiny set, fast), then expand by association edges to the specific answer. Sublinear and high-precision because the kernel is the corpus's "table of contents anchored to reality." For huge vaults, kernel-of-regions → region → constellation is a 3-hop funnel.

### 4.2 Kernel-based answering
Because ≈99% is recoverable from the kernel by association, Calyx can answer a query by:
1. ground the query at its nearest **anchored** kernel nodes,
2. traverse association edges (with hop-attenuation `0.9^hop`, as in ContextGraph causal chains) toward the query's region,
3. compose the answer from the path, with every hop provenance-stamped (`11`).

Retrieval that *reasons over the grounded skeleton* rather than brute-forcing the whole corpus — and degrades gracefully (a missing leaf is reconstructable from the kernel).

## 4b. Multi-level kernels — freedom of scope (A21)

The kernel is computed over **whatever data you point it at.** Scope is an explicit parameter; Lodestar builds the association graph for that scope and runs MFVS on it. The founder's requirement: *"let there be freedom in what data you try to calculate the kernel for — the kernel of all associations, or the kernel of a certain dataset."*

```
build_kernel(vault, scope, anchor_kind?, params?) -> Kernel
```

| Scope | Meaning | Use |
|---|---|---|
| `AllAssociations` | the entire vault's constellation graph | the kernel of everything the system knows — the global grounded skeleton |
| `Collection(id)` | one collection | the core of one dataset/corpus |
| `Domain(anchor_kind)` | constellations grounded by a given outcome | "what passes tests" vs "what users thumbs-up" have different kernels |
| `Subgraph(query)` | the neighborhood a query touches | a *local* kernel for fast, precise answering |
| `TimeWindow(t0,t1)` | constellations in a time range | the kernel of a period (drift, what mattered then) |
| `Tenant(id)` | one tenant's data | per-user core |
| `Filter(predicate)` | any scalar/metadata/anchor filter | arbitrary slice — full freedom |
| `Union/Intersect(scopes)` | composed scopes | kernel of A∩B, bridges between A and B |

Properties:
- **Nested & incremental.** A `Subgraph` kernel is a refinement of the `Collection` kernel; Lodestar caches by `(scope_hash, panel_version)` and updates incrementally as data arrives (Anneal), so re-asking a scope is cheap.
- **Hierarchical (kernel-of-kernels).** For huge scopes, Lodestar computes a kernel **of named regions** first, then drills into the kernel of a region — a multi-level grounded skeleton (kernel of clusters → kernel of constellations).
- **Composable answering.** `kernel_answer(scope)` routes a query through that scope's kernel; a local `Subgraph` kernel gives fast precise answers, the global kernel gives breadth.
- **Each scope reports its own** measured kernel size, kernel-only recall, grounded fraction, and grounding gaps — never an assumed 1% (`§7`).

"Calculate the kernel on many levels" as a first-class, parameterized operation — the same MFVS machinery, any slice of the data, any depth.

## 5. Per-domain kernels & transfer

- A vault can hold **multiple kernels**: one per `AnchorKind` (the kernel for "what passes tests" differs from the kernel for "what users thumbs-up"). Lodestar keys kernels by `(panel_version, anchor_kind, corpus_shard)`.
- **Cross-domain bridge:** because cross-terms bridge domains (paper's "general" claim), a kernel node in domain A can edge to domain B; Lodestar exposes bridge nodes — the constellations that ground two domains at once (high value, e.g. the "want-cause"/"give-cause" hinges in civic matching, or a function that both tests and docs reference in code).

## 6. Outputs

```
Kernel {
  kernel_id, panel_version, anchor_kind, corpus_shard_hash,
  members: Vec<CxId>,                  // the ≈1%
  kernel_graph: Vec<CxId>,             // the ≈10%
  groundedness: { reached_anchor: f32, unanchored_members: Vec<CxId> },  // grounding gaps
  recall: { kernel_only, full, ratio },// A10 trust gate
  built_at, estimator_provenance,      // reproducibility (11)
}
```

`unanchored_members` is actionable: it names *exactly which constellations need a real outcome label* to fully ground the domain — the cheapest possible grounding plan.

## 7. Honesty & limits (binding)

- A kernel computed over an **ungrounded** graph is tagged `provisional` and MUST NOT be sold as "the grounded core" (A2). Lodestar reports the grounded fraction.
- The ≈1%/≈99% figures are *targets observed in dictionary/lexicon graphs*, not guarantees for every corpus; Lodestar reports the **measured** kernel size and kernel-only recall for the actual data, never an assumed 1%.
- MFVS approximation factor and the recall test are reported so the kernel's quality is auditable, not asserted.

## 8. Lodestar API (summary; full in `18`)

```
build_kernel(vault, scope, anchor_kind?, params?) -> Kernel   // scope = any slice (4b): all / collection / domain / subgraph / time / tenant / filter / union
kernel_search(vault, query, scope, anchor_kind?) -> ranked hits (kernel-first funnel)
kernel_answer(vault, query, anchor_kind) -> {answer_path, provenance, hop_scores}
grounding_gaps(vault, anchor_kind) -> [CxId needing an anchor]   // cheapest grounding plan
bridges(vault, anchor_a, anchor_b) -> [CxId grounding both domains]
kernel_health(kernel_id) -> {size, recall_ratio, grounded_fraction, approx_factor}
```

**One sentence:** Lodestar finds the ≈1% of your data that, once anchored to reality, explains and answers the other ≈99% — using it as both an index and a reasoning path.

Sources: [Feedback vertex set / directed FVS approximation](https://en.wikipedia.org/wiki/Feedback_vertex_set); [`O(log τ* log log τ*)` directed multicut/FVS](https://link.springer.com/article/10.1007/PL00009191); [2-approx FVS in tournaments](https://arxiv.org/pdf/1809.08437); [bounded-genus `O(g)`-approx DFVS](https://arxiv.org/abs/2311.01026); Vincent-Lamarre et al. (2016) grounding kernel; Gärdenfors *Conceptual Spaces* (object category = the constellation at an index); Steyvers & Tenenbaum (lexicon grows by differentiation, small-world/scale-free).
