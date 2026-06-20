# 09 — Sextant Search & Navigation (`calyx-sextant`)

**Source files covered:**

- `crates/calyx-sextant/src/lib.rs`
- `crates/calyx-sextant/src/error.rs`
- `crates/calyx-sextant/src/util.rs`
- `crates/calyx-sextant/src/hit.rs`
- `crates/calyx-sextant/src/search.rs`
- `crates/calyx-sextant/src/slot_index_map.rs`
- `crates/calyx-sextant/src/query_admission.rs`
- `crates/calyx-sextant/src/reranker.rs`
- `crates/calyx-sextant/src/guarded.rs`
- `crates/calyx-sextant/src/planner.rs`, `planner_explain.rs`
- `crates/calyx-sextant/src/index/mod.rs`
- `crates/calyx-sextant/src/index/hnsw/mod.rs`, `index/hnsw/graph.rs`, `index/hnsw/scored.rs`
- `crates/calyx-sextant/src/index/dual.rs`
- `crates/calyx-sextant/src/index/bm25.rs`, `index/inverted.rs`, `index/tokenizer.rs`
- `crates/calyx-sextant/src/index/multi.rs`
- `crates/calyx-sextant/src/index/quant_config.rs`
- `crates/calyx-sextant/src/index/diskann/{mod,build,graph,dual,concat,token,token_sidecar}.rs`
- `crates/calyx-sextant/src/index/diskann/search/{mod,helpers}.rs`
- `crates/calyx-sextant/src/index/spann/{mod,centroids,posting}.rs`
- `crates/calyx-sextant/src/index/funnel.rs`
- `crates/calyx-sextant/src/fusion/{mod,rrf,profiles,pipeline,single}.rs`
- `crates/calyx-sextant/src/query/{mod,search}.rs`
- `crates/calyx-sextant/src/navigation/{mod,consensus,traverse}.rs`
- `crates/calyx-sextant/src/temporal/{mod,boost}.rs`
- `crates/calyx-sextant/Cargo.toml`
- cross-checked against `docs/dbprdplans/10_SEXTANT_SEARCH_NAV.md`

Sextant is the Calyx retrieval/navigation crate: per-slot vector and keyword indexes (in-RAM and on-disk), multi-signal fusion with a deterministic query planner, cross-lens navigation, post-retrieval temporal boosts, optional cross-encoder reranking, and Ward-backed guarded search. Dependencies: `calyx-aster`, `calyx-core`, `calyx-paths`, `calyx-ward`, `blake3`, `memmap2`, `rand`/`rand_chacha`, `rayon`, `serde`, `zstd`, `zeroize` (`Cargo.toml`). See [05_core.md](05_core.md) for `CxId`/`SlotId`/`SlotVector`, [13_ward_guard.md](13_ward_guard.md) for guard, [10_loom_associations.md](10_loom_associations.md) for the assoc graph.

---

## 1. The `SextantIndex` trait and registry

### 1.1 Trait (`index/mod.rs`)

All slot indexes implement `pub trait SextantIndex: Send + Sync`:

| Method | Signature | Notes |
|---|---|---|
| `slot` | `fn slot(&self) -> SlotId` | |
| `shape` | `fn shape(&self) -> SlotShape` | `Dense(dim)`, `Sparse(dim)`, `Multi{token_dim}` |
| `insert` | `fn insert(&mut self, cx_id, vector: SlotVector, seq: u64) -> Result<()>` | |
| `search` | `fn search(&self, query: &SlotVector, k, ef: Option<usize>) -> Result<Vec<IndexSearchHit>>` | |
| `rebuild` | `fn rebuild(&mut self) -> Result<()>` | |
| `vector` | `fn vector(&self, cx_id) -> Option<SlotVector>` | |
| `set_base_seq` | `fn set_base_seq(&mut self, seq: u64)` | freshness tracking |
| `stats` | `fn stats(&self) -> IndexStats` | |
| `insert_text` / `search_text` / `candidate_text` | default impls return `CALYX_SEXTANT_VECTOR_SHAPE` / `None` | only `InvertedIndex` overrides |

`IndexSearchHit { cx_id: CxId, score: f32, rank: usize }`. `IndexStats { slot, shape, len, built_at_seq, base_seq, kind: &'static str }`. The `kind` string is load-bearing — `search.rs` branches on `"inverted"` to use the text path. Helper `ranked(scored)` assigns 1-based ranks.

### 1.2 `SlotIndexMap` (`slot_index_map.rs`)

`Arc<RwLock<BTreeMap<SlotId, Arc<RwLock<Box<dyn SextantIndex>>>>>>` plus a parallel `SlotState` map. `register` fails with `CALYX_SEXTANT_SLOT_ALREADY_REGISTERED` on duplicate slot; `slots()` returns only `Active` slots; reads/writes through it fail with `CALYX_SEXTANT_SLOT_MISSING` / `CALYX_SEXTANT_SLOT_INACTIVE`. All search/insert paths `ensure_active` first.

