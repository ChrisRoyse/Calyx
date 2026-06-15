# 08. Sextant Search & Navigation (calyx-sextant)

This reference documents **only what the source code in `crates/calyx-sextant` actually does**. Every claim is traced to a file path and a type/function. Where behaviour cannot be determined from source it is marked **"Not determined from source"**.

See sibling specs: [07_registry_lenses.md](07_registry_lenses.md) (lens/slot registry), and the temporal/causal material here cross-references the same `SlotId` conventions used by the registry.

## Source files covered

- `src/lib.rs` — crate entry / public re-exports.
- `src/index/mod.rs` — `SextantIndex` trait, `IndexSearchHit`, `IndexStats`, `ranked()`.
- `src/index/hnsw/mod.rs`, `src/index/hnsw/graph.rs`, `src/index/hnsw/scored.rs` — in-RAM HNSW dense index.
- `src/index/diskann/mod.rs`, `.../build.rs`, `.../search/mod.rs`, `.../graph.rs`, `.../concat.rs`, `.../token.rs` — DiskANN/Vamana on-disk dense index.
- `src/index/inverted.rs`, `src/index/bm25.rs`, `src/index/tokenizer.rs` — sparse inverted index + BM25.
- `src/index/multi.rs` — MaxSim late-interaction multi-vector index.
- `src/index/dual.rs` — dual directional index.
- `src/index/quant_config.rs` — per-slot quantization policy.
- `src/fusion/mod.rs`, `.../rrf.rs`, `.../single.rs`, `.../profiles.rs`, `.../pipeline.rs` — fusion strategies.
- `src/hit.rs` — provenanced hit types.
- `src/slot_index_map.rs` — slot→index registry.
- `src/search.rs`, `src/search_support.rs`, `src/guarded.rs` — search engine + guarded search.
- `src/reranker.rs` — HTTP reranker client.
- `src/planner.rs`, `src/planner_explain.rs` — intent planner + explain (Stage-3 style).
- `src/query/mod.rs`, `.../search.rs`, `.../planner.rs` — universal cross-model query + cost planner.
- `src/query_admission.rs` — query admission control.
- `src/navigation/*` — neighbors, consensus, traverse, skills (HDBSCAN*).
- `src/temporal/*` — temporal/causal post-retrieval scoring.
- `src/error.rs` — error taxonomy.
- `src/util.rs` — cosine, top_k, stub ledger, hex.

---

## 1. Index abstractions

### 1.1 The `SextantIndex` trait

Defined in `src/index/mod.rs`. Every per-slot index implements `SextantIndex: Send + Sync` with these methods:

| Method | Signature | Behaviour |
|--------|-----------|-----------|
| `slot` | `fn slot(&self) -> SlotId` | Slot this index serves. |
| `shape` | `fn shape(&self) -> SlotShape` | Dense/Sparse/Multi shape. |
| `insert` | `fn insert(&mut self, cx_id, vector: SlotVector, seq: u64) -> Result<()>` | Insert/replace a vector. |
| `search` | `fn search(&self, query: &SlotVector, k, ef: Option<usize>) -> Result<Vec<IndexSearchHit>>` | Top-k ANN search. |
| `rebuild` | `fn rebuild(&mut self) -> Result<()>` | Rebuild graph/postings. |
| `vector` | `fn vector(&self, cx_id) -> Option<SlotVector>` | Recover stored vector. |
| `set_base_seq` | `fn set_base_seq(&mut self, seq)` | Set ledger base sequence. |
| `stats` | `fn stats(&self) -> IndexStats` | Stats (slot, shape, len, seqs, kind). |
| `insert_text` | default `Err(CALYX_SEXTANT_VECTOR_SHAPE)` | Overridden by inverted index. |
| `search_text` | default `Err(CALYX_SEXTANT_VECTOR_SHAPE)` | Overridden by inverted index. |
| `candidate_text` | default `None` | Overridden by inverted index (reranker text). |

`ranked(scored: Vec<(CxId, f32)>) -> Vec<IndexSearchHit>` (same file) assigns 1-based ranks to a scored list.

`IndexSearchHit { cx_id, score: f32, rank: usize }` and `IndexStats { slot, shape, len, built_at_seq, base_seq, kind: &'static str }` are the common result/stat types.

### 1.2 Index implementations (the `kind` registry)

| Type | File | `stats().kind` | `SlotShape` | Notes |
|------|------|----------------|-------------|-------|
| `HnswIndex` | `index/hnsw/mod.rs` | `"hnsw"` | `Dense(dim)` | In-RAM HNSW, default for embedded vaults. |
| `DiskAnnSearch` | `index/diskann/search/mod.rs` | `"DiskANN"` | `Dense(dim)` | On-disk Vamana graph, server-only. |
| `InvertedIndex` | `index/inverted.rs` | `"inverted"` | `Sparse(1_000_000)` | BM25 over an inverted postings map. |
| `MaxSimIndex` | `index/multi.rs` | `"multi_maxsim"` | `Multi { token_dim }` | Late-interaction multi-vector. |
| `DualIndex` | `index/dual.rs` | `"dual"` | mirrors side A | Two `HnswIndex` halves (A/B) with directional score boosts. |

Distance kernel for dense indexes is **cosine similarity** (`util::cosine`): `dot / (||a|| * ||b||)`, returning `0.0` if either norm is zero. `util::top_k` sorts by score descending with a `cx_id.to_string()` tie-break (deterministic) and truncates to `k`.

---

## 2. Dense indexes

### 2.1 In-RAM HNSW (`index/hnsw/`)

`HnswIndex` (`index/hnsw/mod.rs`) holds `rows: Vec<Row>` where `Row { cx_id, vector: Vec<f32>, seq, level: u8, neighbors: Vec<usize> }`, a `positions: HashMap<CxId, usize>`, a `fingerprints: HashMap<[u8;32], Vec<usize>>` for exact-duplicate recall, and an `entry_point: Option<usize>`.

