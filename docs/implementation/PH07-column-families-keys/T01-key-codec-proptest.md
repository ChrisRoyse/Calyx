# PH07 · T01 — Key codec proptest suite (all CFs)

| Field | Value |
|---|---|
| **Phase** | PH07 — Column families + key encoding |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/cf/key.rs` (≤500), `crates/calyx-aster/src/cf/tests.rs` (≤500) |
| **Depends on** | PH06 T02 (SST ordering established) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/04 §4` |

## Goal

Prove all CF key codecs produce big-endian byte sequences that sort in the
intended lexicographic order: `CxId` keys sort by 16-byte blake3 prefix;
`(ScalarId, CxId)` keys sort scalar-first then by cx; `ledger` keys sort by seq
ascending; `anchor` keys sort by CxId then AnchorKind. Also prove that the
`prefix_range` / `KeyRange::contains` logic correctly includes all keys under a
prefix and excludes the next prefix. These are the ordering invariants downstream
range scans and prefix reads depend on.

## Build (checklist of concrete, code-level steps)

- [x] Add proptest for `base_key`/`slot_key`: for any two `CxId` values `a < b`
  (lexicographic on their 16 bytes), `base_key(a) < base_key(b)`.
- [x] Add proptest for `ledger_key`: `seq1 < seq2` → `ledger_key(seq1) <
  ledger_key(seq2)` (big-endian u64 ordering).
- [x] Add proptest for `scalar_key`: `(s1, cx1) < (s2, cx2)` in the natural
  product order → `scalar_key(s1, cx1) < scalar_key(s2, cx2)`.
- [x] Add proptest for `xterm_key`: for fixed `CxId`, `(a1,b1,kind1) <
  (a2,b2,kind2)` in the natural tuple order → `xterm_key(cx, a1, b1, kind1) <
  xterm_key(cx, a2, b2, kind2)`.
- [x] Add test for `anchor_key`: `AnchorKind::TestPass` sorts before
  `AnchorKind::Label("z")` for the same `CxId`.
- [x] Add proptest for `prefix_range` / `KeyRange::contains`: for any `prefix` of
  length 1..=16, all keys starting with `prefix` satisfy `range.contains(key)`;
  a key with the next byte after the prefix does not.
- [x] Add test for `ledger_range(0, 10)`: contains keys `[0,9]`, does not contain
  key `10` or `u64::MAX`.
- [x] Add test: `cx_prefix_range(cx_id)` contains `base_key(cx_id)` and
  `slot_key(cx_id)`, and does not contain `base_key` of a CxId whose first 16
  bytes are the prefix upper bound.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `ledger_key(1) == [0,0,0,0,0,0,0,1]` (big-endian u64 1); `ledger_key
  (u64::MAX) == [0xff; 8]`.
- [x] proptest: all ordering properties for all CF key types (see above).
- [x] edge (≥3): (1) `CxId` with all-zero bytes: `base_key` is 16 zero bytes;
  `prefix_range` is `[start=[0;16], end=Some([0,0,..,0,1])]`; (2) `CxId` with
  all-`0xff` bytes: `prefix_range.end == None` (unbounded); (3) `AnchorKind::Label`
  with empty string sorts after all fixed-width kinds.
- [x] fail-closed: `verify_cx_hash_prefix` with mismatching hash → error code is
  `"CALYX_ASTER_CORRUPT_SHARD"`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-aster cf` output on aiwonder.
- **Readback:** `cargo test -p calyx-aster cf::tests -- --nocapture 2>&1 | tail -10`
- **Prove:** All proptest cases pass (≥100 per invariant); unit tests print the
  known golden byte values and assert. Screenshot posted to PH07 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH07 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