`built_at_seq` vs `base_seq`: `base_seq` advances on every insert; `built_at_seq` is the seq the current graph was built at. `stale_by = base_seq - built_at_seq` drives freshness enforcement (`search.rs::enforce_freshness`).

---

## 2. Vector index: in-RAM HNSW (`index/hnsw/`)

`HnswIndex` is the embedded dense ANN (the substrate for embedded vaults, and reused internally by SPANN and the funnel kernel ANN). It is deterministic (no float-RNG in topology; levels are hashed).

### 2.1 Structure and parameters

`HnswIndex` fields: `slot`, `dim: u32`, `seed: u64`, `max_neighbors: usize` (= **M**), `rows: Vec<Row>`, `positions: HashMap<CxId, usize>`, `fingerprints: HashMap<[u8;32], Vec<usize>>` (exact-vector dedup), `entry_point: Option<usize>`, `quant: QuantConfig`, `built_at_seq`, `base_seq`. `Row { cx_id, vector: Vec<f32>, seq, level: u8, neighbors: Vec<usize>, deleted: bool }`.

| Parameter | Default | Where |
|---|---|---|
| `max_neighbors` (M) | **32** | `HnswIndex::new` |
| `EXACT_CONSTRUCTION_ROWS` | 4096 | `hnsw/graph.rs` — first 4096 rows connect via exhaustive scan |
| `CONSTRUCTION_EF` | 64 | beam width during build |
| `RECENT_CONSTRUCTION_SCAN` | 128 | extra recent rows always considered as candidates |
| `SEARCH_VISIT_LIMIT_EF_MULTIPLIER` | 16 | `max_visits = ef*16` cap at search |
| `CONSTRUCTION_VISIT_LIMIT_EF_MULTIPLIER` | 1 | |
| search `ef` default | `max(k, M*2)`, capped to `rows.len()` | `hnsw/mod.rs::search` |

Level assignment (`level_for`): `blake3(cx_id ‖ seed ‖ ordinal)`, take byte[0], `level = trailing_zeros().min(6)` — deterministic 0..=6 layer geometry (not the classic exponential RNG, but the same skip-list shape).

`QuantConfig` (`quant_config.rs`): `kind: None | Scalar8 | Binary`, `scale`, `zero_point`, `locked`. `Scalar8` quantizes `q = round(v/scale).clamp(-127,127) as i8`; `Binary` maps `v>=0 → 1 else -1`. Config is locked after first insert (`lock_after_first_insert`). The quantizer is implemented but the HNSW search path scores on raw `f32` cosine; there is **no wired GPU parity** — `cpu_gpu_delta` returns `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE` (also true for `MaxSimIndex`).

### 2.2 Build path (`connect_new_row`)

1. For row index `i`:
   - if `i <= EXACT_CONSTRUCTION_ROWS`: candidate set = exhaustive cosine over `rows[..i]` (top-M per level).
   - else: candidate set = `beam_search` (ef=`CONSTRUCTION_EF`, capped) from the greedy-descent start, **plus** the last `RECENT_CONSTRUCTION_SCAN` rows, plus stride neighbors (`i-1, i-2, i-4, i-8, …` powers of two).
2. Select top-M by cosine for layer 0; repeat top-M for each layer `1..=row.level`.
3. `prune_neighbors`: `diversified_neighbors` (M-cap, diversity prune) on the merged candidate list.
4. Add back-edges into each chosen neighbor and re-prune that neighbor.
5. `refresh_entry_after_insert`: entry point is whichever row has the highest `level`.

Re-inserting an existing `cx_id` replaces the vector and triggers a full `rebuild()` (clears all neighbor lists, reconnects every row in order).

### 2.3 Query path (`search`)

1. Fail closed if index empty (`CALYX_SEXTANT_INDEX_EMPTY`), `k==0` or `ef<k` (`CALYX_SEXTANT_EF_TOO_SMALL`), dim mismatch (`CALYX_SEXTANT_DIM_MISMATCH`).
2. `greedy_descent` from entry point down levels `max..1`: at each level hill-climb to the best-cosine neighbor with `level >= current level`.
3. `beam_search` at layer 0 with `effective_ef = ef + tombstone_count` (capped to `rows.len()`); a max-heap of candidates, keeping the `ef` best; stops when the heap's best cannot beat the current worst kept, or `visited >= ef*16`.
4. Merge beam results with `exact_vector_hits` (fingerprint-exact matches — guarantees a stored identical vector is always recallable), keeping max score per `cx_id`.
5. `top_k` and truncate to `k`. Scoring metric is **cosine** (`util::cosine`).

**Tombstones / GC:** `mark_deleted` flips `deleted`; `purge_tombstones`/`clone_without_tombstones` rebuild without dead rows. Implements `calyx_aster::gc::AnnIndexGraph` (`ann_tombstone_stats`, `rebuild_without_tombstones`) for the vault GC path (test `ph58_ann_gc_fsv.rs`). `recall_at(queries,k,ef)` measures recall vs `brute_force`.

Complexity: build is roughly `O(n·ef·M)` after the exhaustive prefix; query is `O(ef·M)` node visits, each an `O(dim)` cosine.