**Config / constants (HNSW):**

| Knob | Value | Location |
|------|-------|----------|
| `max_neighbors` (M) | `32` | `HnswIndex::new` |
| level assignment | `blake3(cx_id ‖ seed ‖ ordinal)[0].trailing_zeros().min(6)` → level `0..=6` | `level_for` |
| `EXACT_CONSTRUCTION_ROWS` | `4096` | `hnsw/graph.rs` (exhaustive build below this) |
| `CONSTRUCTION_EF` | `64` | `hnsw/graph.rs` |
| `RECENT_CONSTRUCTION_SCAN` | `128` | `hnsw/graph.rs` |
| `CONSTRUCTION_VISIT_LIMIT_EF_MULTIPLIER` | `1` | `hnsw/graph.rs` |
| `SEARCH_VISIT_LIMIT_EF_MULTIPLIER` | `16` | `hnsw/graph.rs` |
| default search `ef` | `ef.unwrap_or(needed.max(max_neighbors*2)).min(rows.len())` | `HnswIndex::search` |

The level RNG is seeded by `cx_id`+`seed`+ordinal, so the graph topology is **deterministic** for a given insert order and seed.

**HNSW build steps** (insert → `connect_new_row` in `hnsw/graph.rs`):
1. Row 0 connects to nothing.
2. Build candidate neighbor set: if `index <= EXACT_CONSTRUCTION_ROWS`, do an **exhaustive** cosine scan over all earlier rows (`exhaustive_candidates`); otherwise do an **approximate** set (`approximate_candidate_set`): greedy-descend from the highest-level entry point, beam-search `ef = CONSTRUCTION_EF` (clamped), then union the last `RECENT_CONSTRUCTION_SCAN` rows and power-of-two "stride" back-pointers (`append_stride_neighbors`: indices `i-1, i-2, i-4, i-8, …`).
3. Take top-`max_neighbors` per HNSW level via `top_k_indices`.
4. `prune_neighbors`: replace neighbor list with `diversified_neighbors` (a relative-neighborhood / RobustPrune-style diversification keyed to `max_neighbors`).
5. Add back-edges into each chosen neighbor, then re-prune that neighbor.
6. `refresh_entry_after_insert`: entry point becomes the node with the highest `level`.

Re-inserting an existing `cx_id` updates the vector and calls full `rebuild()` (clears all neighbors, reconnects every row in order).

**HNSW search steps** (`HnswIndex::search`):
1. Errors `CALYX_SEXTANT_INDEX_EMPTY` if no rows; `CALYX_SEXTANT_EF_TOO_SMALL` if `k == 0`.
2. `checked_query` validates the dense vector dimension (else `CALYX_SEXTANT_DIM_MISMATCH`).
3. Resolve `ef` (above); error `CALYX_SEXTANT_EF_TOO_SMALL` if `ef < min(k, rows.len())`.
4. `greedy_descent` from entry point down HNSW levels (hill-climb on cosine, level-restricted neighbors).
5. `beam_search` (`beam_search_indices`): best-first frontier with a `BinaryHeap`, keeping the best `ef` and stopping when `visited >= ef * SEARCH_VISIT_LIMIT_EF_MULTIPLIER` and the frontier can no longer improve the worst kept score.
6. Merge beam results with `exact_vector_hits` (bit-exact duplicates found via the `fingerprints` map, keeping the max score per `cx_id`).
7. `top_k` and truncate to `k`; assign ranks via `ranked`.

Helpers: `brute_force(query, k)` (exact cosine top-k), `recall_at(queries, k, ef)` (measures ANN/exact overlap), `neighbor_counts`, `layer_histogram`.

### 2.2 DiskANN / Vamana on-disk graph (`index/diskann/`)

Server-only NVMe-resident Vamana graph (`index/diskann/mod.rs` header comment). Embedded vaults keep the in-RAM HNSW.

**On-disk format** (`index/diskann/graph.rs`):

| Constant | Value |
|----------|-------|
| `DISKANN_MAGIC` | `b"CLXDA001"` |
| `DISKANN_FORMAT_VERSION` | `1` |
| `DISKANN_BLOCK_ALIGN` | `4096` (4 KiB page-aligned blocks) |
| `DISKANN_MAX_DIM` | `8192` |
| `DISKANN_MAX_M` | `512` |
| `node_block_size(dim, m_max)` | `(dim*4 + 4 + m_max*4)` rounded up to 4 KiB |

`DiskAnnHeader { format_version, dim, m_max, max_degree, entry_point_id, node_count }` occupies the first aligned block. Each node block stores `[raw f32 vector | neighbor_count: u32 | neighbors: [u32; m_max] zero-padded]`; node `id` lives at byte offset `HEADER + id * node_block_size`. Reader uses `memmap2::Mmap`.

**Build params** `DiskAnnBuildParams { dim, m_max, ef_construction, alpha }` (`build.rs`). `validate()` enforces `1..=8192` dim, `1..=512` m_max, `ef_construction >= 1`, `alpha` finite in `1.0..=4.0`.

Build constants: `BUILD_SEED = 42`, `BUILD_BATCH_MIN = 256`, `BUILD_BATCH_DIVISOR = 32` (batch cap `n/32`, min 256). IDs must be dense `0..n`.

**Vamana build steps** (`build::vamana`):
1. L2-normalize all vectors (`normalize`); distance kernel becomes `dist = 1 - dot` (= `1 - cosine`). Graph file stores *original* vectors verbatim.
2. Entry = `medoid` (point closest to the normalized centroid).
3. Seed each node with random init edges (`ChaCha8Rng(BUILD_SEED)`, `m_max` random neighbors).
4. **Two passes**, `alpha = 1.0` then `alpha = params.alpha`. Each pass: shuffle order, advance in geometrically growing batches. Within a batch every point greedy-searches the *same frozen adjacency snapshot* in parallel (`rayon`), then edges are applied sequentially → deterministic regardless of thread count.
5. Per point: `greedy_search` from medoid with `ef = max(ef_construction, m_max)`, then `robust_prune(p, candidates, alpha, m_max)`.
6. Back-edges grouped by target in a `BTreeMap`, each affected node re-pruned once per batch (in parallel) if its degree exceeds `m_max`.

