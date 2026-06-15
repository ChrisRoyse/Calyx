# Stage 4 — Sextant Search & Navigation (PH23–PH26)

> **STATUS: ✅ DONE (FSV-signed-off, commit `9dc197c`).** `calyx-sextant`
> implements dense/sparse slot indexes, RRF/WeightedRRF/SingleLens fusion with
> provenance and freshness, planner/explain/navigation, tokenizer/varint/BM25,
> and real SciFact qrels evidence. FSV root:
> `/home/croyse/calyx/data/fsv-stage4-sextant-20260608003414`; final evidence
> hash `796b4812a3e2ac47a6ace81934be5799514d94f7e42b28b45b265386a98b6db8`.
> Later stage FSV has consumed Sextant successfully; current active frontier is
> tracked in `03_PHASE_MAP.md`.
> Post-sweep fail-closed hardening #282 adds duplicate-slot rejection,
> no-lenses rejection, and distinct planner cost-cap errors for the Stage 6
> handoff.
> Post-sweep hardening #284 replaces the dense-index exact-scan shortcut with
> native deterministic `ef` HNSW beam traversal and byte-readback recall FSV.
> Post-sweep hardening #286 refreshes `explain.provenance_hex` after stored
> constellation provenance is attached, removes AP-60 temporal slots 20/21/22
> from primary WeightedRRF profiles before PH40, and makes WeightedRRF skip slots
> not explicitly named by its profile. PH40 plus #615 later implemented and
> FSV-signed the AP-60 final surface as a post-retrieval stage.
> Post-sweep hardening #290 wires `FusionStrategy::Pipeline` to a real sparse
> recall candidate subset, returns no Pipeline hits when sparse stage 1 has no
> candidates, and makes reranker HTTP non-2xx responses fail closed.
> FSV root: `/home/croyse/calyx/data/fsv-issue290-sextant-pipeline-reranker-20260608`.
> Post-sweep hardening #312 closes the no-stage-1 Pipeline blind spot: a
> Pipeline query over dense-only slots now returns zero hits instead of falling
> back to unrestricted RRF.
> FSV root: `/home/croyse/calyx/data/fsv-issue312-pipeline-no-stage1-20260608`.
> Post-sweep hardening #296 wires the reranker into
> `SearchEngine::search_with_reranker` for final Pipeline ordering, with
> request-scoped candidate text and fail-closed non-2xx/mismatch behavior. This
> is a controlled SearchEngine wire FSV and is distinct from the Stage 4
> resident `:8089` reranker-score readback.
> FSV root: `/home/croyse/calyx/data/fsv-issue296-reranker-search-20260608`.
> Post-sweep hardening #297 adds `QueryFilters` for scalar, anchor, and
> built-in constellation metadata predicates in the SearchEngine path.
> FSV root: `/home/croyse/calyx/data/fsv-issue297-query-filters-20260608`.
> Post-sweep hardening #308 makes filtered searches use the full indexed
> candidate set before applying predicates, and rebuilds HNSW neighbor links
> when an existing `CxId` is reinserted with a new vector.
> FSV root: `/home/croyse/calyx/data/fsv-issue308-sextant-filter-hnsw-20260608`.
> Post-sweep hardening #305 removes the last public mock reranker scoring
> helper; PH26 reranking now either calls the request-scoped real HTTP reranker
> or fails closed.
> Post-sweep hardening #299 removes Sextant CPU-self GPU parity shims:
> MaxSim/quant parity requests now fail loud with
> `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE`, and SearchEngine fan-out is documented
> as per-slot CPU/index-owned until a real Forge grouped fan-out path is wired.
> FSV root: `/home/croyse/calyx/data/fsv-issue299-gpu-parity-fanout-20260608`.
> Post-sweep hardening #322 makes PH25 varint postings fail closed: unsorted
> doc IDs return `CALYX_SEXTANT_POSTINGS_NOT_SORTED`, malformed/truncated/overflow
> bytes return `CALYX_SEXTANT_POSTINGS_CORRUPT`, and the Stage 4 readback records
> exact bytes for the `[1,3,7] -> 010204` happy path.
> FSV root: `/home/croyse/calyx/data/fsv-issue322-postings-fail-closed-20260608`.
> Post-sweep hardening #323 makes PH25 sparse vector readback preserve the
> original non-contiguous sparse IDs and weights after insert and rebuild, while
> text overwrites clear stale sparse-vector readback state.
> FSV root: `/home/croyse/calyx/data/fsv-issue323-sparse-vector-readback-20260608`.
> Post-sweep hardening #324 adds configurable Pipeline recall headroom through
> `Query::recall_k`; Pipeline now recalls sparse candidates with `recall_k`
> before dense scoring/rerank and only then truncates to final `query.k`.
> FSV root: `/home/croyse/calyx/data/fsv-issue324-pipeline-recall-headroom-20260608`.
> Post-sweep hardening #325 makes `RerankRequest` own candidate strings as
> `Zeroizing<String>` and keeps the serialized HTTP body in `Zeroizing<String>`;
> FSV records the container types and captured wire request separately.
> FSV root: `/home/croyse/calyx/data/fsv-issue325-reranker-candidate-privacy-20260608`.
> Post-sweep hardening #326 adds `SearchEngine::planned_explain_search`, which
> plans first, executes the planned query, and returns planner intent/strategy/
> cost/timeout with the executed provenanced hits in one readback object.
> FSV root: `/home/croyse/calyx/data/fsv-issue326-planned-explain-path-20260608`.
> Post-sweep hardening #327 adds slot lifecycle state to `SlotIndexMap`: parked
> or retired slots are excluded from default search, and explicit inactive-slot
> search/insert/rebuild returns `CALYX_SEXTANT_SLOT_INACTIVE`.
> Post-sweep hardening #339 adds `Hit.provenance_source` and
> `Query::require_stored_provenance`; searches can now fail closed with
> `CALYX_SEXTANT_PROVENANCE_MISSING` instead of silently returning stub lineage.
> #339 also adds a Registry -> Aster backfill -> Sextant index/search FSV using
> Registry-produced vectors. The SciFact qrels FSV now requires stored
> provenance on returned hits and records the real-label RRF delta; resident
> TEI-produced dense-vector qrels remain a later dataset/eval-phase extension
> owned by PH69 T02/T03 and PH70 T01.
> FSV root for #339:
> `/home/croyse/calyx/data/fsv-issue339-registry-sextant-integration-20260608`
> (`registry-sextant-readback.json`
> `2163eeb8397de004a8a1c39e04631ccc7aa3f68836a7aa713bca7a6911cf6708`,
> `real-qrels-readback.json`
> `b687d33525be9a32e46feebc333254a089fe7772f0195b6bd5bead2efc16a3ef`).