### 2.4 `DualIndex` (`index/dual.rs`)

Two `HnswIndex` (`a`, `b`; b seeded `seed ^ 0x9e37`) for asymmetric slots, with `boost_a_to_b` / `boost_b_to_a` (default 1.0). `insert_side`/`search_side` (multiplies hit scores by the side boost); the trait `search` returns side A. `stats().kind = "dual"`.

---

## 3. Vector index: on-disk DiskANN / Vamana (`index/diskann/`)

Server-scale dense ANN: an NVMe-resident Vamana graph, mmap-read, with raw-f32 rescore. Module header notes it is server-only; embedded vaults keep in-RAM HNSW.

### 3.1 On-disk format (`diskann/graph.rs`)

| Constant | Value |
|---|---|
| `DISKANN_MAGIC` | `b"CLXDA001"` |
| `DISKANN_FORMAT_VERSION` | 1 |
| `DISKANN_BLOCK_ALIGN` | 4096 (page) |
| `DISKANN_MAX_DIM` | 8192 |
| `DISKANN_MAX_M` | 512 |

Layout: one 4 KiB header block, then one page-aligned node block each. `node_block_size = ceil(dim*4 + 4 + m_max*4, 4096)`. Node block: `[raw f32 vector | neighbor_count: u32 LE | neighbors: [u32 LE]]` zero-padded — one read fetches a node's full search state. Node `id` is at `HEADER + id*block`. Header fields (all LE): `format_version, dim, m_max, max_degree, entry_point_id, node_count: u64`.

`DiskAnnHeader` { `format_version, dim, m_max, max_degree, entry_point_id, node_count` }. Writer (`DiskAnnGraphWriter`) stages to `<path>.tmp`, fsyncs, atomically renames (crash-safe; `Drop` unlinks tmp). Reader (`DiskAnnGraphReader`) is a read-only `Mmap`; `read_node` does a zero-copy `cast_le_slice` (fails closed `CALYX_INDEX_CORRUPT` on misalignment). All shape/degree/file-length invariants are validated on decode.

### 3.2 Build path — Vamana (`diskann/build.rs`)

`DiskAnnBuildParams { dim, m_max, ef_construction, alpha: f32 }`. Validation: `dim ∈ 1..=8192`, `m_max ∈ 1..=512`, `ef_construction >= 1`, `alpha ∈ 1.0..=4.0`. IDs must be dense `0..n`. (`DiskAnnSearch::open` uses a fixed build `alpha = 1.2` for later rebuilds.)

Algorithm (`vamana`), two-pass per the DiskANN paper, parallel-deterministic:
1. **Normalize** every vector to unit L2 (`normalize`); distance kernel = `1 - dot` (= `1 - cosine`). Graph file stores **original** (un-normalized) vectors.
2. **Entry** = medoid (point nearest the dataset centroid).
3. **Init**: each node gets `m_max` random edges (seeded `ChaCha8Rng`, `BUILD_SEED = 42`).
4. **Two passes**, `alpha ∈ [1.0, params.alpha]`. Within a pass, advance in **prefix-doubling batches** (`BUILD_BATCH_MIN = 256`, growing ×2 up to `n/BUILD_BATCH_DIVISOR=n/32`):
   - Each node in the batch `greedy_search`es the **frozen** adjacency snapshot in parallel (`ef = max(ef_construction, m_max)`), unions in its current neighbors, then `robust_prune(p, candidates, alpha, m_max)`.
   - Forward edges assigned sequentially (batch order); back-edges grouped by target (`BTreeMap`, deterministic), each affected node re-pruned once, in parallel.
5. **RobustPrune**: sort candidates by distance to `p`; greedily keep the nearest, drop any candidate `c` where `alpha · dist(star, c) > dist(p, c)`; stop at `r = m_max`.

Build is parallel (`rayon`) yet fully deterministic regardless of thread count (test `ph68_parallel_build_fsv.rs`).

### 3.3 Query path (`diskann/search/mod.rs`)

`DiskAnnSearchParams { beamwidth, ef_search, rescore_k, rescore_from_raw }`, defaults **beamwidth 32, ef_search 64, rescore_k 64, rescore_from_raw true**. Validated `>0` each.

`search_ids(query, k, params)`:
1. Validate dim/finiteness; require `ef_search >= want` (`want=min(k,n)`).
2. Beam search from `entry_point_id`: maintain `candidates` sorted by distance; expand the nearest un-expanded node; `prefetch` the top-`beamwidth` blocks (`posix_fadvise WILLNEED` on Unix; no-op on Windows — see Gaps); score every neighbor (`distance = max(0, 1-cosine)`); truncate candidate list to `max(ef_search, rescore_k)`; stop via `stop_search` (no un-expanded candidate can beat the `rescore_k`-th best) or when `expanded >= min(ef_search, n)`.
3. Take `rescore_k` best by graph distance.
4. If `rescore_from_raw` and a `raw_sidecar` dir exists: re-score each candidate against the raw f32 vector read from the sidecar (`<id>` / `<id>.raw` / `<id:08>.raw` / `<cx_id>` / `<cx_id>.raw`), then re-sort.
5. Truncate to `want`. The `SextantIndex::search` wrapper converts to `score = 1.0 - dist`.

