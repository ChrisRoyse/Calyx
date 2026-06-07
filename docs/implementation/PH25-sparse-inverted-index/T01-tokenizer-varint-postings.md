# PH25 · T01 — Tokenizer + varint postings encoding

| Field | Value |
|---|---|
| **Phase** | PH25 — Sparse lens inverted index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/tokenizer.rs` (≤500), `crates/calyx-sextant/src/index/inverted.rs` (≤500 — postings encoding portion) |
| **Depends on** | PH24 T01 (`Hit`, `Query`) |
| **Axioms** | A16, A19 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/20 §2` |

## Goal

A deterministic tokenizer (whitespace + punctuation split, lowercase) and the
varint delta-encoded postings list encoding that the inverted index will store.
Both must have byte-exact test coverage before the index layer builds on them.

## Build (checklist of concrete, code-level steps)

- [ ] `tokenizer.rs`:
  - `fn tokenize(text: &str) -> Vec<String>`: split on ASCII whitespace +
    `!"#$%&'()*+,-./:;<=>?@[\]^_{|}~`; lowercase; filter empty tokens;
    no stemming at this stage
  - `fn tokenize_with_positions(text: &str) -> Vec<(String, u32)>`: returns
    `(token, byte_offset)` — needed for ColBERT MaxSim in Pipeline strategy
  - stopword filtering is opt-in via a `StopwordSet` parameter (default: empty,
    no stopwords removed by default — do not remove words that may be significant
    in corpus-specific queries)
- [ ] `inverted.rs` — postings encoding:
  - `fn encode_postings(doc_ids: &[u32]) -> Vec<u8>`: delta-encode sorted doc_ids
    then varint-encode each delta (standard LEB128 encoding); return the raw bytes
  - `fn decode_postings(bytes: &[u8]) -> Result<Vec<u32>, CalyxError>`: inverse;
    `CALYX_SEXTANT_POSTINGS_CORRUPT` if bytes are malformed
  - `fn compress_block(postings: &[u8]) -> Vec<u8>`: zstd level-3 compress the
    encoded bytes (use `zstd` crate); no-op (return as-is) if `postings.len() < 64`
    (threshold below which compression expands)
  - `fn decompress_block(bytes: &[u8], was_compressed: bool) -> Result<Vec<u8>, CalyxError>`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `tokenize("Hello, World! foo")` → `["hello", "world", "foo"]`
- [ ] unit: `tokenize("")` → `[]`
- [ ] unit: `encode_postings([1, 3, 7])` → known byte sequence (compute once:
      deltas=[1,2,4], varint=[0x01, 0x02, 0x04]) — assert exact bytes
- [ ] unit: `decode_postings(encode_postings(xs)) == xs` for `xs=[1,3,7,100,1000]`
- [ ] proptest: `decode_postings(encode_postings(xs)) == xs` for any sorted `Vec<u32>`
- [ ] proptest: `encode_postings` output is shorter than `4 * xs.len()` bytes for
      typical doc_id ranges (within-corpus monotonic IDs have small deltas)
- [ ] edge: `encode_postings([])` → `[]`; `decode_postings([])` → `Ok([])`
- [ ] edge: `decode_postings` on truncated bytes → `CALYX_SEXTANT_POSTINGS_CORRUPT`
- [ ] fail-closed: unsorted input to `encode_postings` → `CALYX_SEXTANT_POSTINGS_NOT_SORTED`
      (caller must sort; enforce at the boundary)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant tokenizer postings -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant tokenizer postings -- --nocapture 2>&1`
- **Prove:** test prints `encode([1,3,7])=010204 decode_ok=true round_trip_ok=true`;
  exact hex bytes `010204` confirm the varint encoding is correct

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH25 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