`robust_prune`: sort candidates by distance to `p`; repeatedly keep the closest (`star`) and drop any `c` where `alpha * dist(star, c) > dist(p, c)`, until `m_max` kept.

**Search params** `DiskAnnSearchParams { beamwidth: 32, ef_search: 64, rescore_k: 64, rescore_from_raw: true }` (defaults). `DiskAnnSearch::open` sets `build_params.alpha = 1.2`.

**DiskANN search steps** (`search/mod.rs::search_ids`):
1. Validate query dim (`CALYX_INDEX_DIM_MISMATCH`) and finiteness.
2. Error if `ef_search < min(k, n)`.
3. Beam search from `entry_point_id`: maintain a sorted `candidates` list; expand best-unexpanded node, prefetch the top `beamwidth` blocks (`prefetch_node`), read neighbors, score by `distance` (= `1 - dot`), keep min distance per id; truncate to `max(ef_search, rescore_k)`; stop via `stop_search` or once `ef_search` nodes expanded.
4. Take top `rescore_k`.
5. If `rescore_from_raw` and a `raw_sidecar` dir exists, re-score the survivors against raw f32 vectors read from the sidecar (`rescore_from_raw` / `read_raw_vector`).
6. Truncate to `k`. The `SextantIndex::search` wrapper converts distance back to similarity: `score = 1.0 - dist`.

`insert`/`rebuild` reconstruct vectors from the graph, append the new vector, and **fully rebuild** the on-disk graph (no incremental edge insertion).

Sibling DiskANN modules: `concat.rs` (`ConcatCrossTermDiskAnn` cross-term concatenation), `token.rs` (`TokenDiskAnnMaxSim` token-level MaxSim over DiskANN), `token_sidecar.rs`.

---

## 3. Sparse inverted index (`index/inverted.rs`, `bm25.rs`, `tokenizer.rs`)

`InvertedIndex` holds `docs: BTreeMap<CxId, String>`, `vectors`, `postings: BTreeMap<String, Vec<Posting>>` where `Posting { cx_id, tf: usize }`, and `doc_len: BTreeMap<CxId, usize>`. `shape()` is `Sparse(1_000_000)`.

**Build steps** (`index_text`):
1. `tokenize` the text (`tokenizer::tokenize`: Unicode-lowercase, split on non-alphanumeric, emit alphanumeric runs).
2. Record `doc_len = number of terms`; store the raw text.
3. Count term frequencies; append a `Posting { cx_id, tf }` to each term's postings vector.
4. Update `built_at_seq`/`base_seq` to `max(seq)`.

For sparse `SlotVector::Sparse` inserts, each `SparseEntry.idx` is encoded as a synthetic token `"t{idx}"` and indexed as text. `insert_text` indexes free text directly. `candidate_text(cx_id)` returns the stored doc string (used by the reranker).

**Search steps** (`search_text`):
1. Tokenize query into a `BTreeSet` of unique terms.
2. Compute `avg_len = Σ doc_len / total_docs`.
3. For each query term present in postings, for each posting compute a BM25 term score and accumulate per `cx_id`.
4. `ranked(top_k(scores, k))`.

**BM25 formula** (`bm25.rs`, Lucene-like defaults `k1 = 1.2`, `b = 0.75`):

```rust
idf = ln( (N - df + 0.5) / (df + 0.5) + 1.0 )
len_norm = doc_len / avg_doc_len            // 1.0 if avg_doc_len <= 0
denom = tf + k1 * (1.0 - b + b * len_norm)
score = idf * (tf * (k1 + 1.0)) / denom
```

`score_term` returns `0.0` when `tf == 0 || total_docs == 0 || doc_freq == 0`.

**Postings codec** (`tokenizer.rs`): `encode_varint_deltas` / `decode_varint_deltas` store sorted doc ids as LEB128 varint deltas. Out-of-order ids error `CALYX_SEXTANT_POSTINGS_NOT_SORTED`; truncated/overflow blocks error `CALYX_SEXTANT_POSTINGS_CORRUPT`.

---

## 4. MaxSim multi-vector index (`index/multi.rs`)

`MaxSimIndex` stores rows `(CxId, Vec<Vec<f32>>, seq)` (one vector per token). **MaxSim late-interaction score**:

```rust
maxsim(query, doc) = Σ_{q in query} max_{d in doc} cosine(q, d)
```

`search` requires a `SlotVector::Multi { token_dim, tokens }` matching `self.token_dim` (else `CALYX_SEXTANT_VECTOR_SHAPE`), computes MaxSim against every row, and returns `ranked(top_k(...))`. `cpu_gpu_delta` always errors `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE` (no Forge GPU path wired).

`DualIndex` (`index/dual.rs`) wraps two `HnswIndex` halves (`a`, `b`, seed `b = seed ^ 0x9e37`) with directional multipliers `boost_a_to_b` / `boost_b_to_a`; `search` defaults to side A scaled by `boost_a_to_b`.

---

## 5. Quantization (`index/quant_config.rs`)

`QuantConfig { kind: QuantKind, scale: f32, zero_point: i8, locked }`. `QuantKind ∈ { None, Scalar8, Binary }`. `lock_after_first_insert` freezes config after first vector. `quantize`:

| Kind | Encoding |
|------|----------|
| `None` | passthrough `approx = values` |
| `Scalar8` | `q = round(v / max(scale,1e-6)).clamp(-127,127)`; `approx = q*scale` |
| `Binary` | `byte = (v >= 0.0) as u8`; `approx = ±1.0` |

