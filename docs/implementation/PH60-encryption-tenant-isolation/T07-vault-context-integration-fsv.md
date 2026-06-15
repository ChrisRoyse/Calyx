# PH60 · T07 — Integration: `VaultContext` wires key + keyspace + write lock + grant + quota; cross-vault FSV test

| Field | Value |
|---|---|
| **Phase** | PH60 — Encryption at rest/in transit + tenant isolation |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault/mod.rs` (≤500), `crates/calyx-aster/src/tests/ph60_integration.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05, T06 |
| **Axioms** | A33, A16 |
| **PRD** | `dbprdplans/30 §3` (FSV: cross-vault read denied+audited) |

## Goal

Wire all PH60 components into a single `VaultContext` aggregate that every
vault-scoped operation receives. Then run a full integration FSV test proving both
phase exit gates: (1) cross-vault read attempt without a grant returns
`CALYX_VAULT_ACCESS_DENIED` and leaves an audit record; (2) ZFS encryption status
is probed and logged in the vault manifest. The integration test exercises the
complete defense-in-depth stack (key + keyspace + grant) end-to-end on aiwonder.

## Build (checklist of concrete, code-level steps)

- [ ] `struct VaultContext { vault_id: VaultId, key: VaultKey, keyspace: KeyspaceGuard, write_lock: Arc<VaultWriteLock>, grants: Arc<RwLock<GrantStore>>, quota: QuotaGuard, zfs_status: ZfsEncryptionStatus }` —
  the single aggregate every storage operation receives.
- [ ] `impl VaultContext { pub fn new(vault_id: VaultId, master_key: &[u8], config: QuotaConfig, zfs_dataset: &str) -> Result<Self> }` —
  derives `VaultKey` via HKDF; builds `KeyspaceGuard`; obtains the shared per-vault
  `VaultWriteLock` instance; constructs empty `GrantStore`; constructs `QuotaGuard`;
  probes ZFS; returns `CALYX_VAULT_KEY_MISSING` if `master_key` empty.
- [ ] `pub fn check_cross_vault_read(&self, dst: VaultId, actor: ActorId, now: Timestamp) -> Result<()>` —
  calls `grant_store.check_grant(self.vault_id, dst, actor, now)`; the `CALYX_VAULT_ACCESS_DENIED`
  error propagates unchanged.
- [ ] `pub fn encode_key(&self, cf: CfName, user_key: &[u8]) -> Vec<u8>` — delegates
  to `self.keyspace.encode_key(cf, user_key)`.
- [ ] `pub fn with_write_lock<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T>` —
  acquires `self.write_lock.lock()` around WAL group-commit work. Do not create a
  fresh `VaultWriteLock` inside this method.
- [ ] `pub fn decrypt_value(&self, nonce: &[u8; 12], ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>>` —
  delegates to `self.key.decrypt(...)`.
- [ ] `pub fn encrypt_value(&self, nonce: &[u8; 12], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>>` —
  delegates to `self.key.encrypt(...)`.
- [ ] Re-export all PH60 types from `vault/mod.rs`; update `calyx-aster/src/lib.rs` to
  expose `pub mod vault`.

### Integration test (`tests/ph60_integration.rs`)

- [ ] Build two `VaultContext`s (`ctx_a`, `ctx_b`) with distinct `VaultId`s and
  distinct derived keys.
- [ ] Ingest a synthetic constellation into vault-a using `ctx_a.encode_key` +
  `ctx_a.encrypt_value`; read it back via `ctx_a.decode_key` + `ctx_a.decrypt_value`
  and assert plaintext matches.
- [ ] Attempt to read vault-b's CF key range using `ctx_a` (pass `ctx_a.keyspace` to
  `decode_key` with a vault-b-prefixed raw key) → assert `CALYX_VAULT_KEYSPACE_MISMATCH`.
- [ ] Attempt `ctx_a.check_cross_vault_read(vault_b_id, actor, T)` with no grant →
  assert `CALYX_VAULT_ACCESS_DENIED`; assert `ctx_a.grants.read().audit_events(1)`
  contains a `Denied` record for `(vault_a, vault_b, actor)`.
- [ ] Add grant `(vault_a, vault_b, actor)` to `ctx_a.grants`; re-run
  `check_cross_vault_read` → assert `Ok(())`.
- [ ] Attempt `ctx_a.encrypt_value` then decrypt with `ctx_b.key` → assert
  `CALYX_DECRYPTION_FAILED` (different vault → different derived key → tag mismatch).
- [ ] Construct two contexts for the same `VaultId` through the production constructor
  and prove they share the same `Arc<VaultWriteLock>` instance (`Arc::ptr_eq`) before
  running a lock-exclusion smoke test.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] integration: full stack as described above; all assertions are hard-coded
  expected values (known `VaultId`s, known master keys, known synthetic constellation
  bytes).
- [ ] unit: `VaultContext::new` with empty master → `CALYX_VAULT_KEY_MISSING`.
- [ ] edge (≥3): two `VaultContext`s with the same master but different `VaultId`s →
  keys are distinct (HKDF info differs); `VaultContext` with ZFS unavailable →
  constructs successfully (ZFS absence is not an error, only a warning); quota
  charge on constructed `VaultContext` → respects configured limits.
- [ ] fail-closed: every denied path returns the structured `CALYX_*` code; no silent
  `Ok(())` on missing grant or mismatched keyspace.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ph60_integration` test binary on aiwonder; audit ring contents printed
  as JSON; ZFS status printed.
- **Readback:**
  1. `cargo test -p calyx-aster ph60_integration -- --nocapture 2>&1` must print:
     - `cross_vault_read without grant = Err(CALYX_VAULT_ACCESS_DENIED)`
     - `audit ring[0] = Denied { src: vault_a, dst: vault_b, actor: ... }`
     - `decrypt with wrong vault key = Err(CALYX_DECRYPTION_FAILED)`
     - `cross_vault_read with grant = Ok(())`
  2. (sudo-gated) On aiwonder: `sudo zfs get encryption tank/calyx` confirms
     `aes-256-gcm`; Calyx's `assert_encrypted_or_warn("tank/calyx")` returns
     `ZfsEncryptionStatus::Enabled { algorithm: "aes-256-gcm" }`.
- **Prove:** before: no tenant isolation; after: cross-vault read blocked (key
  mismatch + keyspace mismatch + grant denial, three independent layers); audit
  ring populated; ZFS encryption status visible in vault manifest.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines each (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH60 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
