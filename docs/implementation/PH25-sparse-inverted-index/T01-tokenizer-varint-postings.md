# PH25 · T01 — Tokenizer + varint postings encoding

| Field | Value |
|---|---|
| **Phase** | PH25 — Sparse lens inverted index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/tokenizer.rs` (≤500; tokenizer + varint postings codec), `crates/calyx-sextant/src/index/inverted.rs` (≤500; in-RAM postings index) |
| **Depends on** | PH24 T01 (`Hit`, `Query`) |
| **Axioms** | A16, A19 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/20 §2` |

## Goal

A deterministic tokenizer (whitespace + punctuation split, lowercase) and the
varint delta-encoded postings list encoding that the inverted index will store.
Both must have byte-exact test coverage before the index layer builds on them.

## Current implementation

Post-sweep #322 makes the postings codec fail closed. The implemented API is:

- `encode_varint_deltas(doc_ids: &[u32]) -> Result<Vec<u8>>`
- `decode_varint_deltas(bytes: &[u8]) -> Result<Vec<u32>>`

The known byte vector is `[1,3,7] -> 010204`; unsorted input returns
`CALYX_SEXTANT_POSTINGS_NOT_SORTED`; malformed/truncated/overflow bytes return
`CALYX_SEXTANT_POSTINGS_CORRUPT`.

## Build (current Stage 4 scope)

- [x] `tokenizer.rs`:
  - `fn tokenize(text: &str) -> Vec<String>`: split on ASCII whitespace +
    `!"#$%&'()*+,-./:;<=>?@[\]^_{|}~`; lowercase; filter empty tokens;
    no stemming at this stage
- [x] `tokenizer.rs` — postings encoding:
  - `fn encode_varint_deltas(doc_ids: &[u32]) -> Result<Vec<u8>>`: delta-encode
    sorted/nondecreasing doc IDs, varint-encode each delta, and reject unsorted
    input before bytes are written
  - `fn decode_varint_deltas(bytes: &[u8]) -> Result<Vec<u32>>`: inverse; reject
    malformed, truncated, varint-overflow, and delta-overflow blocks as
    `CALYX_SEXTANT_POSTINGS_CORRUPT`

Position-aware tokenization and zstd/SPANN compressed postings are not claimed
by the Stage 4 in-RAM sparse slot; compressed postings persistence is deferred
to PH68.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `tokenize("Hello, World! foo")` → `["hello", "world", "foo"]`
- [x] unit: `tokenize("")` → `[]`
- [x] unit: `encode_varint_deltas([1, 3, 7])` → known byte sequence (compute once:
      deltas=[1,2,4], varint=[0x01, 0x02, 0x04]) — assert exact bytes
- [x] unit: `decode_varint_deltas(encode_varint_deltas(xs)) == xs` for
      deterministic vectors
- [x] edge: `encode_varint_deltas([])` → `[]`; `decode_varint_deltas([])` → `Ok([])`
- [x] edge: `decode_varint_deltas` on truncated bytes → `CALYX_SEXTANT_POSTINGS_CORRUPT`
- [x] fail-closed: unsorted input to `encode_varint_deltas` → `CALYX_SEXTANT_POSTINGS_NOT_SORTED`
      (caller must sort; enforce at the boundary)
- [x] fail-closed: varint overflow bytes → `CALYX_SEXTANT_POSTINGS_CORRUPT`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/home/croyse/calyx/data/fsv-issue322-postings-fail-closed-20260608/stage4-readback.json`
- **Readback:** Stage 4 FSV on aiwonder plus `cargo test -p calyx-sextant`
- **Prove:** readback records `varint_hex="010204"`,
  `varint_decoded=[1,3,7]`,
  `postings_unsorted_error="CALYX_SEXTANT_POSTINGS_NOT_SORTED"`, and
  `postings_corrupt_error="CALYX_SEXTANT_POSTINGS_CORRUPT"`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence attached to GitHub issue #322
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
