# PH60 · T02 — `KeyspaceGuard` + `VaultWriteLock`: per-vault key-prefix + cross-vault read block

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
(key + **keyspace** + grant — `30 §2`). `KeyspaceGuard` is only the stateless
key codec; the write lock is a separate per-vault shared object so two contexts
for the same vault cannot accidentally lock different mutexes.

## Build (checklist of concrete, code-level steps)

- [ ] `fn vault_prefix(vault_id: &VaultId) -> [u8; 16]` — exact full-ULID bytes
  (`*vault_id.as_bytes()`) used as the leading prefix on every CF key for that
  vault; deterministic and collision-free across distinct `VaultId`s because no
  bits are truncated or hashed.
- [ ] `struct KeyspaceGuard { vault_id: VaultId, prefix: [u8; 16] }` — constructed
  from a `VaultId`; carries no mutable state or lock; `Clone + Copy` allowed since
  it holds only value bytes and no secret material.
- [ ] `impl KeyspaceGuard { pub fn new(vault_id: VaultId) -> Self }` — derives prefix.
- [ ] `pub fn encode_key(&self, cf: CfName, user_key: &[u8]) -> Vec<u8>` — prepends
  `prefix ‖ cf_byte ‖ user_key`; this is the only path that produces a storable CF key
  for a vault-scoped operation.
- [ ] `pub fn decode_key<'a>(&self, raw: &'a [u8]) -> Result<(CfName, &'a [u8])>` —
  verifies the leading 16 bytes equal `self.prefix`; if not, returns
  `CALYX_VAULT_KEYSPACE_MISMATCH` (fail closed — never silently returns another vault's
  key, A16).
- [ ] `pub fn owns_key(&self, raw: &[u8]) -> bool` — fast prefix check without
  allocating; used in range-scan filters.
- [ ] `struct VaultWriteLock` — a standalone, non-`Copy`, single-instance per-vault
  `Mutex<()>` owner (shared by `Arc` where multiple contexts need the same vault);
  acquired before any WAL group-commit that touches this vault's keyspace.
- [ ] `impl VaultWriteLock { pub fn lock(&self) -> VaultWriteLockGuard<'_> }` —
  acquires that vault's shared mutex and returns a guard that releases on drop.
  `KeyspaceGuard` must not own or manufacture this mutex.
- [ ] Add `CALYX_VAULT_KEYSPACE_MISMATCH` to `calyx-core/src/error.rs`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `encode_key` for vault-a and vault-b with the same `user_key` produce
  byte-different encoded keys (assert `key_a != key_b`).
- [ ] unit: `decode_key` on a key encoded by vault-a's guard → succeeds; same raw
  bytes handed to vault-b's guard → `CALYX_VAULT_KEYSPACE_MISMATCH`.
- [ ] unit: `owns_key` returns `true` for own-prefix and `false` for a neighbouring
  vault's prefix; check boundary byte at offset 15.
- [ ] proptest: `∀ vault_id, cf, user_key`: `decode_key(encode_key(cf, user_key)) == (cf, user_key)`.
- [ ] edge (≥3): empty `user_key` (encoded length is 17 bytes: 16-byte prefix +
  CF byte); raw key exactly 16 bytes (prefix only, missing CF byte) fails closed;
  `user_key` containing all-zero bytes must not alias another vault's prefix; two
  distinct `VaultId`s sharing the same low or high 64 bits still produce different
  16-byte prefixes.
- [ ] fail-closed: raw key shorter than 17 bytes → `CALYX_VAULT_KEYSPACE_MISMATCH`;
  raw key with correct prefix length but wrong prefix → `CALYX_VAULT_KEYSPACE_MISMATCH`.
- [ ] concurrency: two handles to the same shared `VaultWriteLock` exclude each other;
  two independently constructed locks for the same `VaultId` are not acceptable
  production wiring and must be caught in the integration card.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** compiled test binary with two synthetic vaults using known `VaultId`s.
- **Readback:** `cargo test -p calyx-aster keyspace -- --nocapture 2>&1` prints the
  encoded keys for vault-a and vault-b for an identical user key and confirms they
  differ; prints `CALYX_VAULT_KEYSPACE_MISMATCH` for the cross-vault decode attempt.
- **Prove:** before: no keyspace prefix enforcement; after: vault-a's encoded key
  first 16 bytes differ from vault-b's by `xxd` inspection; `decode_key` with mismatched
  guard returns the structured error code, not the user key.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH60 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