`insert`/`rebuild` re-materialize all vectors from the graph and rebuild the file (DiskANN is not incrementally mutable in place). Complexity: query is `O(ef_search · m_max)` block reads plus `rescore_k` raw reads; build is the dominant cost (two parallel Vamana passes).

### 3.4 Dual DiskANN (`diskann/dual.rs`)

`DualDiskAnnSearch` holds forward (`asym_a`) + reverse (`asym_b`) graphs at `<vault>/idx/slot_NN.asym_{a,b}/graph.cda`. `Direction { Forward, Reverse }`. `DirectionalBoost { forward_weight, reverse_weight }` (default 0.5/0.5; `new` validates finite, non-negative, **sum == 1.0 ± 1e-6**). `search_merged` runs both directions and merges by `max(score·weight)` per id; `search_directional` does one side. Fails `CALYX_INDEX_DIRECTION_UNAVAILABLE` if a direction's graph is missing; `degraded` flag set if side B insert fails. `cx_from_local` synthesizes CxIds from dense local ids. `stats().kind = "DualDiskANN"`. (Test `diskann_dual.rs`.)

### 3.5 Concat cross-term DiskANN (`diskann/concat.rs`)

`ConcatCrossTermDiskAnn` builds a DiskANN graph over materialized `xterm` vectors with a `keys.cdx` sidecar (`CLXXTRM1` v1) mapping each node to `ConcatCrossTermKey { cx_id, a: SlotId, b: SlotId }`. `search_terms` returns `ConcatCrossTermHit { key, distance }`.

### 3.6 Token DiskANN + segmented MaxSim (`diskann/token.rs`)

`TokenDiskAnnMaxSim` is the **server-scale late-interaction** index: every doc's token vectors are flattened into one DiskANN graph; sidecars (`token_sidecar.rs`) store `DocSegment { cx_id, start, len }`, a `token→doc` map, and an mmap'd raw-token blob. Query (`search_tokens`): for each query token, DiskANN-retrieve `candidate_tokens_per_query` nearest token-ids → collect their docs → for each candidate doc, read its token segment and compute full `MaxSimIndex::maxsim(query, doc_tokens)` → `top_k`. `stats().kind = "token_diskann_maxsim"`.

---

## 4. Vector index: SPANN (`index/spann/`)

Sparse/dense slot index with **centroid ANN in RAM, posting lists on disk** (SPANN paper).

### 4.1 Centroids (`spann/centroids.rs`)

`SpannCentroidIndex` persisted as `centroids.spn` (`SPANN_CENTROID_MAGIC = b"CLXSP001"`, version 1). Fields: `dim`, `centroids: Vec<Vec<f32>>`, `posting_list_offsets`, `assignments: Vec<(vec_id, centroid_id)>`, an internal `HnswIndex` over the centroids, and `centroid_lookup`.

Build (`try_build_centroids`): **k-means** with `KMEANS_ITERS = 12` Lloyd iterations, k-means++ seeding (`kmeans_pp`, seeded `ChaCha8Rng`). Default cluster count = `floor(sqrt(n)).max(1)` (`default_cluster_count`), capped to `n`; empty clusters reseed to the farthest vector. Assignment/centroid distance is **squared L2** (`l2_sq`). `nearest_centroids(query, n_probe)` queries the RAM HNSW (`ef = max(n_probe, 64)`). On-disk format is fully length/magic-validated.

### 4.2 Posting lists (`spann/posting.rs`)

One file per centroid: `pl_NNNN.spb`, **zstd level 3** over a raw block. Block format: `count: u32 LE`, then per entry `varint(cx_id delta)` + `score: f32 LE`. cx_ids must be **strictly ascending** (delta-encoded); decode fails closed `CALYX_INDEX_CORRUPT` on overflow / non-finite score / trailing bytes. Writer stages to `.tmp`, fsyncs, renames.

`SpannSearch` (`default_n_probe = 8`): `insert` assigns the dense form of a sparse vector to its nearest centroid (`assign`) and appends `(local_id, score)` (score = sum of sparse entry values) to that centroid's list. `search(query, k, n_probe)`: for each of the `n_probe` nearest centroids, read its posting list, keep `max(score)` per cx_id, sort desc, truncate `k`. Requires sparse query vectors (`dense_sparse` densifies for centroid assignment). `stats().kind = "SPANN"`. (Test `spann.rs`.)

---

## 5. Kernel-first funnel search (`index/funnel.rs`)

3-hop sublinear search for huge vaults ("kernel ANN → expand by association → search within regions").

`FUNNEL_MIN_VAULT_SIZE = 10_000_000` — `search` fails `CALYX_INDEX_FUNNEL_VAULT_TOO_SMALL` below this (small vaults route to direct HNSW/DiskANN).

`FunnelParams` defaults: `n_kernel_probe = 8`, `n_region_beam = 32`, `n_cx_beam = 64`, `n_regions_to_expand = 4` (all must be `>0`).

