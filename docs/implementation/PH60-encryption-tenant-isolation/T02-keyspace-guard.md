# PH60 · T02 — `KeyspaceGuard`: per-vault key-prefix + write lock + cross-vault read block

| Field | Value |
|---|---|
| **Phase** | PH60 — Encryption at rest/in transit + tenant isolation |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault/keyspace.rs` (≤500) |
| **Depends on** | T01 (`VaultKey`) · PH07 (CF key encoding) |
| **Axioms** | A33, A16 |
| **PRD** | `dbprdplans/30 §3` (Tenant isolation — per-vault keyspace) |

## Goal

Implement `KeyspaceGuard` which enforces that every CF key written or read by a
vault operation is prefixed with that vault's unique `VaultId` prefix, making it
structurally impossible for one vault's read path to accidentally access another
vault's key range. This is the second layer of defense-in-depth tenant isolation
(key + **keyspace** + grant — `30 §2`). The write lock prevents concurrent
cross-vault mutations from interleaving keys.

## Build (checklist of concrete, code-level steps)

- [ ] `fn vault_prefix(vault_id: &VaultId) -> [u8; 8]` — big-endian 8-byte encoding
  of `vault_id.as_u64()` used as the leading prefix on every CF key for that vault;
  deterministic and collision-free across distinct `VaultId`s.
- [ ] `struct KeyspaceGuard { vault_id: VaultId, prefix: [u8; 8] }` — constructed
  from a `VaultId`; carries no mutable state; `Clone + Copy` allowed since it holds
  no secret material.
- [ ] `impl KeyspaceGuard { pub fn new(vault_id: VaultId) -> Self }` — derives prefix.
- [ ] `pub fn encode_key(&self, cf: CfName, user_key: &[u8]) -> Vec<u8>` — prepends
  `prefix ‖ cf_byte ‖ user_key`; this is the only path that produces a storable CF key
  for a vault-scoped operation.
- [ ] `pub fn decode_key<'a>(&self, raw: &'a [u8]) -> Result<(CfName, &'a [u8])>` —
  verifies the leading 8 bytes equal `self.prefix`; if not, returns
  `CALYX_VAULT_KEYSPACE_MISMATCH` (fail closed — never silently returns another vault's
  key, A16).
- [ ] `pub fn owns_key(&self, raw: &[u8]) -> bool` — fast prefix check without
  allocating; used in range-scan filters.
- [ ] `struct VaultWriteLock` — a `Mutex<()>`-backed RAII guard; acquired before any
  WAL group-commit that touches this vault's keyspace; released on drop.
- [ ] `impl KeyspaceGuard { pub fn write_lock(&self) -> VaultWriteLockGuard }` —
  acquires the per-vault `Mutex`; returns a guard that releases on drop.
- [ ] Add `CALYX_VAULT_KEYSPACE_MISMATCH` to `calyx-core/src/error.rs`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `encode_key` for vault-a and vault-b with the same `user_key` produce
  byte-different encoded keys (assert `key_a != key_b`).
- [ ] unit: `decode_key` on a key encoded by vault-a's guard → succeeds; same raw
  bytes handed to vault-b's guard → `CALYX_VAULT_KEYSPACE_MISMATCH`.
- [ ] unit: `owns_key` returns `true` for own-prefix and `false` for a neighbouring
  vault's prefix; check boundary byte at offset 7.
- [ ] proptest: `∀ vault_id, cf, user_key`: `decode_key(encode_key(cf, user_key)) == (cf, user_key)`.
- [ ] edge (≥3): empty `user_key`; key exactly 8 bytes (prefix only, no user key);
  `user_key` containing all-zero bytes (must not alias another vault's prefix);
  two `VaultId`s whose `as_u64()` differ only in the last byte.
- [ ] fail-closed: raw key shorter than 9 bytes → `CALYX_VAULT_KEYSPACE_MISMATCH`;
  raw key with correct prefix length but wrong prefix → `CALYX_VAULT_KEYSPACE_MISMATCH`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** compiled test binary with two synthetic vaults using known `VaultId`s.
- **Readback:** `cargo test -p calyx-aster keyspace -- --nocapture 2>&1` prints the
  encoded keys for vault-a and vault-b for an identical user key and confirms they
  differ; prints `CALYX_VAULT_KEYSPACE_MISMATCH` for the cross-vault decode attempt.
- **Prove:** before: no keyspace prefix enforcement; after: vault-a's encoded key
  prefix differs from vault-b's by `xxd` inspection; `decode_key` with mismatched
  guard returns the structured error code, not the user key.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH60 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
