# PH54 · T03 — Inverted index: term-match + BM25 (reuse PH25)

| Field | Value |
|---|---|
| **Phase** | PH54 — Secondary indexes (btree/inverted) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/index/inverted.rs` (≤500) |
| **Depends on** | T01 (SecondaryIndex trait), PH25 (inverted posting-list infrastructure in calyx-sextant) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/20 §1/§2`, `dbprdplans/04 §2` |

## Goal

Implement the inverted secondary index for text fields in the general data
layer. Key encoding: `(idx_id, term_hash, pk) → weight` (discriminant `0x11`).
This reuses the posting-list write logic from PH25 (BM25 IDF computation,
term tokenization) — do not re-implement; call into the existing
`calyx-sextant` inverted-list primitives or copy only the encoding function
into `calyx-aster` to avoid a circular dependency. Supports: term-match
(exact term presence), multi-term AND/OR, and BM25 scoring.

## Build (checklist of concrete, code-level steps)

- [ ] Define inverted key schema (discriminant `0x11`):
  ```
  key = 0x11 | collection_id (8B BE) | index_id (4B BE) | term_hash (8B BE, FNV-1a of lowercase term) | pk_bytes
  val = weight (f32 BE) — TF-IDF weight for the term in this document
  ```
- [ ] Implement `InvertedIndex` struct implementing `SecondaryIndex`:
  - `encode_index_key(field_val: &FieldValue::Text, pk)`: tokenize text into
    terms (whitespace + punctuation split, lowercase); emit one key per term.
  - `encode_scan_prefix(field_val: &FieldValue::Text)`: one term only (used
    for point lookup of a single term).
  - Weights: `weight = tf / (tf + k1 * (1 - b + b * dl / avgdl))` (BM25
    component; IDF is applied at query time). `k1=1.2`, `b=0.75`. `tf` = term
    frequency in this field value. `dl` = document length in tokens.
    `avgdl` stored as a running average in a stats row in the index CF.
- [ ] Implement `inverted_update_avgdl(vault, col, spec, new_dl: u32) -> Result<()>`:
  - Read current `avgdl_stats` row (key = `0x11 | collection_id | index_id | 0xFF_FF_...`).
  - Update running average `avgdl = (avgdl * doc_count + new_dl) / (doc_count + 1)`.
  - Write back in the same WAL batch as the posting entries.
- [ ] Implement `inverted_match(vault, col, spec, term: &str) -> Result<Vec<(RecordKey, f32)>>`:
  - Range scan `[0x11 | col | idx | term_hash | 0x00..] → [.. | term_hash | 0xFF..]`.
  - Return `(pk, weight)` pairs sorted by descending weight.
- [ ] Implement `inverted_bm25(vault, col, spec, terms: &[&str], n_docs: u64, limit: usize) -> Result<Vec<(RecordKey, f32)>>`:
  - For each term: `inverted_match`, compute `score += weight * idf(term, n_docs, match_count)`.
  - Merge scores across terms (AND = must appear in all; OR = sum).
  - Return top-`limit` by score.
- [ ] Copy only the term-hash and tokenization functions from PH25 into a private
  `index/terms.rs` helper (≤200 lines) to avoid calyx-sextant circular dep.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: index `{pk=1, body="the quick brown fox"}` and `{pk=2, body="quick lazy dog"}`;
  `inverted_match("quick")` → `[(pk=1, w1), (pk=2, w2)]` both present; `w1` and
  `w2` are positive finite f32.
- [ ] unit: `inverted_bm25(["quick","fox"], n_docs=2)` → pk=1 scores higher than
  pk=2 (pk=1 matches both terms).
- [ ] proptest: `inverted_match(term)` always returns only records containing that
  exact term (no false positives); for a random corpus of N docs.
- [ ] edge (≥3): (1) term not in any doc → empty result; (2) empty field value →
  no index entries written; (3) very long text (5000 tokens) → all tokens
  indexed, no truncation; (4) two docs with identical text → both returned.
- [ ] fail-closed: `FieldValue` not `Text` passed to `InvertedIndex` →
  `CALYX_INVALID_ARGUMENT`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `index_inverted` CF in the vault SST.
- **Readback:**
  ```
  calyx collection create --vault /home/croyse/calyx/test-vault --name text_col --mode records --index inverted:body:text
  calyx record put --vault /home/croyse/calyx/test-vault --collection text_col --pk 1 --data '{"body":"the quick brown fox"}'
  calyx record put --vault /home/croyse/calyx/test-vault --collection text_col --pk 2 --data '{"body":"quick lazy dog"}'
  calyx index term-match --vault /home/croyse/calyx/test-vault --collection text_col --index body --term quick
  calyx index bm25 --vault /home/croyse/calyx/test-vault --collection text_col --index body --terms "quick fox"
  xxd /home/croyse/calyx/test-vault/cf/index_inverted/000001.sst | head -8
  ```
- **Prove:** `term-match("quick")` returns pks `{1,2}`; `bm25("quick fox")`
  returns pk=1 ranked above pk=2; `xxd` shows `0x11` discriminant.
  Evidence posted to PH54 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH54 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