Steps (`KernelFirstSearch::search`):
1. **Probe kernel** (`probe_kernel`): query the in-RAM `KernelRegionAnn` (an `HnswIndex` over kernel-region vectors) for the `n_kernel_probe` nearest kernel regions. Empty kernel → `CALYX_INDEX_KERNEL_UNAVAILABLE`.
2. **Expand regions** (`expand_regions`): DiskANN search the `region_ann` graph (beamwidth=`n_region_beam`) for up to `n_regions_to_expand` `RegionCandidate { kernel_region, region, score=1-dist }`.
3. **Search within regions** (`search_within_regions`): run the final cx search (`FinalCxSearch::DiskAnn` or `::Spann`, `n_cx_beam`), drop any hit whose `RegionPartitions` assignment is not in the expanded region set, keep `max(score)` per cx, sort desc, truncate `k`. Returns `FunnelHit { cx_id, score, path: FunnelPath { kernel_region, region, cx } }`.

(Test `funnel.rs`.)

---

## 6. BM25 keyword index (`index/inverted.rs`, `index/bm25.rs`)

### 6.1 Tokenizer (`index/tokenizer.rs`)

`tokenize`: lowercases (`char::to_lowercase`), splits on any non-`alphanumeric` char, no stemming/stopwords. Deterministic. The module also provides varint delta codec for posting ids (`encode/decode_varint_deltas`, `CALYX_SEXTANT_POSTINGS_NOT_SORTED` / `CALYX_SEXTANT_POSTINGS_CORRUPT`).

### 6.2 Posting/index layout

`InvertedIndex` (in-memory): `docs: BTreeMap<CxId,String>`, `vectors`, `postings: BTreeMap<term, Vec<Posting{cx_id, tf}>>`, `doc_len: BTreeMap<CxId, usize>`, `scorer: Bm25`. Postings store per-doc term frequency. A sparse `SlotVector` is indexed by synthesizing text `"t<idx> t<idx> …"` from entry indices; `insert_text`/`search_text` index real text. `shape()` = `Sparse(1_000_000)`. `stats().kind = "inverted"`.

### 6.3 Scoring formula (`index/bm25.rs`)

**`Bm25 { k1, b }` with Lucene-like defaults `k1 = 1.2`, `b = 0.75`.**

```
idf(N, df)        = ln( (N - df + 0.5) / (df + 0.5) + 1.0 )      // Lucene BM25, +1 inside ln
len_norm          = doc_len / avg_doc_len                         // 1.0 if avg<=0
score_term        = idf · ( tf·(k1+1) ) / ( tf + k1·(1 - b + b·len_norm) )
```

`tf==0 || N==0 || df==0 → 0.0`. `search_text` tokenizes the query into a unique term set, sums `score_term` per matching doc (avg_doc_len computed live), then `top_k`.

---

## 7. Late-interaction / multi-vector (`index/multi.rs`)

`MaxSimIndex` — ColBERT-style MaxSim over per-token vectors. `rows: Vec<(CxId, Vec<Vec<f32>>, seq)>`, `token_dim`. Shape `Multi { token_dim }`. `stats().kind = "multi_maxsim"`.

**MaxSim formula** (`MaxSimIndex::maxsim`):
```
maxsim(Q, D) = Σ_{q∈Q}  max_{d∈D}  cosine(q, d)
```
i.e. for each query token, take its single best-matching doc token (cosine), and sum over query tokens. `search` brute-forces MaxSim over all rows and `top_k`s. The server-scale variant (token DiskANN, §3.6) reuses this exact `maxsim`. `cpu_gpu_delta` is unimplemented (`CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE`).

---

## 8. Fusion (`fusion/`)

### 8.1 Strategy enum (`fusion/mod.rs`)

`FusionStrategy = SingleLens { slot } | Rrf | WeightedRrf { profile: RrfProfile } | Pipeline`. `fuse(results, context)` dispatches. `FusionContext { k, explain, strategy, weights: BTreeMap<SlotId,f32>, stage1_slots: Vec<SlotId> }`.

### 8.2 RRF (`fusion/rrf.rs`)

**Reciprocal Rank Fusion, constant `RRF_K = 60.0`:**
```
contribution(weight, rank) = weight / (rank + 60)
fused_score(cx) = Σ_slots  weight_slot / (rank_{slot}(cx) + 60)
```
- `rrf_fuse`: all participating slots weight 1.0.
- `weighted_rrf_fuse`: per-slot weights from `context.weights`, default weight 0.0 (slots not in the profile do not contribute); slots with `weight<=0` skipped.
- Ties broken by `cx_id` string order; truncated to `context.k`. Each hit records `PerLensContribution { slot, rank, raw_score, weight, contribution }`.

### 8.3 Weighted profiles (`fusion/profiles.rs`)