`cpu_gpu_delta` always errors `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE`. HNSW accepts a quant config via `with_quant` (used as policy; cosine still runs on stored f32 vectors).

---

## 6. Fusion strategies (`src/fusion/`)

`FusionStrategy` (`fusion/mod.rs`):

| Variant | Meaning |
|---------|---------|
| `SingleLens { slot }` | Take top-k from one slot, scores unchanged. |
| `Rrf` | Reciprocal Rank Fusion, uniform weight 1.0 per slot. |
| `WeightedRrf { profile: RrfProfile }` | RRF with per-slot weights from a named profile (via `FusionContext.weights`). |
| `Pipeline` | Two-stage: stage-1 slots filter the candidate set, remaining slots score it. |

`fuse(results: &BTreeMap<SlotId, Vec<IndexSearchHit>>, context: &FusionContext)` dispatches to `single_lens_fuse` / `rrf_fuse` / `weighted_rrf_fuse` / `pipeline_fuse`.

`FusionContext { k, explain: bool, strategy, weights: BTreeMap<SlotId, f32>, stage1_slots: Vec<SlotId> }`.

### 6.1 Formulas

**RRF constant** (`fusion/rrf.rs`): `const RRF_K: f32 = 60.0;`

```rust
// rrf_contribution(weight, rank):
weight / (rank as f32 + RRF_K)         // rank is 1-based; RRF_K = 60.0
```

| Method | Per-item score | Function |
|--------|----------------|----------|
| RRF | `Σ_slot 1.0 / (rank_slot + 60.0)` | `rrf_fuse` |
| WeightedRRF | `Σ_slot weight_slot / (rank_slot + 60.0)` | `weighted_rrf_fuse` (calls `fuse_with_weights(..., context.weights, 0.0)`) |
| SingleLens | `score = raw_score` (passthrough) | `single_lens_fuse` |

`rrf_fuse_restricted(results, context, candidates: &BTreeSet<CxId>)` is the RRF helper that restricts output to a candidate set (used by the pipeline). Each `Hit` carries `per_lens: Vec<PerLensContribution>` recording `{ slot, rank, raw_score, weight, contribution }`.

### 6.2 Weighted profiles (`fusion/profiles.rs`)

`weighted_profiles() -> Vec<WeightedProfile>` returns 14 named profiles. `WeightedProfile { profile: RrfProfile, weights: BTreeMap<SlotId,f32>, lexical_excludes_dense: bool }`. Within a profile, the slot list is weighted positionally: `weight = 1.0 / (idx + 1.0)` → `1.0, 0.5, 0.333…, 0.25, …`.

| Profile | Slots (SlotId) | lexical_excludes_dense |
|---------|----------------|------------------------|
| Causal | 4, 8, 18 | false |
| Code | 8, 9, 10, 11, 16 | false |
| Entity | 3, 8, 18 | false |
| Temporal | 8 | false |
| Speaker | 5, 8 | false |
| Style | 6, 8 | false |
| Civic | 1, 2, 3, 8 | false |
| Media | 8, 9, 10 | false |
| Bridge | 8, 14, 18 | false |
| Kernel | 7, 8, 15 | false |
| Semantic | 8 | false |
| Lexical | 1 | **true** |
| Multimodal | 8, 9, 10, 11 | false |
| General | 1, 8, 18 | false |

`lookup(profile) -> Option<WeightedProfile>` finds one profile by name.

**AP-60 temporal slots:** `AP60_TEMPORAL_PRIMARY_SLOTS = [SlotId(20), SlotId(21), SlotId(22)]`; `is_ap60_temporal_primary_slot(slot)` — these are kept out of primary retrieval and applied only as a post-retrieval temporal boost (see §11).

### 6.3 Pipeline steps (`fusion/pipeline.rs`)

`pipeline_fuse`:
1. If `stage1_slots` empty → return empty.
2. `stage1_candidates`: union of all `CxId` appearing in the stage-1 slots. Empty → return empty.
3. `non_stage1_results`: the remaining (scoring) slots.
4. If no scoring slots → `rrf_fuse_restricted(all results, candidates)`.
5. Else score the candidate set with `rrf_fuse_restricted(scoring_results, candidates)`; if that is empty, fall back to RRF over all slots restricted to the candidates.

---

## 7. Provenanced hits (`src/hit.rs`)

`Hit` is the public result type:

| Field | Type | Meaning |
|-------|------|---------|
| `cx_id` | `CxId` | Constellation id. |
| `score` | `f32` | Fused score. |
| `rank` | `usize` | 1-based final rank. |
| `event_time_secs` | `Option<i64>` | Event time (temporal). |
| `temporal_scores` | `Option<TemporalScores>` | E2/E3/E4 scores if temporal applied. |
| `causal_confidence` | `CausalConfidence` | Causal gate confidence. |
| `causal_gate` | `Option<CausalGateEvidence>` | Causal multiplier evidence. |
| `per_lens` | `Vec<PerLensContribution>` | Per-slot rank/weight/contribution. |
| `cross_terms_used` | `bool` | Whether cross-term expansion ran. |
| `guard` | `Option<HitGuardEvidence>` | Guard verdict (if kept by guard). |
| `provenance` | `LedgerRef` | Ledger reference (seq + 32-byte hash). |
| `provenance_source` | `ProvenanceSource` | `Stored` or `Stub`. |
| `freshness` | `FreshnessTag` | Built/base seq + policy. |
| `explain` | `Option<ExplainBreakdown>` | Strategy + provenance hex + dropped guards. |

