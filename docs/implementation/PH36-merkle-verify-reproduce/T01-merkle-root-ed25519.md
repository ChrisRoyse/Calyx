# PH36 · T01 — `merkle.rs`: range root + leaf hashing + Ed25519-signed export

| Field | Value |
|---|---|
| **Phase** | PH36 — Merkle checkpoints + verify_chain + reproduce() |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/merkle.rs` (≤500) |
| **Depends on** | — (first card; depends on PH35 `LedgerEntry` + codec) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §2`, `11 §5` |

## Status

DONE / FSV-signed-off on aiwonder for #249. Implemented in
`crates/calyx-ledger/src/merkle.rs` plus `calyx merkle-root` in
`crates/calyx-cli/src/merkle.rs`. Evidence root:
`/home/croyse/calyx/data/fsv-issue249-merkle-root-ed25519-20260609`.

Readback facts:
- 4-row synthetic ledger CF writes rows `0..4` after a before-read of zero rows.
- `root_0_4` = `c7e306ce6a90128afebd835f75e71f96485e12ea7a8dff5abaee40ecdebdb4da`.
- 4-hash golden root = `522a628f043f5aaebab28ea89a73cc0597d209943e8b984c09213852b3afe814`.
- `calyx merkle-root --ledger <dir> --range 0..4`, `--vault <dir>`, and
  `CALYX_LEDGER_DIR=<dir> calyx merkle-root --range 0..4` all print the JSON
  root byte-for-byte.
- Ed25519 signature verification round-trips, tampered roots fail verification,
  and a missing row fails closed with `CALYX_LEDGER_CORRUPT`.

Post-sweep follow-ups:
- #347: bind `range_start` and `range_end` into the signed Merkle export payload.
- #348: make `calyx merkle-root --vault` read the real Aster Ledger CF, or fail
  closed instead of creating/reading a side ledger directory.

## Goal

Build a Merkle tree over a contiguous range of ledger entries `[seq_a, seq_b)`.
Each leaf is `blake3(entry_hash)` (the `entry_hash` is already the hash of all
entry fields, so this is a hash-of-hash for the tree). The root can optionally
be signed with an Ed25519 key for tamper-evident export. This implements the
"periodic Merkle checkpoint" from `11 §2` and the `merkle_root(vault, range)`
API from `11 §5`.

## Build (checklist of concrete, code-level steps)

- [ ] `fn leaf_hash(entry_hash: &[u8; 32]) -> [u8; 32]` — `blake3(b"leaf" ‖ entry_hash)`.
- [ ] `fn combine_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32]` —
  `blake3(b"node" ‖ left ‖ right)` (domain-separated).
- [ ] `fn merkle_root_of_hashes(entry_hashes: &[[u8; 32]]) -> [u8; 32]` —
  bottom-up binary tree; if `len == 0` return `[0u8; 32]`; if `len` is odd,
  duplicate the last leaf (standard padding); iterative, not recursive.
- [ ] `pub fn merkle_root(cf_reader: &dyn LedgerCfReader, range: &KeyRange) -> Result<[u8; 32]>` —
  reads all `entry_hash` fields in the range (uses `decode_header` fast-path
  from PH35-T02 plus an `entry_hash` offset decode); returns the Merkle root.
- [ ] `struct MerkleExportBundle { range_start: u64, range_end: u64, root: [u8; 32], signature: Option<[u8; 64]>, signer_pubkey: Option<[u8; 32]> }` —
  serialise to canonical JSON (serde).
- [ ] `fn sign_root(root: &[u8; 32], signing_key: &[u8; 32]) -> [u8; 64]` —
  Ed25519 signature over `b"calyx-ledger-root-v1" ‖ root` using the `ed25519-dalek`
  crate; `signing_key` is a 32-byte seed.
- [ ] `fn verify_signature(bundle: &MerkleExportBundle) -> bool` — re-derive
  verifying key from stored `signer_pubkey`; verify signature over `root`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `merkle_root_of_hashes(&[[0u8;32]; 1])` → assert equals golden
  `blake3(b"leaf" ‖ [0u8;32])` (single-leaf tree).
- [ ] unit: `merkle_root_of_hashes(&[[0u8;32]; 2])` → assert equals
  `combine_hash(leaf_hash([0;32]), leaf_hash([0;32]))`.
- [ ] unit: 4-entry range with known `entry_hash` values → assert Merkle root
  matches a hard-coded golden constant (regression test for tree stability).
- [ ] unit: sign a root with a fixed 32-byte seed → verify round-trip;
  assert `verify_signature` returns `true`; flip one root byte → `false`.
- [ ] edge (≥3): empty range → root `[0u8;32]`; single entry; 3 entries (odd
  — padding required); 1000 entries (performance must be sub-second on aiwonder).
- [ ] fail-closed: `verify_signature` with `signature=None` → returns `false`
  (not a panic); `merkle_root` over a range with a missing row →
  `CALYX_LEDGER_CORRUPT`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** compiled test binary + a synthetic ledger CF with 4 known entries
- **Readback:** `cargo test -p calyx-ledger -- --nocapture merkle_golden 2>&1`
  prints the 32-byte Merkle root; assert equals the hard-coded golden constant.
  `calyx merkle-root --vault test --range 0..4` prints the same 32-byte hex;
  confirm byte-exact match.
- **Prove:** before: no Merkle function; after: golden test passes; flipping one
  entry's `entry_hash` byte changes the root; `verify_signature` round-trip
  succeeds with known seed.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH36 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