`RrfProfile` (14 variants): `Causal, Code, Entity, Temporal, Speaker, Style, Civic, Media, Bridge, Kernel, Semantic, Lexical, Multimodal, General`. `weighted_profiles()` ships each as `WeightedProfile { profile, weights, lexical_excludes_dense }`. For each profile a slot list is given; **weight for the i-th slot (0-based) = `1/(i+1)`** (rank-decayed). Example slot lists: `Causal → [4,8,18]`, `Code → [8,9,10,11,16]`, `Temporal → [8]`, `Lexical → [1]` (with `lexical_excludes_dense = true`), `General → [1,8,18]`, `Multimodal → [8,9,10,11]`. AP-60 temporal primary slots `[20,21,22]` are excluded from primary retrieval (`is_ap60_temporal_primary_slot`).

### 8.4 Pipeline (`fusion/pipeline.rs`)

Two-stage: stage-1 candidate set = union of cx_ids from `stage1_slots` (the sparse/inverted lenses); then `rrf_fuse_restricted` over the **non-stage-1** slots, restricted to that candidate set (falls back to all slots restricted to candidates if no scoring slots/empty). `summarize_pipeline` reports `subset_ok` (final ⊆ stage1). This is the recall→score path that an optional ColBERT/cross-encoder rerank finishes (§10).

### 8.5 Single lens (`fusion/single.rs`)

Passes one slot's hits through unchanged (rank/score preserved), capped to `k`.

---

## 9. Query planner (`planner.rs`)

### 9.1 Intent classification (`classify`)

Deterministic keyword classifier over lowercased `query.text`, checked in order → `IntentLabel` (same 14 labels as `RrfProfile`). Examples: `"fn "/"rust"/"trait"/"compile"/"stacktrace"/"function" → Code`; `"because"/"caused"/"why"/"causal"/"leads to" → Causal`; `"who"/"entity"/"person"/… → Entity`; `"when"/"recent"/"recurring"/… → Temporal`; `"exact"/"keyword"/"bm25" → Lexical`; otherwise `General`.

### 9.2 Strategy selection (`strategy_for`)

`Code → SingleLens{ first slot or SlotId 8 }`; `General → Rrf`; `Lexical/Causal/Temporal` and all others → `WeightedRrf` with the matching `RrfProfile` (`profile_for`).

### 9.3 Bounds, cost, plan (`plan`)

`PlanLimits` defaults: `max_k = 100`, `max_ef = 512`, `max_slots = 16`, `max_cost = 20_000_000`, `timeout_ms = 5_000`.

`plan(query, index_size)`:
1. classify intent; if `query.fusion` is set use it (`override_used = true`) else pick via `strategy_for`.
2. `enforce_bounds`: `k>0` and `<=max_k` (`CALYX_SEXTANT_PLAN_UNBOUNDED`); no lenses + empty index → `CALYX_SEXTANT_NO_LENSES`; `ef` (default 64) `>0` and `<=max_ef`; `slots.len() <= max_slots`.
3. `estimate_cost = slots · ef · k · index_size / 100` (saturating).
4. `enforce_cost`: `> max_cost → CALYX_SEXTANT_PLAN_COST_EXCEEDED`.
5. Returns `PlannedQuery { query (fusion now set), intent, strategy, override_used, cost_estimate, timeout_ms }`. `PlannerExplain` (`planner_explain.rs`) wraps a plan + hits for explain output.

---

## 10. Search engine & reranking (`search.rs`, `reranker.rs`, `query_admission.rs`)

### 10.1 `SearchEngine::search_inner`

1. Acquire a query-admission permit; `query.validate()`.
2. Slots = `query.slots` or all active slots; empty → `CALYX_SEXTANT_NO_LENSES`.
3. `enforce_freshness` per slot against `query.freshness` (`FreshDerived` rejects any staleness; `StaleOk { seq_lag }` rejects `stale_by > seq_lag`) → `CalyxError::stale_derived`.
4. Strategy = `query.fusion` or `default_strategy(slots)`. **Reranker requires `Pipeline`** else `CALYX_SEXTANT_QUERY_SHAPE`.
5. `candidate_window`: for Pipeline / guarded queries, recall `recall_k` or `k·10` (`DEFAULT_PIPELINE_RECALL_MULTIPLIER`); for filtered queries, up to slot len; else `k`.
6. Per slot: `"inverted"` → `search_text`, else build the slot's query vector and `search`. `fuse` → apply scalar/anchor/metadata `filters` → optional rerank → `apply_query_guard` → truncate to `k` → renumber → attach provenance + freshness.

`Query` (`query/search.rs`): `text, vector: Option<SlotVector>, guard_vectors, slots, k (default 10), ef (default Some(64)), recall_k, explain, require_stored_provenance, freshness, fusion, filters, guard`. `validate()` rejects `k==0`, `ef==0`, `recall_k==0`, duplicate slots, non-finite/empty/absent vectors, malformed filters, out-of-range guard tau. Filters: `ScalarPredicate` (Eq/Gt/Gte/Lt/Lte), `AnchorPredicate`, `MetadataPredicate` (Vault/Modality/PanelVersion/CreatedAt/InputRedacted/InputPointerContains).

### 10.2 Query admission (`query_admission.rs`)

