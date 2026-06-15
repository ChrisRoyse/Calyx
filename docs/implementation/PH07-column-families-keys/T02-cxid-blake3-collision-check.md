# PH07 · T02 — CxId blake3 prefix + collision check

| Field | Value |
|---|---|
| **Phase** | PH07 — Column families + key encoding |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/cf/key.rs` (≤500), `crates/calyx-aster/src/cf/tests.rs` (≤500) |
| **Depends on** | T01 (key ordering established) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/04 §4` |

## Goal

Prove that `full_content_hash` + `cx_id_from_full_hash` + `verify_cx_hash_prefix`
form a correct, collision-detecting content-addressing chain:
`cx_id = blake3(input ‖ panel_version ‖ salt)[0..16]`; write stores the full 32B
hash; read verifies the stored CxId matches; any mismatch returns
`CALYX_ASTER_CORRUPT_SHARD`. This implements the PRD `04 §5` dedup short-circuit
(same bytes → same CxId → no-op on second ingest).

## Build (checklist of concrete, code-level steps)

- [x] Write a test that `full_content_hash([b"input", &7u32.to_be_bytes(),
  b"salt"])` produces a stable 32-byte hex digest (compute the expected value once
  and hard-code it as the golden).
- [x] Write a test that `cx_id_from_full_hash(&full_hash).as_bytes() ==
  &full_hash[0..16]`.
- [x] Write a test that `verify_cx_hash_prefix(cx_id, &full_hash)` returns `Ok(())`
  when the first 16 bytes match, and returns
  `Err(code == "CALYX_ASTER_CORRUPT_SHARD")` when they don't.
- [x] Add a proptest: for any `(input, panel_version, salt)`, computing the hash
  twice with the same inputs produces the same `CxId` (deterministic).
- [x] Add a test proving idempotent ingest: the same `input_bytes` + `panel_version`
  + `vault_salt` always produces the same `CxId`; two different `input_bytes`
  produce different `CxId` values with overwhelming probability (seed RNG, use
  two distinct 32-byte random inputs).
- [x] Verify `full_content_hash` uses length-delimited encoding:
  `hasher.update(&(part.len() as u64).to_be_bytes()); hasher.update(part);`
  for each part — this is already implemented; add a test that confirms
  `hash([b"ab", b"c"]) != hash([b"a", b"bc"])` (length-prefix prevents extension
  ambiguity).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `full_content_hash([b"hello"])` == known 32-byte hex golden (compute
  once and assert; seed: the input itself is deterministic).
- [x] unit: `verify_cx_hash_prefix` with matching prefix → Ok; with first byte
  flipped → `CALYX_ASTER_CORRUPT_SHARD`.
- [x] proptest: determinism — same inputs always same `CxId`.
- [x] edge (≥3): (1) empty part list → valid hash (all-zero extension); (2) parts
  of length 0 contribute length prefix only; (3) extension ambiguity test:
  `hash([b"ab", b"c"]) != hash([b"a", b"bc"])`.
- [x] fail-closed: `verify_cx_hash_prefix(cx_id, full_hash)` with any single-byte
  mutation in `full_hash[0..16]` → `CALYX_ASTER_CORRUPT_SHARD`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Blake3 digest of a known test input.
- **Readback:**
  ```
  cargo test -p calyx-aster cf::tests::full_content_hash_golden -- --nocapture 2>&1
  ```
- **Prove:** The printed 64-character hex matches the hard-coded golden in the
  test source. Any change to the input bytes produces a different hex value.
  Screenshot of passing test output posted to PH07 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH07 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
