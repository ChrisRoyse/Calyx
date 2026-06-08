# Stage 4 — Sextant Search & Navigation (PH23–PH26)

> **STATUS: ✅ DONE (FSV-signed-off, commit `9dc197c`).** `calyx-sextant`
> implements dense/sparse slot indexes, RRF/WeightedRRF/SingleLens fusion with
> provenance and freshness, planner/explain/navigation, tokenizer/varint/BM25,
> and real SciFact qrels evidence. FSV root:
> `/home/croyse/calyx/data/fsv-stage4-sextant-20260608003414`; final evidence
> hash `796b4812a3e2ac47a6ace81934be5799514d94f7e42b28b45b265386a98b6db8`.
> Stage 5 has consumed Sextant successfully; next active stage is Lodestar
> (`16_STAGE6_LODESTAR.md`).

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
- **FSV gate.** multi-lens **recall@10 ≥ single-lens + Δ (≥15%)** on a real
  labeled corpus with qrels (BEIR/MS MARCO subset on aiwonder); every Hit
  carries a real provenance ref (read it).
- **Axioms/PRD.** A15, `10 §2/§5`, `19 §4`.

## PH25 — Sparse lens inverted index
- **Objective.** Full-text/keyword as a sparse lexical **lens** (subsumes
  Elasticsearch, A19): inverted lists + BM25.
- **Deps.** PH24.
- **Deliverables.** `index/inverted.rs` (postings, varint+zstd blocks), BM25
  scorer, SPLADE/keyword lens slot wiring; SPANN tiering deferred to Stage 17.
- **Key tasks.** term→postings; BM25; integrate as a slot in fusion + the
  `Pipeline` strategy (sparse recall → multi-lens score → rerank).
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
- **FSV gate.** intent auto-selects the right strategy (verified per case);
  `explain=true` returns the per-lens + provenance breakdown; an unbounded plan
  is rejected.
- **Axioms/PRD.** A17, `10 §2/§7`, `17 §7.3` (planner cost caps).

---

## Stage 4 exit — ✅ achieved
Multi-lens search beats single-lens on a real corpus, every hit is provenanced
and explainable, lexical search is just a lens, and the planner picks strategy
by intent — PRD `SEARCH`. With Stage 0–4 + a migration shadow, Calyx answers a
real vault with multiple lenses and provenance: the demo that justifies the
project.