`QueryAdmissionConfig` defaults: `max_concurrent = 128`, `max_queued = 512`, `queue_timeout = 250 ms`. Returns a permit or `CalyxError::backpressure`; tracks in-flight/queued/rejection counters (`QueryAdmissionStats`).

### 10.3 Reranker (`reranker.rs`)

`RerankerClient { endpoint, timeout }` POSTs to an `http://` cross-encoder (`:8089` TEI) over a raw `TcpStream`. Candidate text is **request-scoped and never persisted**: `RerankCandidateText` wraps `Zeroizing<String>`, Debug-redacted, no `Serialize`/`Deserialize`/`Display` (enforced by `compile_fail` doctests; FSV tests `reranker_*nonpersistence_fsv.rs`). Errors: `CALYX_SEXTANT_RERANKER_{ENDPOINT,TIMEOUT,PROTOCOL,NO_CANDIDATES}`. Parses both `{"scores":[…]}` and TEI rank-array `[{index,score}]` responses; rejects mismatched counts / non-finite / non-2xx. In `search.rs`, pipeline hits are re-scored and re-ranked by the returned scores (strategy label becomes `"pipeline+rerank"`). ColBERT MaxSim (§7) is the in-engine late-interaction reranker for `Pipeline`.

---

## 11. Hit / provenance types (`hit.rs`)

`Hit { cx_id, score, rank, event_time_secs, temporal_scores, causal_confidence, causal_gate, per_lens: Vec<PerLensContribution>, cross_terms_used, guard: Option<HitGuardEvidence>, provenance: LedgerRef, provenance_source: Stored|Stub, freshness: FreshnessTag, explain: Option<ExplainBreakdown> }`.

`FreshnessTag { built_at_seq, base_seq, stale_by, policy }` (`fresh`/`stale_ok`). `ExplainBreakdown { strategy, per_lens_count, provenance_hex, recurrence_boost, guard_dropped }`. `HitGuardEvidence { mode: InRegionOnly, verdict: GuardVerdict }`, `DroppedGuardHit { cx_id, mode, reason, verdict }`. Provenance is a real `LedgerRef` when the stored constellation is present, else a deterministic `stub_ledger` blake3 stub (`util.rs`).

---

## 12. Navigation (`navigation/`)

`pub use`d helpers: `neighbors`/`define`/`compare_lenses` (`lens_nav.rs`), `agree`/`disagree` (`consensus.rs`), `traverse`/`traverse_graph` (`traverse.rs`), `skills`/`search_skill`/`define` skill tree (`skills.rs` + HDBSCAN `hdbscan.rs`).

- **Cross-lens consensus** (`consensus.rs`): per-slot cosine of each candidate vs the anchor; `Agree` scores by **min** per-slot cosine (all lenses concur), `Disagree` by **max − min spread** (anomaly). Needs `MIN_CONSENSUS_LENSES = 2` active dense lenses else `CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES`. `ConsensusHit { cx_id, rank, score, mean_cosine, min_cosine, max_cosine, spread, per_slot }`.
- **Asymmetric traversal** (`traverse.rs`): walks the vault `AssocGraph` (`calyx_paths`). `Forward`/`Backward`/`Both`; score = best path-weight product **attenuated by `0.9^hop`** (`calyx_paths::attenuate`). `MAX_TRAVERSE_HOPS = 10`; out-of-range → `CALYX_SEXTANT_TRAVERSE_HOPS`; missing anchor → `CALYX_SEXTANT_CX_MISSING`; no graph → `CALYX_SEXTANT_ASSOC_GRAPH_MISSING`. `TraverseStep { cx_id, hop, direction, score, via }`.
- **Skills** (`skills.rs`): HDBSCAN clustering of constellations into a named `SkillTree`; `SkillParams` (`min_cluster_size>=2`, `min_samples>=1` else `CALYX_SEXTANT_SKILL_PARAMS`); skill budget via `CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED`.

---

## 13. Temporal post-retrieval boosts (`temporal/`)

AP-60: temporal signals are **post-retrieval only, never during ANN, never dominant**. Three scores in `[0,1]` (`boost.rs`):

- **E2 recency** `score_e2_recency(event, query, decay)`: `Linear { max_age }` → `1 - age/max`; `Exponential { half_life }` → `exp(-age·0.693/half_life)`; `Step` → 0.8 (<1h) / 0.5 (<1d) / 0.1.
- **E3 periodic** `score_e3_periodic`: +0.5 if local hour matches target, +0.5 if local day-of-week matches (tz-aware via `temporal_time_bucket`).
- **E4 sequence** `score_e4_sequence(rank, total)`: `1 - rank/total`.

`fuse_temporal(scores, weights) = clamp(w.recency·E2 + w.sequence·E4 + w.periodic·E3, 0, 1)` (default `FusionWeights` = 50/35/15 recency/sequence/periodic, defined in `calyx-core`; plan doc §6). `apply_temporal_boost` multiplies each hit: `score += score·(fuse_temporal·post_retrieval_alpha + recurrence_total)`, then re-sorts. `TemporalPolicy.validate()` enforces AP-60 (weights sum to 1.0, alpha bounded) → `CALYX_TEMPORAL_*`. **Causal gate** (`causal_gate.rs`): high-confidence causal hits ×1.10, low-confidence ×0.85 (`causal_gate_mult`; `CausalConfidence`). Recurrence boost (`recurrence_boost.rs`) reads vault recurrence rows. Window recall budget in `recall_budget.rs`/`window.rs` (`CALYX_TEMPORAL_WINDOW_BUDGET_EXHAUSTED`).

