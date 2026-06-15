# PH25 · T02 — Inverted index: build, insert, term lookup

| Field | Value |
|---|---|
| **Phase** | PH25 — Sparse lens inverted index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/inverted.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH23 T01 (`Index` trait) |
| **Axioms** | A16, A19 |
| **PRD** | `dbprdplans/10 §3` |

## Goal

Build the `InvertedIndex` struct: a `HashMap<String, PostingsList>` with
document statistics (total docs, avg doc length, per-doc token counts) needed
by BM25. Implements `Index` trait with text-based insert and term-lookup search.
In-RAM only; SPANN disk tiering is Stage 17.

## Current implementation note

The shipped `InvertedIndex` stores text for BM25/candidate text and, for the
`insert(SlotVector::Sparse)` path, also stores the original sparse vector for
`vector()` readback. Post-sweep #323 proves non-contiguous sparse IDs and
weights survive insert and rebuild, while a later `insert_text` overwrite clears
the stale vector readback.

## Build (checklist of concrete, code-level steps)

- [x] `PostingsList` struct:
  ```rust
  pub struct PostingsList {
      pub doc_ids: Vec<u32>,          // sorted, delta-encoded on disk
      pub term_freqs: Vec<u32>,       // parallel to doc_ids
      pub compressed: Option<Vec<u8>>,// None = use doc_ids directly (small list)
  }
  ```
- [x] `InvertedIndex` struct:
  ```rust
  pub struct InvertedIndex {
      terms: HashMap<String, PostingsList>,
      doc_lengths: HashMap<u32, u32>,  // internal doc_id -> token count
      total_docs: u32,
      sum_doc_lengths: u64,
      cx_to_docid: HashMap<CxId, u32>,
      docid_to_cx: Vec<CxId>,
      tokenizer_config: TokenizerConfig,
  }
  ```
- [x] `fn insert_document(&mut self, id: CxId, text: &str) -> Result<(), CalyxError>`:
      assign internal doc_id, tokenize, update `PostingsList` per term,
      record `doc_lengths`
- [x] `fn lookup_term(&self, term: &str) -> Option<&PostingsList>`
- [x] `fn term_count(&self) -> usize` — number of unique terms
- [x] Implement `Index` trait:
      - `insert` expects the caller to pass a pre-embedded "vector" that is
        actually the raw text encoded as UTF-8 bytes in a `Vec<f32>` (via a
        newtype or a text_as_vec helper); alternatively, add a separate
        `insert_text` method and have `insert` return `CALYX_SEXTANT_WRONG_INDEX_KIND`
        if called with a float vec on a sparse index — document this clearly
      - `search` takes the query text (same encoding), tokenizes, scores via BM25
        (T03), returns top-k `(CxId, f32)` pairs
      - `remove`: mark doc_id as tombstoned; excluded from search results;
        postings not compacted until rebuild
      - `rebuild`: re-inserts all non-tombstoned documents from scratch

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: insert 3 docs, `lookup_term("foo")` → doc_ids contains the correct
      subset; `term_count()` returns the correct unique term count
- [x] unit: insert then remove a doc → `search("foo")` no longer returns that cx
- [x] unit: sparse vector insert with non-contiguous IDs returns the original IDs
      and weights from `vector(cx)`; rebuild preserves the same readback
- [x] unit: `total_docs` and `sum_doc_lengths` are updated correctly on each insert
- [x] proptest: for any set of docs, `lookup_term(t).doc_ids` is a subset of all
      inserted doc_ids
- [x] edge: insert empty text → 0 tokens, doc_length=0, still tracked in `total_docs`
- [x] edge: remove non-existent cx → `Ok(false)` (idempotent)
- [x] fail-closed: text passed to `insert` via the wrong vector encoding path →
      `CALYX_SEXTANT_WRONG_INDEX_KIND` with remediation hint

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant inverted_index -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant inverted_index -- --nocapture 2>&1`
- **Prove:** prints `term_count=N total_docs=3 lookup_foo_len=M remove_ok=true`
  with N and M matching the expected values for the seeded test corpus
- **Post-sweep #323 SoT:**
  `/home/croyse/calyx/data/fsv-issue323-sparse-vector-readback-20260608/sparse-vector-readback.json`
  proves `insert_preserves_sparse_ids=true`, `rebuild_preserves_sparse_ids=true`,
  and `text_overwrite_clears_stale_sparse_ids=true`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH25 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
