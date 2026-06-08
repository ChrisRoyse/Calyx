# PH25 — Sparse lens inverted index

**Stage:** S4 — Sextant Search & Navigation  ·  **Crate:** `calyx-sextant`  ·
**PRD roadmap:** P3  ·  **Axioms:** A19, A16

## Objective

Full-text/keyword search as a sparse lexical lens, subsuming Elasticsearch (A19):
an in-RAM inverted index with tokenizer/varint readback and a BM25 scorer.
The sparse lens wires into the existing fusion layer as a first-class slot, so
`RRF` and `WeightedRRF` gain lexical recall automatically. The `Pipeline`
strategy now enforces sparse stage-1 candidate subsets before multi-lens scoring;
final reranker ordering through `SearchEngine` is implemented and FSV-backed by
issue #296. SPANN
tiering is deferred to Stage 17 (PH68). The FSV gate is term-match + BM25 ranking
correct on a known corpus, with the sparse lens participating in RRF and Pipeline
(read the hits on aiwonder).

## Dependencies

- **Phases:** PH24 (fusion layer — `SlotIndexMap`, `FusionStrategy`, `Hit`),
  PH06 (SSTable writer/reader for postings persistence, optional at this stage —
  in-RAM is sufficient for PH25; disk-backed deferred to PH68)
- **Provides for:** PH26 (planner uses `Pipeline` strategy), PH40 (temporal
  boost applies after Pipeline), PH55 (universal query surface routes BM25
  through Sextant), PH68 (DiskANN/SPANN replaces in-RAM inverted index at scale)

## Current state

PH25 is implemented and FSV-signed off. `InvertedIndex` is a real sparse slot,
BM25 participates in RRF, and post-sweep #290 wires `FusionStrategy::Pipeline`
to use sparse/inverted results as the stage-1 candidate set before multi-lens
scoring. Final Pipeline hits are constrained to that candidate set, and an
empty sparse stage 1 returns no Pipeline hits instead of falling back to dense
RRF. Post-sweep #322 hardens the varint postings codec: unsorted doc IDs fail
closed before encoding, and malformed/truncated/overflow bytes fail closed on
decode with cataloged Sextant errors. Post-sweep #323 preserves original sparse
vector IDs and weights for `vector()` readback after sparse-vector insert and
rebuild, while text inserts clear stale vector readback.
Post-sweep #324 adds configurable Pipeline recall headroom via `Query::recall_k`
so sparse stage 1 can recall more than final `k` before dense scoring and
reranker request construction; final results remain capped at `query.k`.

Compressed postings blocks and SPANN tiering are deferred to PH68; the current
Stage 4 source of truth is the in-memory index plus byte-readback FSV artifacts.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/index/inverted.rs` | in-RAM inverted index: posting lists and term lookup |
| `crates/calyx-sextant/src/index/bm25.rs` | BM25 scorer: IDF, TF normalization, `b=0.75 k1=1.2` defaults |
| `crates/calyx-sextant/src/index/tokenizer.rs` | whitespace + punctuation tokenizer; lowercase; stopwords optional |
| `crates/calyx-sextant/src/fusion/pipeline.rs` | `Pipeline` strategy: sparse recall → multi-lens score over the bounded candidate subset; final rerank ordering through `SearchEngine::search_with_reranker` (#296) |
| `crates/calyx-sextant/tests/stage4_fsv.rs` | BM25 ranking correctness + Pipeline subset readback on a known corpus |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Tokenizer + varint postings encoding | — |
| T02 | Inverted index: build, insert, term lookup | T01 |
| T03 | BM25 scorer | T02 |
| T04 | Sparse `Index` impl + `SlotIndexMap` wiring | T03 |
| T05 | `Pipeline` strategy (sparse → multi-lens bounded candidate subset; rerank ordering implemented by #296) | T04 |
| T06 | Sparse lens in RRF/Pipeline: FSV on known corpus | T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run the Stage 4 FSV on aiwonder. The readback JSON must include:
- `sparse_top=<expected_doc_id>` matching the hand-labeled corpus answer.
- `pipeline_subset_ok=true`, proving final Pipeline hits came from sparse
  stage-1 candidates.
- `pipeline_empty_stage1_hits=0`, proving zero sparse candidates do not fall
  back to dense-only hits.
- `rrf_top_differs_from_single=true`, proving sparse/multi-lens fusion changes
  the result surface.
- `varint_hex="010204"` and `varint_decoded=[1,3,7]`, proving the known postings
  happy path is byte-exact.
- `postings_unsorted_error="CALYX_SEXTANT_POSTINGS_NOT_SORTED"` and
  `postings_corrupt_error="CALYX_SEXTANT_POSTINGS_CORRUPT"`, proving fail-closed
  boundary handling.
- `insert_preserves_sparse_ids=true`, `rebuild_preserves_sparse_ids=true`, and
  `text_overwrite_clears_stale_sparse_ids=true`, proving sparse vector readback
  preserves original non-contiguous IDs/weights and does not keep stale state.
- `recovered_outside_sparse_top_k=true`, `wide_final_len=1`, and
  `reranker_request_text_count=3`, proving `recall_k` headroom is used before
  dense scoring/rerank while final output remains capped at `query.k`.
- #296 `reranker-search-readback.json` shows baseline order `03..03, 02..02`
  changed to reranked order `02..02, 03..03`, `strategy=pipeline+rerank`, and
  candidate text scoped to sparse stage-1 rows.

For #290 the readback root is
`/home/croyse/calyx/data/fsv-issue290-sextant-pipeline-reranker-20260608`.
For #296 the readback root is
`/home/croyse/calyx/data/fsv-issue296-reranker-search-20260608`.
For #322 the readback root is
`/home/croyse/calyx/data/fsv-issue322-postings-fail-closed-20260608`.
For #323 the readback root is
`/home/croyse/calyx/data/fsv-issue323-sparse-vector-readback-20260608`.
For #324 the readback root is
`/home/croyse/calyx/data/fsv-issue324-pipeline-recall-headroom-20260608`.

## Risks / landmines

- **varint correctness**: off-by-one in delta encoding (d-gaps) corrupts all
  postings; the current codec asserts byte-exact `[1,3,7] -> 010204`, rejects
  unsorted input, and rejects corrupt/truncated/overflow encoded bytes.
- **sparse vector readback**: vector inserts and text inserts share the same
  inverted text index, but `vector()` must return the original sparse IDs and
  weights only when a sparse vector was actually inserted.
- **compressed postings deferral**: zstd/SPANN persistence is PH68 work; do not
  describe the Stage 4 in-RAM sparse slot as disk-tiered or compressed.
- **BM25 k1/b tuning**: defaults `b=0.75 k1=1.2` match Lucene and are the
  correct starting point; do not make them per-query-configurable yet — planner
  will handle this in PH26/PH46.
- **SPANN deferral**: do not add any on-disk tiering or centroid-based routing
  here; the `Index` trait seam must be clean so Stage 17 can swap in SPANN.
