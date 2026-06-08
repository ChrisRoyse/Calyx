# Stage 4 — Sextant Search & Navigation (PH23–PH26)

> **STATUS: ✅ DONE (FSV-signed-off, commit `9dc197c`).** `calyx-sextant`
> implements dense/sparse slot indexes, RRF/WeightedRRF/SingleLens fusion with
> provenance and freshness, planner/explain/navigation, tokenizer/varint/BM25,
> and real SciFact qrels evidence. FSV root:
> `/home/croyse/calyx/data/fsv-stage4-sextant-20260608003414`; final evidence
> hash `796b4812a3e2ac47a6ace81934be5799514d94f7e42b28b45b265386a98b6db8`.
> Stage 5 has consumed Sextant successfully; next active stage is Lodestar
> (`16_STAGE6_LODESTAR.md`).
> Post-sweep fail-closed hardening #282 adds duplicate-slot rejection,
> no-lenses rejection, and distinct planner cost-cap errors for the Stage 6
> handoff.
> Post-sweep hardening #284 replaces the dense-index exact-scan shortcut with
> native deterministic `ef` HNSW beam traversal and byte-readback recall FSV.
> Post-sweep hardening #286 refreshes `explain.provenance_hex` after stored
> constellation provenance is attached, removes AP-60 temporal slots 20/21/22
> from primary WeightedRRF profiles until PH40, and makes WeightedRRF skip slots
> not explicitly named by its profile.
> Post-sweep hardening #290 wires `FusionStrategy::Pipeline` to a real sparse
> recall candidate subset, returns no Pipeline hits when sparse stage 1 has no
> candidates, and makes reranker HTTP non-2xx responses fail closed.
> FSV root: `/home/croyse/calyx/data/fsv-issue290-sextant-pipeline-reranker-20260608`.
> Post-sweep hardening #296 wires the reranker into
> `SearchEngine::search_with_reranker` for final Pipeline ordering, with
> request-scoped candidate text and fail-closed non-2xx/mismatch behavior.
> FSV root: `/home/croyse/calyx/data/fsv-issue296-reranker-search-20260608`.

The query engine: per-slot ANN, multi-lens fusion (RRF), provenance on every
hit, sparse/lexical search, and a planner that picks strategy by intent. The
payoff of the constellation architecture — many lenses, many ways to search.
Lands in `calyx-sextant`. Completing PH24 + the migration shadow is the
**recommended first demo** (PRD `19 §2`). **Living-system role:** cognition /
attention.

---

## PH23 — Per-slot HNSW index
- **Objective.** An in-RAM HNSW per dense slot (DiskANN deferred to Stage 17);
  each slot owns its index + quant config.
- **Deps.** PH20 (lenses), PH13 (distance).
- **Deliverables.** `index/hnsw.rs` implementing `Index`; insert on ingest;
  search with `ef`; dual-index scaffold for asymmetric slots.
- **Key tasks.** quantized vectors via Forge; recall vs brute-force harness;
  concurrent-read-safe; rebuildable from base (self-heal later).
- **Post-sweep note.** `SlotIndexMap` now fails closed on duplicate slot
  registration with `CALYX_SEXTANT_SLOT_ALREADY_REGISTERED` (#282).
- **Post-sweep note.** `HnswIndex::search` now uses greedy descent plus
  `ef`-bounded beam traversal, with fail-closed empty-index, `ef`, and dim
  errors (#284). Brute force is retained only as a recall reference.
- **FSV gate.** insert N + search → recall vs brute-force ≥ target; SingleLens
  p99 within budget on aiwonder (read measured latency).
- **Axioms/PRD.** `10 §3`, `19 §4`.

## PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits
- **Objective.** Multi-lens fusion that beats single-lens recall, with every hit
  carrying its lineage.
- **Deps.** PH23, PH35 (Ledger stub for refs).
- **Deliverables.** `fusion/` (SingleLens, RRF `Σ w/(rank+60)`, WeightedRRF
  profiles), `Hit { cx, score, per_lens[], provenance, freshness }`, `explain`.
- **Key tasks.** rank fusion across chosen slots; per-lens contribution; attach
  `LedgerRef`; freshness (FreshDerived|StaleOk).
- **Post-sweep note.** WeightedRRF now treats missing profile weights as
  exclusion rather than implicit unit weight; plain RRF still assigns unit
  weights across participating slots (#286).
- **FSV gate.** multi-lens **recall@10 ≥ single-lens + Δ (≥15%)** on a real
  labeled corpus with qrels (BEIR/MS MARCO subset on aiwonder); every Hit
  carries a real provenance ref (read it).
- **Axioms/PRD.** A15, `10 §2/§5`, `19 §4`.

## PH25 — Sparse lens inverted index
- **Objective.** Full-text/keyword as a sparse lexical **lens** (subsumes
  Elasticsearch, A19): inverted lists + BM25.
- **Deps.** PH24.
- **Deliverables.** `index/inverted.rs` (in-RAM postings with tokenizer/varint
  readback), BM25 scorer, SPLADE/keyword lens slot wiring; compressed SPANN
  tiering deferred to Stage 17.
- **Key tasks.** term→postings; BM25; integrate as a slot in fusion + the
  `Pipeline` strategy (sparse recall → multi-lens score → rerank).
- **Post-sweep note.** Pipeline now uses inverted/sparse slots as stage-1
  candidates and final scoring is restricted to that candidate set; zero sparse
  candidates returns zero Pipeline hits rather than dense fallback (#290).
- **FSV gate.** term match + BM25 ranking correct on a known corpus; sparse lens
  participates in RRF/pipeline (read hits).
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
- **Post-sweep note.** Planner-selected temporal profile currently routes
  through semantic slot 8 only; AP-60 temporal slots 20/21/22 are reserved for
  PH40 post-retrieval temporal boost rather than primary retrieval (#286).
- **Post-sweep note.** Reranker requests now use the live TEI `texts` wire
  schema, parse rank-array responses back into candidate order, and fail closed
  on non-2xx status instead of returning mock scores (#290).
- **Post-sweep note.** `SearchEngine::search_with_reranker` now applies
  reranker scores to final Pipeline hit ordering using only candidate text from
  the sparse stage-1 index; it fails closed on non-Pipeline use, missing
  candidate text, non-2xx responses, or score-vector mismatch (#296).
- **FSV gate.** intent auto-selects the right strategy (verified per case);
  `explain=true` returns the per-lens + provenance breakdown; an unbounded plan
  is rejected; Pipeline reranker readback shows baseline order, reranked order,
  HTTP request text scope, and `pipeline+rerank` strategy.
- **Axioms/PRD.** A17, `10 §2/§7`, `17 §7.3` (planner cost caps).

---

## Stage 4 exit — ✅ achieved
Multi-lens search beats single-lens on a real corpus, every hit is provenanced
and explainable, lexical search is just a lens, and the planner picks strategy
by intent — PRD `SEARCH`. With Stage 0–4 + a migration shadow, Calyx answers a
real vault with multiple lenses and provenance: the demo that justifies the
project.