`PerLensContribution { slot, rank, raw_score, weight, contribution }`. `FreshnessTag { built_at_seq, base_seq, stale_by, policy }` with constructors `fresh(seq)` (policy `"fresh_derived"`) and `stale_ok(built_at_seq, base_seq)` (policy `"stale_ok"`, `stale_by = base_seq - built_at_seq`). `ProvenanceSource ∈ { Stored, Stub }`; stub ledgers come from `util::stub_ledger` (`blake3(cx ‖ seq)`). `ExplainBreakdown { strategy, per_lens_count, provenance_hex, recurrence_boost, guard_dropped }`. Guard types: `HitGuardMode::InRegionOnly`, `HitGuardEvidence { mode, verdict: GuardVerdict }`, `DroppedGuardHit { cx_id, mode, reason, verdict }`.

---

## 8. Search engine & API (`src/search.rs`, `slot_index_map.rs`, `guarded.rs`)

### 8.1 `SlotIndexMap`

`SlotIndexMap { indexes: Arc<RwLock<BTreeMap<SlotId, SharedIndex>>>, states: Arc<RwLock<BTreeMap<SlotId, SlotState>>> }` (`slot_index_map.rs`). `SharedIndex = Arc<RwLock<Box<dyn SextantIndex>>>`.

| Method | Behaviour |
|--------|-----------|
| `register(index)` | Insert; errors `CALYX_SEXTANT_SLOT_ALREADY_REGISTERED` on duplicate; marks `Active`. |
| `slots()` | Active slot ids only. |
| `registered_slots()` | All registered slots. |
| `set_slot_state` / `slot_state` | Active/Inactive toggling (inactive → `CALYX_SEXTANT_SLOT_INACTIVE`). |
| `stats()` / `insert` / `insert_text` / `search` / `search_text` / `vector` / `candidate_text` / `rebuild` | Routed to the per-slot index. |
| `assert_isolated(a, b, query)` | Test utility. |

### 8.2 `SearchEngine`

`SearchEngine { indexes: SlotIndexMap, docs: BTreeMap<CxId, Constellation>, query_admission: QueryAdmissionController, assoc_graph: Option<calyx_paths::AssocGraph> }` (`search.rs`).

Public API:

| Method | Signature |
|--------|-----------|
| `new` | `fn new(indexes: SlotIndexMap) -> Self` |
| `set_query_admission_config` | `fn set_query_admission_config(&mut self, config: QueryAdmissionConfig)` |
| `query_admission_stats` / `query_admission_metrics_text` | stats / Prometheus text |
| `put_constellation` / `constellation` / `constellation_ids` | constellation store (sorted ids) |
| `set_assoc_graph` / `assoc_graph` | association graph for navigation |
| `search` | `fn search(&self, query: &Query) -> Result<Vec<Hit>>` |
| `search_with_reranker` | `fn search_with_reranker(&self, query, reranker: &RerankerClient) -> Result<Vec<Hit>>` |
| `search_with_guard_report` | `fn search_with_guard_report(&self, query) -> Result<GuardedSearchReport>` |
| `planned_search` | `fn planned_search(&self, query, planner: &QueryPlanner) -> Result<Vec<Hit>>` |
| `planned_explain_search` | `fn planned_explain_search(&self, query, planner) -> Result<PlannerExplain>` |

**End-to-end search flow** (`search_inner`):
1. Acquire a query-admission permit; validate query; resolve slots (query slots or all active; none → `CALYX_SEXTANT_NO_LENSES`).
2. `enforce_freshness` against each slot's `built_at_seq`/`base_seq`.
3. Choose fusion strategy (query override or `default_strategy`); a reranker is only allowed with `Pipeline` fusion.
4. `candidate_window`: per-slot fetch `search_k` (= `recall_k`, default ~10×k, when guarded/filtered or Pipeline; else `k`).
5. Per-slot search: inverted slots use `search_text`; dense/multi slots use vector `search` with `ef`. Collect `per_slot: BTreeMap<SlotId, Vec<IndexSearchHit>>`.
6. `fusion::fuse` with weights + `stage1_slots` (the inverted/sparse slots).
7. `apply_filters` (scalar/anchor/metadata predicates).
8. If a reranker is present, `rerank_pipeline_hits` (see §9).
9. `apply_query_guard` (in-region-only guard; collects `dropped_guard_hits`).
10. Truncate to `k`, `renumber_hits` (1-based).
11. `attach_provenance_and_freshness`: stored constellation → real `LedgerRef` + event time + freshness; otherwise a stub ledger (or `CALYX_SEXTANT_PROVENANCE_MISSING` if `require_stored_provenance`).
12. Return `GuardedSearchReport { hits, dropped_guard_hits }`.

### 8.3 Guarded search (`guarded.rs`)

`GuardedSearchReport { hits: Vec<Hit>, dropped_guard_hits: Vec<DroppedGuardHit> }`. `apply_query_guard`:
1. Only acts when the query has `QueryGuard::InRegionOnly(profile)`.
2. `validate_trusted_guard_profile` (delegates to `calyx_ward`).
3. `produced_guard_slots`: builds per-slot produced vectors from `guard_vectors` (or replicates the main query vector); multi-slot profiles require slot-aware guard vectors; non-dense vectors error.
4. Per candidate: missing constellation → drop (reason `"missing_constellation"`); else call `calyx_ward::guard_non_high_stakes`. `verdict.overall_pass == true` → keep + attach `HitGuardEvidence`; else drop with reason `"ood"`.
5. If `query.explain`, attach `dropped_guard_hits` into each hit's explain.

---

## 9. Reranker (`src/reranker.rs`)

`RerankerClient { endpoint: String, timeout: Duration }` speaks **plain HTTP/1.1 POST** (only `http://`, not `https://`).

Types: `RerankCandidateText` wraps `Zeroizing<String>` (non-serializable, redacted Debug, request-scoped); `RerankRequest { query: Zeroizing<String>, candidates: Vec<RerankCandidateText> }`; `RerankResponse { scores: Vec<f32> }`.

