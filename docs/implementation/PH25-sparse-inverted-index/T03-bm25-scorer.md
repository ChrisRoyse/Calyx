# PH25 · T03 — BM25 scorer

| Field | Value |
|---|---|
| **Phase** | PH25 — Sparse lens inverted index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/bm25.rs` (≤500) |
| **Depends on** | T02 (this phase) |
| **Axioms** | A16, A19 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/20 §2` |

## Goal

Implement BM25 with standard defaults (`b=0.75`, `k1=1.2`) that match Lucene and
the academic definition. The scorer is called per-term during `InvertedIndex::search`;
it uses the index's document statistics (N, avgdl, per-doc lengths, postings).
Correct ranking on a known corpus is the primary deliverable.

## Build (checklist of concrete, code-level steps)

- [x] `Bm25Config` struct: `k1: f32 = 1.2, b: f32 = 0.75` with `Default` impl
- [x] `fn bm25_score(tf: u32, df: u32, n: u32, avgdl: f32, dl: u32, cfg: &Bm25Config) -> f32`:
      ```
      idf = ln((n - df + 0.5) / (df + 0.5) + 1.0)
      tf_norm = (tf as f32 * (cfg.k1 + 1.0)) / (tf as f32 + cfg.k1 * (1.0 - cfg.b + cfg.b * dl as f32 / avgdl))
      score = idf * tf_norm
      ```
      (matches Lucene's BM25Similarity formula exactly)
- [x] `fn score_query(index: &InvertedIndex, query_tokens: &[String], cfg: &Bm25Config) -> HashMap<u32, f32>`:
      for each query token, iterate its postings, accumulate `bm25_score` per doc_id
- [x] Wire into `InvertedIndex::search`: call `score_query`, sort by score desc,
      map internal doc_ids back to `CxId`, return top-k
- [x] `CALYX_SEXTANT_BM25_ZERO_DOCS`: if `index.total_docs == 0`, fail-closed

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: hand-crafted corpus (3 docs, known TF/DF):
      ```
      doc0: "the cat sat on the mat"         (6 tokens)
      doc1: "the cat in the hat"             (5 tokens)
      doc2: "the dog barked at the postman"  (6 tokens)
      ```
      query="cat hat" → doc1 should rank highest (has both terms); compute
      expected score manually and assert within f32 tolerance 1e-3
- [x] unit: `bm25_score(tf=1, df=1, n=1, avgdl=5.0, dl=5, cfg=default)` →
      known value (compute: idf=ln(1.5), tf_norm=2.2/2.2=1.0, score=ln(1.5)≈0.405);
      assert within 1e-4
- [x] unit: increasing tf → increasing score (monotone in TF for fixed DF)
- [x] unit: increasing df → decreasing score (monotone in DF for fixed TF)
- [x] proptest: `bm25_score` is non-negative for all valid non-zero inputs
- [x] edge: `df == 0` → treat as 1 (defensive; IDF denominator never 0)
- [x] edge: `total_docs == 0` → `CALYX_SEXTANT_BM25_ZERO_DOCS`
- [x] fail-closed: NaN in score → `CALYX_SEXTANT_BM25_NAN` (check after computation;
      caused by extreme avgdl=0.0 edge case)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant bm25 -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant bm25 -- --nocapture 2>&1`
- **Prove:** test prints `query='cat hat' top1=doc1 score=NNN expected_top1=doc1 ok=true`;
  the golden score value (NNN) is computed from the formula above, printed, and
  locked as a constant in the test

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH25 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