---

## 14. Guarded search (`guarded.rs`)

`apply_query_guard`: when `query.guard = InRegionOnly(profile)`, every fused hit is checked via Ward (`guard_non_high_stakes`) using the hit constellation's required-slot vectors (`MatchedSlots`) against the produced query/guard vectors (`ProducedSlots`). Passing hits get `HitGuardEvidence`; failing hits move to `dropped_guard_hits` with a reason (`"ood"`, `"missing_constellation"`, `"missing_hit_slot:N"`, `"ward_error:…"`). **Multi-slot `InRegionOnly` is fail-closed without slot-aware `guard_vectors`** (`CALYX_SEXTANT_VECTOR_SHAPE`); legacy single top-level vector accepted only for single-slot profiles. Returned in `GuardedSearchReport { hits, dropped_guard_hits }`. See [13_ward_guard.md](13_ward_guard.md).

---

## 15. Universal / cross-model query (`query/`)

`UniversalQuery` is the single query surface over every collection mode (relational, document, kv, timeseries, graph_hop, vector, aggregate, ask). `plan` (`query/planner.rs`, `DEFAULT_COST_CAP_MS`) compiles it to a `CrossModelPlan { steps: Vec<PlanStep>, estimated_cost_ms, explain }`; `execute` (`query/executor.rs`) runs it; `ask` (`query/ask.rs`) is grounded NL answering (`AskResult`, `DEFAULT_ASK_TOP_K = 10`). `PlanStep` variants: `RelationalScan, DocScan, KvGet, TsRangeScan, GraphHop, VectorFusion, Aggregate, Ask`. Cost-cap breach → `CALYX_PLANNER_COST_CAP`. See [19_mcp_api_tools_reference.md](19_mcp_api_tools_reference.md) for the exposed surface.

---

## 16. Error taxonomy (`error.rs`)

All built via `sextant_error(code, message)` which attaches a fixed `remediation`. Roots (string constants, fail-closed): planner (`CALYX_SEXTANT_PLAN_UNBOUNDED`, `CALYX_SEXTANT_PLAN_COST_EXCEEDED`, `CALYX_PLANNER_COST_CAP`), reranker (`…RERANKER_{TIMEOUT,ENDPOINT,PROTOCOL,NO_CANDIDATES}`), slots/index (`…SLOT_{ALREADY_REGISTERED,MISSING,INACTIVE}`, `…INDEX_EMPTY`, `…EF_TOO_SMALL`, `…DIM_MISMATCH`, `…VECTOR_SHAPE`, `…QUERY_SHAPE`), lenses (`…NO_LENSES`, `CALYX_LENS_NOT_FOUND`), postings (`…POSTINGS_{CORRUPT,NOT_SORTED}`), navigation (`…CONSENSUS_INSUFFICIENT_LENSES`, `…ASSOC_GRAPH_MISSING`, `…TRAVERSE_HOPS`, `…CX_MISSING`, `…SKILL_*`), on-disk index (`CALYX_INDEX_{CORRUPT,IO,DIM_MISMATCH,INVALID_PARAMS,DIRECTION_UNAVAILABLE,FUNNEL_VAULT_TOO_SMALL,KERNEL_UNAVAILABLE}`), GPU parity (`…GPU_PARITY_UNAVAILABLE`), provenance (`…PROVENANCE_MISSING`, `…RECURRENCE_READ_ERROR`), temporal (`CALYX_TEMPORAL_*`, re-exported from `calyx-core`).

---

## Gaps / not covered

- **GPU parity is unimplemented**: `QuantConfig::cpu_gpu_delta` and `MaxSimIndex::cpu_gpu_delta` always return `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE`; the plan's "GPU-batched distance (Forge)" is not wired. HNSW scores on raw f32 cosine even when a quant config is set.
- **DiskANN prefetch** (`posix_fadvise WILLNEED`) is Unix-only; on Windows `prefetch_node` is a no-op (this doc was authored on Windows).
- **DiskANN / token-DiskANN inserts are not incremental**: `insert`/`rebuild` re-materialize all vectors and rebuild the whole graph file.
- The `FusionWeights` 50/35/15 defaults and causal-gate ×1.10/×0.85 multipliers are defined in `calyx-core` (re-exported here); exact default values were taken from the AP-60 plan doc §6 and `causal_gate.rs`, not from a literal in `temporal/boost.rs`.
- Planner cost model (`slots·ef·k·index_size/100`) is a heuristic, not a measured latency model.
- `temporal/`, `navigation/skills.rs`/`hdbscan.rs`, and the `query/executor`/`ask` internals are summarized at the public-surface level; deep per-function detail for those is deferred to their tests and the planning doc.