**Rerank protocol** (`RerankerClient::rerank`):
1. Error `CALYX_SEXTANT_RERANKER_NO_CANDIDATES` if empty; `CALYX_SEXTANT_RERANKER_ENDPOINT` if not `http://` or unresolvable.
2. Resolve host:port, `TcpStream::connect_timeout`, set read/write timeouts (timeout → `CALYX_SEXTANT_RERANKER_TIMEOUT`).
3. Serialize `{query, texts:[…]}` JSON (zeroized), `POST /rerank HTTP/1.1` with `Connection: close`.
4. Read response; require `2xx` status; parse body (`CALYX_SEXTANT_RERANKER_PROTOCOL` on malformed / non-finite / wrong count).
5. Two wire formats accepted: standard `{"scores":[…]}` and **TEI** rank-array `[{"index":i,"score":s},…]` (re-ordered into candidate order).

In `rerank_pipeline_hits`, candidate text is pulled from the stage-1 (inverted) slots' `candidate_text`, hits are re-sorted by reranker score (stable tie-break), and scores/ranks updated. Reranker output is not persisted.

---

## 10. Query planner & explain

### 10.1 Intent planner (`src/planner.rs`, `planner_explain.rs`)

`QueryPlanner { limits: PlanLimits }`. `PlanLimits` defaults: `max_k = 100`, `max_ef = 512`, `max_slots = 16`, `max_cost = 20_000_000`, `timeout_ms = 5_000`.

**Planning stages** (`plan(query, index_size)`):
1. `classify` → `IntentLabel` (14 variants: Causal, Code, Entity, Temporal, Speaker, Style, Civic, Media, Bridge, Kernel, Semantic, Lexical, Multimodal, General — keyword classification, `General` default).
2. Detect override (`query.fusion.is_some()`).
3. `strategy_for(intent, query)` unless overridden → a `FusionStrategy`.
4. `enforce_bounds` (k/ef/slots/index_size).
5. `estimate_cost(query, index_size)` → `u64`.
6. `enforce_cost` against `max_cost`.
7. Set `query.fusion`; return `PlannedQuery { query, intent, strategy, override_used, cost_estimate, timeout_ms }`.

`PlannerExplain { intent, strategy, override_used, cost_estimate, timeout_ms, hits: Vec<Hit> }` is produced by `PlannerExplain::new(plan, hits)` — planner metadata plus the executed hits.

### 10.2 Universal cross-model query (`src/query/`)

`UniversalQuery` (`query/mod.rs`) carries optional sub-queries: `relational`, `document`, `kv`, `timeseries`, `graph_hop`, `vector: VectorQuery { lens_ids, query_vec, limit }`, `aggregate`, `ask`, plus `cost_cap_ms`, `explain`, `isolation: IsolationLevel`.

`plan(vault, query) -> Result<CrossModelPlan>` (`query/planner.rs`) builds an ordered list of `PlanStep` (RelationalScan, DocScan, KvGet, TsRangeScan, GraphHop, VectorFusion, Aggregate, Ask) and an `ExplainOutput { steps: Vec<ExplainStep>, total_cost_ms }` where `ExplainStep { ordinal, kind: PlanStepKind, estimated_cost_ms, chosen_index }`.

**Cost model constants** (`query/planner.rs`):

| Constant | ms |
|----------|----|
| `DEFAULT_COST_CAP_MS` | `30_000` |
| `MIN_FULL_SCAN_COST_MS` | `50.0` |
| `FULL_SCAN_COST_PER_1K_ROWS_MS` | `50.0` |
| `INDEX_SCAN_COST_MS` | `5.0` |
| `KV_GET_COST_MS` | `0.1` |
| `TS_COST_PER_1K_POINTS_MS` | `1.0` |
| `GRAPH_HOP_COST_MS` | `10.0` |
| `VECTOR_LENS_COST_MS` | `5.0` |
| `ASK_COST_MS` | `200.0` |
| `DOC_SCAN_MIN_COST_MS` | `25.0` |
| `AGGREGATE_COST_MS` | `1.0` |

`full_scan_cost(rows) = max(50.0, rows/1000 * 50.0)`. Cost cap precedence: query `cost_cap_ms` → collection policy cap → `DEFAULT_COST_CAP_MS`; exceeding it errors `CALYX_PLANNER_COST_CAP`.

The Stage-4 `Query` (`query/search.rs`) fields: `text`, `vector`, `guard_vectors`, `slots`, `k` (default 10), `ef` (default `Some(64)`), `recall_k`, `explain`, `require_stored_provenance`, `freshness: FreshnessRequirement` (FreshDerived | StaleOk{seq_lag}), `fusion`, `filters: QueryFilters`, `guard: Option<QueryGuard>`.

### 10.3 Admission control (`src/query_admission.rs`)

`QueryAdmissionConfig { max_concurrent: 128, max_queued: 512, queue_timeout: 250ms }`. `QueryAdmissionController::acquire`:
1. `max_concurrent == 0` → reject (`queue_full_rejected_total++`).
2. Admit immediately if `in_flight < max_concurrent`.
3. If queue at `max_queued` → reject (queue full).
4. Else enqueue and `wait_for_permit`: loop until admitted, deadline (`queue_timeout`) → reject (`deadline_rejected_total++`).

`QueryAdmissionPermit` decrements `in_flight` and notifies one waiter on drop. `QueryAdmissionStats` exposes in_flight/queued counters and cumulative admitted/queued/rejected totals (+ Prometheus `metrics_text`).

---

## 11. Temporal & causal post-retrieval scoring (`src/temporal/`)

Temporal scoring is **additive, post-retrieval, and never dominant** (AP-60). Violations error `CALYX_TEMPORAL_AP60_VIOLATION`.

Three signals (`TemporalScores { e2_recency, e3_periodic, e4_sequence }`, all `[0,1]`):
- **E2 recency** (`score_e2_recency`): age decay via `DecayFunction` (`Linear{max_age_secs}`, `Exponential{half_life_secs}`, `Step` brackets 0.8/0.5/0.1).
- **E3 periodic** (`score_e3_periodic`): hour-of-day / day-of-week target match.
- **E4 sequence** (`score_e4_sequence`): position in the result list.

