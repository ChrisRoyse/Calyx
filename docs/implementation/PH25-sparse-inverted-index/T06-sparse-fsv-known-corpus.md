# PH25 · T06 — Sparse lens in RRF/Pipeline: FSV on known corpus

| Field | Value |
|---|---|
| **Phase** | PH25 — Sparse lens inverted index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/tests/sparse_bm25.rs` (≤500) |
| **Depends on** | T05 (this phase) |
| **Axioms** | A19, A16 |
| **PRD** | `dbprdplans/10 §2`, `dbprdplans/10 §3`, `dbprdplans/20 §2` |

## Goal

The PH25 exit gate: prove on aiwonder that (1) BM25 ranking is correct on a
known corpus (specific top-1 matches expected), (2) the sparse lens participates
in RRF fusion and changes the result order vs dense-only, (3) the Pipeline
strategy produces a valid ranked list from sparse recall → multi-lens score.
This closes the "subsumes Elasticsearch" claim (A19).

## Build (checklist of concrete, code-level steps)

- [x] `tests/sparse_bm25.rs` — always-runs corpus test (small corpus, no external
      dataset required):
      - Corpus (20 hand-written documents covering: programming, history, cooking,
        science — seeded, locked in the test file as string constants):
        ```
        doc0: "Rust is a systems programming language focused on safety"
        doc1: "Python is a high-level programming language"
        doc2: "The French Revolution began in 1789"
        doc3: "The American Revolution was in 1776"
        … (17 more)
        ```
      - Query: `"programming language"` → assert `doc0` and `doc1` in top-3
      - Query: `"revolution 1789"` → assert `doc2` in top-1
      - RRF test: also insert 128-dim random unit vecs for a dense slot (seeded);
        run RRF with both slots; assert result set differs from BM25-only (at
        least one position change in top-5)
      - Pipeline test: run Pipeline with sparse_slot + dense_slot, recall_k=50,
        rerank=None; assert top-10 is non-empty and contains the expected docs
- [x] Print at end:
      ```
      bm25_top1=doc2_ok=true rrf_differs_from_sparse_only=true pipeline_top10_nonempty=true
      ```
- [x] Mark the RRF-with-dense test `#[ignore]` only if TEI is required; since
      dense vecs here are random (not embedded), it always runs

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] integration (always runs): `"programming language"` → doc0 or doc1 in top-2
- [x] integration (always runs): `"revolution 1789"` → doc2 in top-1
- [x] integration: RRF with sparse+dense → result differs from sparse-only
- [x] integration: Pipeline top-10 is a subset of sparse recall candidates
- [x] unit: `compute_recall_at_k` re-used from PH24 harness — import, do not copy
- [x] edge: query with no matching terms → `Ok(vec![])`, no panic
- [x] fail-closed: corpus load failure (missing constant) → compile error, not
      runtime panic (constants are inline in the test file)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stdout of `cargo test -p calyx-sextant sparse_bm25 -- --nocapture` on
  aiwonder
- **Readback:** `cargo test -p calyx-sextant sparse_bm25 -- --nocapture 2>&1 | grep -E 'bm25|rrf|pipeline'`
- **Prove:** must print `bm25_top1=doc2_ok=true rrf_differs_from_sparse_only=true
  pipeline_top10_nonempty=true`; screenshot attached to the PH25 GitHub issue as
  FSV evidence

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH25 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