The query engine: per-slot ANN, multi-lens fusion (RRF), provenance on every
hit, sparse/lexical search, and a planner that picks strategy by intent. The
payoff of the constellation architecture — many lenses, many ways to search.
Lands in `calyx-sextant`. Completing PH24 + the migration shadow is the
**recommended first demo** (PRD `19 §2`). **Living-system role:** cognition /
attention.

---

## PH23 — Per-slot HNSW index
- **Objective.** An in-RAM HNSW per dense slot; DiskANN is deferred to Stage 17
  (PH68 T01/T02/T04/T06). Each slot owns its index + quant config.
- **Deps.** PH20 (lenses), PH13 (distance).
- **Deliverables.** `index/hnsw.rs` implementing `Index`; insert on ingest;
  search with `ef`; dual-index scaffold for asymmetric slots.
- **Key tasks.** per-slot quant config; recall vs brute-force harness;
  concurrent-read-safe; rebuildable from base (self-heal later). Forge CUDA
  kernels are validated in Forge; Sextant does not claim a wired GPU HNSW or
  quantization path until that integration exists.
- **Post-sweep note.** `SlotIndexMap` now fails closed on duplicate slot
  registration with `CALYX_SEXTANT_SLOT_ALREADY_REGISTERED` (#282).
- **Post-sweep note.** `HnswIndex::search` now uses greedy descent plus
  `ef`-bounded beam traversal, with fail-closed empty-index, `ef`, and dim
  errors (#284). Brute force is retained only as a recall reference.
- **Post-sweep note.** Re-inserting an existing `CxId` updates the stored vector
  and rebuilds in-memory neighbor links so searches do not use stale topology
  after the vector moves (#308).
- **Post-sweep note.** MaxSim and quantization CPU/GPU delta helpers now return
  `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE` instead of comparing CPU output to
  itself; #299 readback records the explicit unavailable state.
- **FSV gate.** insert N + search → recall vs brute-force ≥ target; PH23 keeps
  the 10,000-row HNSW recall/p99 readback, and #640 adds release-mode 1e6-cx
  embedded-scale budget readback on aiwonder: SingleLens p99=686 us, RRF-6
  p99=3570 us, pipeline p99=17507 us, exact byte-identical known-I/O top hit.
  PH70 T01 still owns the later real-qrels recall delta gate. #327 proves
  parked/retired slots are not searched by default and fail closed when
  explicitly requested.
- **Axioms/PRD.** `10 §3`, `19 §4`.

## PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits
- **Objective.** Multi-lens fusion that beats single-lens recall, with every hit
  carrying its lineage.
- **Deps.** PH23, PH35 (Ledger stub for refs).
- **Deliverables.** `fusion/` (SingleLens, RRF `Σ w/(rank+60)`, WeightedRRF
  profiles), `Hit { cx, score, per_lens[], provenance, provenance_source,
  freshness }`, `explain`.
- **Key tasks.** rank fusion across chosen slots; per-lens contribution; attach
  `LedgerRef`; freshness (FreshDerived|StaleOk).
- **Post-sweep note.** WeightedRRF now treats missing profile weights as
  exclusion rather than implicit unit weight; plain RRF still assigns unit
  weights across participating slots (#286).
- **Post-sweep note.** Top-level `SearchEngine::search` currently fan-outs by
  calling each slot index in sequence and then fusing results; #299 documents
  this as `per_slot_cpu_index_calls` rather than a Forge grouped GPU fan-out.
- **Post-sweep note.** Hits now expose whether provenance came from a stored
  constellation or a deterministic stub. `Query::require_stored_provenance(true)`
  fails closed on missing stored rows (#339).
- **FSV gate.** multi-lens **recall@10 ≥ single-lens + Δ (≥15%)** on a real
  labeled corpus with qrels (BEIR SciFact subset on aiwonder); every Hit carries
  stored non-zero provenance when required, with deterministic stub fallback
  only when callers do not request stored provenance. The #339 SciFact readback
  proves real labels, stored-provenance enforcement, and retrieval/fusion
  mechanics; real resident-TEI dense-vector qrels are deferred to PH69 T02/T03
  and PH70 T01.
- **Axioms/PRD.** A15, `10 §2/§5`, `19 §4`.

## PH25 — Sparse lens inverted index
- **Objective.** Full-text/keyword as a sparse lexical **lens** (subsumes
  Elasticsearch, A19): inverted lists + BM25.
- **Deps.** PH24.
- **Deliverables.** `index/inverted.rs` (in-RAM postings with tokenizer/varint
  readback), BM25 scorer, SPLADE/keyword lens slot wiring; compressed SPANN
  tiering deferred to Stage 17 (PH68 T03/T06).
- **Key tasks.** term→postings; BM25; integrate as a slot in fusion + the
  `Pipeline` strategy (sparse recall → multi-lens score → rerank).
- **Post-sweep note.** Pipeline now uses inverted/sparse slots as stage-1
  candidates and final scoring is restricted to that candidate set; zero sparse
  candidates or no selected sparse stage returns zero Pipeline hits rather than
  dense fallback (#290, #312).
- **Post-sweep note.** Varint postings encoding now rejects unsorted input
  before bytes are written, and decoding rejects malformed, truncated, overflow,
  or delta-overflow blocks with explicit Sextant error codes (#322).
- **Post-sweep note.** Sparse vector inserts now retain the original
  `SparseEntry` IDs and weights for `vector()` readback; rebuild preserves the
  stored sparse vector, and text inserts clear stale vector readback (#323).
- **Post-sweep note.** Pipeline now uses configurable recall headroom
  (`Query::recall_k`, default `k*10`) for sparse stage-1 candidates before dense
  scoring and reranker request construction; final output remains capped at
  `query.k` (#324).
- **FSV gate.** term match + BM25 ranking correct on a known corpus; sparse lens
  participates in RRF/pipeline (read hits); postings readback proves byte-exact
  encoding plus fail-closed unsorted/corrupt edges; sparse vector readback
  proves non-contiguous original IDs/weights survive insert and rebuild; recall
  headroom proves a dense-preferred candidate outside sparse top-k is recovered
  when inside `recall_k`.
- **Axioms/PRD.** A19, `10 §2/§3`, `20 §2`.

## PH26 — Query planner + intent + explain
- **Objective.** Auto-select fusion strategy by intent (overridable); full
  `explain` breakdown.
- **Deps.** PH25.
- **Deliverables.** `planner.rs` (intent classifier → strategy; 14 ContextGraph
  weight profiles as defaults), reranker hook (reuse :8089), `explain=true`
  output, cost caps + timeouts.
- **Key tasks.** intent→strategy map; rerank stage (candidate text request-
  scoped, never persisted — privacy); bounded plans.
- **Post-sweep note.** Planner bounds now reject `k=0`, no-lenses, ef/slot
  over-cap, and cost-cap cases with distinct catalog codes (#282).
- **Post-sweep note.** Planner-selected temporal profile routes through semantic
  slot 8 only; AP-60 temporal slots 20/21/22 stayed out of primary retrieval.
  PH40/#615 later implemented the temporal boost as a post-retrieval surface
  with FSV evidence.
- **Post-sweep note.** Reranker requests now use the live TEI `texts` wire
  schema, parse rank-array responses back into candidate order, and fail closed
  on non-2xx status instead of returning mock scores (#290).
- **Post-sweep note.** `SearchEngine::search_with_reranker` now applies
  reranker scores to final Pipeline hit ordering using only candidate text from
  the sparse stage-1 index; it fails closed on non-Pipeline use, missing
  candidate text, non-2xx responses, or score-vector mismatch (#296).
- **Post-sweep note.** Reranker candidate text is wrapped in
  `Zeroizing<String>` as soon as `SearchEngine` pulls it from the sparse index,
  and `RerankRequest` owns `Vec<Zeroizing<String>>`; `zeroizing_ok` is no longer
  used as proof of candidate ownership (#325).
- **Post-sweep note.** `SearchEngine::planned_explain_search` now integrates
  `QueryPlanner::plan` with the executed search path and returns a
  `PlannerExplain` envelope containing intent, chosen strategy, override flag,
  cost estimate, timeout, and provenanced hits (#326).
- **Post-sweep note.** `QueryFilters` now executes scalar comparisons, anchor
  kind/value/source/confidence predicates, and built-in metadata predicates
  (vault, modality, panel version, created time, input redaction/pointer) against
  stored constellations; rows without stored constellation metadata are excluded
  fail-closed (#297).
- **Post-sweep note.** Filtered queries now request all indexed candidates for
  the selected slots before applying predicates and final `k` truncation; the
  former fixed widening window could miss valid lower-ranked filtered matches
  (#308).
- **FSV gate.** intent auto-selects the right strategy (verified per case);
  `explain=true` returns the per-lens + provenance breakdown; an unbounded plan
  is rejected; Pipeline reranker readback shows baseline order, reranked order,
  HTTP request text scope, zeroizing candidate container types, and
  `pipeline+rerank` strategy; planned explain readback shows planner intent,
  strategy, cost, timeout, and hit provenance in the same executed-path artifact;
  query filter readback
  shows unfiltered ids, filtered ids, provenance hashes, and excluded ids absent.
- **Axioms/PRD.** A17, `10 §2/§7`, `17 §7.3` (planner cost caps).

---

## Stage 4 exit — ✅ achieved
Multi-lens search beats single-lens on a real corpus, every hit is provenanced
and explainable, lexical search is just a lens, and the planner picks strategy
by intent — PRD `SEARCH`. With Stage 0–4 + a migration shadow, Calyx answers a
real vault with multiple lenses and provenance: the demo that justifies the
project.