`fuse_temporal(scores, weights: FusionWeights{recency,periodic,sequence}) = (w_r·e2 + w_s·e4 + w_p·e3).clamp(0,1)`. `apply_temporal_boost(hits, policy, query_time_secs, tz_offset)` multiplies each hit by `1 + fuse_temporal*alpha` (`BoostConfig.post_retrieval_alpha`, AP-60-capped at 0.10), then re-ranks. `temporal_search(engine, query, window, policy, clock, tz_offset)` is the windowed entry point (validates primary slots, runs windowed primary search, filters by window, applies boost + causal gate).

**Causal gate** (`temporal/causal_gate.rs`): `causal_gate_mult(confidence, cfg)` → `causal_high_mult` (≈1.10) for High, `causal_low_mult` (≈0.85) for Low, `1.0` for Neutral/Absent; confidence is derived from hit metadata / ward verdicts (`derive_causal_confidence`). `apply_causal_gate` re-ranks after multiplying.

**Window recall budget** (`temporal/recall_budget.rs`): `WindowRecallPolicy` is `Exhaustive` (one pass over the union bound of primary-slot lengths) or `Bounded { max_candidates }` (geometric ×4 deepening until k in-window rows found, corpus exhausted, or cap hit → `CALYX_TEMPORAL_WINDOW_BUDGET_EXHAUSTED`). `WindowRecallReport` records requested_k, in_window_count, candidates_fetched, rounds, effective_budget, corpus_exhausted.

`recurrence_boost` (`temporal/recurrence_boost.rs`) adds a frequency+recency multiplier from `AsterVault` recurrence scalars; corrupt scalars error `CALYX_SEXTANT_RECURRENCE_READ_ERROR`.

---

## 12. Navigation (`src/navigation/`)

Public functions (re-exported from `lib.rs`):

| Function | Behaviour |
|----------|-----------|
| `neighbors(engine, cx_id, slot, k)` | k-NN of a constellation within one lens (`lens_nav.rs`). |
| `compare_lenses(engine, query, slots)` | Per-lens search across slots → `Vec<LensComparison>`. |
| `define(engine, cx_id, slot, k)` | New `Constellation` averaging neighborhood vectors across slots. |
| `agree(engine, anchor, k, slot_filter)` | Consensus ranked by **min** per-lens cosine. |
| `disagree(engine, anchor, k, slot_filter)` | Consensus ranked by **spread** (max − min cosine). |
| `traverse(engine, anchor, direction, max_hops)` | Walk the engine's association graph. |
| `traverse_graph(graph, anchor, direction, max_hops)` | Walk an explicit `AssocGraph`. |
| `skills(engine, params)` | HDBSCAN* skill discovery → `SkillTree`. |
| `search_skill(engine, tree, skill, query)` | Search restricted to a skill's members. |

**Consensus** (`navigation/consensus.rs`): `ConsensusMode ∈ {Agree, Disagree}`. For each candidate, compute per-shared-lens `SlotCosine { slot, cosine }` (via `dense_cosine`), require ≥2 shared lenses (else added to `skipped_insufficient_overlap`; needs ≥2 dense lenses overall or `CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES`). `ConsensusHit { cx_id, rank, score, mean_cosine, min_cosine, max_cosine, spread, per_slot }`; score = `min` (Agree) or `max−min` (Disagree). `ConsensusReport { anchor, mode, slots, hits, skipped_insufficient_overlap }`.

**Traverse** (`navigation/traverse.rs`): `MAX_TRAVERSE_HOPS = 10`; `TraverseDirection ∈ {Forward, Backward, Both}`. `scored_walk` BFS with attenuation `0.9^hop` on path-weight products, keeping the best score per node; `TraverseStep { cx_id, hop, direction, score, via }`, `TraversePath { anchor, direction, max_hops, steps }`. Errors: `CALYX_SEXTANT_TRAVERSE_HOPS` (hops outside `1..=10`), `CALYX_SEXTANT_ASSOC_GRAPH_MISSING`, `CALYX_SEXTANT_CX_MISSING`.

**Skills** (`navigation/skills.rs`, `hdbscan.rs`): `SkillParams { min_cluster_size>=2, min_samples>=1, max_constellations, slots, allow_single_cluster }`. Pairwise fused distance = `1 − mean(per-shared-lens cosine)` clamped to `[0,2]`. Deterministic **HDBSCAN\*** pipeline (`hdbscan.rs`): core distances → mutual reachability → Prim MST (index tie-breaks) → single-linkage dendrogram → condense → excess-of-mass selection. `SkillNode { name (blake3 of sorted members), parent, children, members, size, depth, lambda_birth, stability, selected }`, `SkillTree { root, nodes, selected, noise, params }`. Param errors → `CALYX_SEXTANT_SKILL_PARAMS`; budget → `CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED`; unknown name → `CALYX_SEXTANT_SKILL_UNKNOWN`; no lens overlap → `CALYX_SEXTANT_SKILL_PAIR_NO_OVERLAP`.

---

## 13. Error taxonomy (`src/error.rs`)

`sextant_error(code, message)` builds a `CalyxError` with a remediation string. Codes (string value == constant name):

