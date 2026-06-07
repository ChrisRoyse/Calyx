# PH25 — Sparse lens inverted index

**Stage:** S4 — Sextant Search & Navigation  ·  **Crate:** `calyx-sextant`  ·
**PRD roadmap:** P3  ·  **Axioms:** A19, A16

## Objective

Full-text/keyword search as a sparse lexical lens, subsuming Elasticsearch (A19):
an inverted index with varint+zstd-compressed postings lists and a BM25 scorer.
The sparse lens wires into the existing fusion layer as a first-class slot, so
`RRF` and `WeightedRRF` gain lexical recall automatically. The `Pipeline`
strategy (sparse recall → multi-lens score → rerank) is also implemented here,
providing the maximum-precision path (`10 §2`). SPANN tiering is deferred to
Stage 17 (PH68). The FSV gate is term-match + BM25 ranking correct on a known
corpus, with the sparse lens participating in RRF and Pipeline (read the hits
on aiwonder).

## Dependencies

- **Phases:** PH24 (fusion layer — `SlotIndexMap`, `FusionStrategy`, `Hit`),
  PH06 (SSTable writer/reader for postings persistence, optional at this stage —
  in-RAM is sufficient for PH25; disk-backed deferred to PH68)
- **Provides for:** PH26 (planner uses `Pipeline` strategy), PH40 (temporal
  boost applies after Pipeline), PH55 (universal query surface routes BM25
  through Sextant), PH68 (DiskANN/SPANN replaces in-RAM inverted index at scale)

## Current state (build off what exists)

`calyx-sextant` has HNSW (`SlotIndexMap`, `Index` trait) and fusion
(`RRF`/`WeightedRRF`/`SingleLens`, `Hit`, `search()`) from PH23–PH24. The
sparse slot is a no-op placeholder in the PH24 recall harness. This phase makes
it real.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/index/inverted.rs` | in-RAM inverted index: posting lists, varint encoding, zstd block compression |
| `crates/calyx-sextant/src/index/bm25.rs` | BM25 scorer: IDF, TF normalization, `b=0.75 k1=1.2` defaults |
| `crates/calyx-sextant/src/index/tokenizer.rs` | whitespace + punctuation tokenizer; lowercase; stopwords optional |
| `crates/calyx-sextant/src/fusion/pipeline.rs` | `Pipeline` strategy: sparse recall → multi-lens score → rerank hook |
| `crates/calyx-sextant/tests/sparse_bm25.rs` | BM25 ranking correctness + Pipeline recall on a known corpus |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Tokenizer + varint postings encoding | — |
| T02 | Inverted index: build, insert, term lookup | T01 |
| T03 | BM25 scorer | T02 |
| T04 | Sparse `Index` impl + `SlotIndexMap` wiring | T03 |
| T05 | `Pipeline` strategy (sparse → multi-lens → rerank hook) | T04 |
| T06 | Sparse lens in RRF/Pipeline: FSV on known corpus | T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run `cargo test -p calyx-sextant sparse_bm25 -- --nocapture` on aiwonder. The
test must print:
- `bm25_top1=<expected_doc_id>` matching the hand-labeled corpus answer
- `pipeline_recall@10=NNN` where NNN ≥ the single-lens baseline
- `rrf_with_sparse_recall@10=NNN` showing the sparse lens participates in RRF

Screenshot or copy of these three lines attached to the PH25 GitHub issue.

## Risks / landmines

- **varint correctness**: off-by-one in delta encoding (d-gaps) corrupts all
  postings; use a known-good test vector from a reference implementation and
  assert byte-exact round-trip.
- **zstd at small block sizes**: the zstd compression ratio for very short
  postings lists may expand rather than compress; document the threshold below
  which raw varint is stored uncompressed.
- **BM25 k1/b tuning**: defaults `b=0.75 k1=1.2` match Lucene and are the
  correct starting point; do not make them per-query-configurable yet — planner
  will handle this in PH26/PH46.
- **SPANN deferral**: do not add any on-disk tiering or centroid-based routing
  here; the `Index` trait seam must be clean so Stage 17 can swap in SPANN.