| Constant | Meaning (abridged remediation) |
|----------|-------------------------------|
| `CALYX_SEXTANT_PLAN_UNBOUNDED` | Plan cost unbounded; tighten k/ef/slots. |
| `CALYX_SEXTANT_PLAN_COST_EXCEEDED` | Plan exceeds cost budget. |
| `CALYX_PLANNER_COST_CAP` | Cross-model scope exceeds cost cap. |
| `CALYX_SEXTANT_RERANKER_TIMEOUT` | Reranker connect/read/write timeout. |
| `CALYX_SEXTANT_RERANKER_ENDPOINT` | Reranker endpoint invalid/unresolvable. |
| `CALYX_SEXTANT_RERANKER_PROTOCOL` | Reranker response malformed / wrong count. |
| `CALYX_SEXTANT_RERANKER_NO_CANDIDATES` | Rerank request had no candidates. |
| `CALYX_SEXTANT_NO_LENSES` | No slot indexes registered. |
| `CALYX_SEXTANT_SLOT_ALREADY_REGISTERED` | Duplicate SlotId. |
| `CALYX_SEXTANT_SLOT_MISSING` | Slot not registered. |
| `CALYX_SEXTANT_SLOT_INACTIVE` | Slot parked/inactive. |
| `CALYX_SEXTANT_INDEX_EMPTY` | Index has no documents. |
| `CALYX_SEXTANT_EF_TOO_SMALL` | HNSW ef < requested results. |
| `CALYX_SEXTANT_DIM_MISMATCH` | Query vector dim mismatch. |
| `CALYX_SEXTANT_VECTOR_SHAPE` | Wrong vector shape for index. |
| `CALYX_SEXTANT_QUERY_SHAPE` | Invalid query structure. |
| `CALYX_INVALID_ARGUMENT` | Argument validation failed. |
| `CALYX_ANSWER_UNGROUNDED` | ASK answer lacks grounding. |
| `CALYX_LENS_NOT_FOUND` | Lens slot not found. |
| `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE` | No wired Forge GPU path. |
| `CALYX_SEXTANT_POSTINGS_CORRUPT` | Sparse postings block corrupt. |
| `CALYX_SEXTANT_POSTINGS_NOT_SORTED` | Postings not sorted by doc id. |
| `CALYX_SEXTANT_PROVENANCE_MISSING` | Stored constellation missing for hit. |
| `CALYX_SEXTANT_RECURRENCE_READ_ERROR` | Recurrence scalar corrupt. |
| `CALYX_SEXTANT_CX_MISSING` | Constellation not indexed. |
| `CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES` | <2 dense lenses for consensus. |
| `CALYX_SEXTANT_ASSOC_GRAPH_MISSING` | Association graph not set. |
| `CALYX_SEXTANT_TRAVERSE_HOPS` | Hops outside 1..=10. |
| `CALYX_SEXTANT_SKILL_UNKNOWN` | Unknown skill name. |
| `CALYX_SEXTANT_SKILL_PARAMS` | Invalid skill params. |
| `CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED` | Skill clustering budget exceeded. |
| `CALYX_SEXTANT_SKILL_PAIR_NO_OVERLAP` | Clustered pair shares no lens. |
| `CALYX_TEMPORAL_WINDOW_BUDGET_EXHAUSTED` | Window candidate budget exhausted. |
| `CALYX_INDEX_CORRUPT` | On-disk DiskANN graph corrupt. |
| `CALYX_INDEX_IO` | Index I/O error. |
| `CALYX_INDEX_DIM_MISMATCH` | DiskANN query dim mismatch. |
| `CALYX_INDEX_INVALID_PARAMS` | Invalid DiskANN params. |
| `CALYX_TEMPORAL_AP60_VIOLATION` | Temporal signal used in primary retrieval. |
| `CALYX_TEMPORAL_INVALID_BOOST_CONFIG` | Invalid temporal boost config. |
| `CALYX_TEMPORAL_INVALID_PERIOD` | Invalid hour/day period. |
| `CALYX_TEMPORAL_INVALID_WINDOW` | Invalid time window. |
| `CALYX_TEMPORAL_WEIGHT_SUM` | Temporal weights must sum to 1.0. |

---

## 14. Constants summary

| Constant | Value | Where |
|----------|-------|-------|
| HNSW M (`max_neighbors`) | 32 | `hnsw/mod.rs` |
| HNSW level cap | 6 | `hnsw/mod.rs::level_for` |
| `EXACT_CONSTRUCTION_ROWS` | 4096 | `hnsw/graph.rs` |
| `CONSTRUCTION_EF` | 64 | `hnsw/graph.rs` |
| `RECENT_CONSTRUCTION_SCAN` | 128 | `hnsw/graph.rs` |
| `SEARCH_VISIT_LIMIT_EF_MULTIPLIER` | 16 | `hnsw/graph.rs` |
| `RRF_K` | 60.0 | `fusion/rrf.rs` |
| BM25 `k1` / `b` | 1.2 / 0.75 | `index/bm25.rs` |
| `DISKANN_MAGIC` / version | `CLXDA001` / 1 | `diskann/graph.rs` |
| `DISKANN_BLOCK_ALIGN` | 4096 | `diskann/graph.rs` |
| `DISKANN_MAX_DIM` / `DISKANN_MAX_M` | 8192 / 512 | `diskann/graph.rs` |
| DiskANN `BUILD_SEED` | 42 | `diskann/build.rs` |
| DiskANN default search | beamwidth 32, ef_search 64, rescore_k 64 | `diskann/search/mod.rs` |
| DiskANN open `alpha` | 1.2 | `diskann/search/mod.rs` |
| `MAX_TRAVERSE_HOPS` | 10 | `navigation/traverse.rs` |
| traverse attenuation | `0.9^hop` | `navigation/traverse.rs` |
| `DEFAULT_COST_CAP_MS` | 30_000 | `query/planner.rs` |
| `PlanLimits` defaults | k 100, ef 512, slots 16, cost 20M, timeout 5000ms | `planner.rs` |
| admission defaults | concurrent 128, queued 512, timeout 250ms | `query_admission.rs` |
| Inverted slot shape | `Sparse(1_000_000)` | `index/inverted.rs` |

---

*All behaviour above is traced to `crates/calyx-sextant` source as of this writing. Items not appearing in source (e.g. GPU parity, incremental DiskANN edge insertion) are explicitly stubbed/rebuild-only in the code.*
