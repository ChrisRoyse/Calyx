# PH60 · T01 — `VaultKey`: per-vault key derivation (HKDF) + AES-256-GCM context

| Field | Value |
|---|---|
| **Phase** | PH60 — Encryption at rest/in transit + tenant isolation |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault/key.rs` (≤500) |
| **Depends on** | — (first card; depends on PH09 `VaultId` + PH03 error catalog) |
| **Axioms** | A33, A16 |
| **PRD** | `dbprdplans/30 §2` (Crypto at rest axis) |

## Goal

Implement the `VaultKey` type that derives a unique AES-256-GCM encryption key
per vault from host-provided key material using HKDF-SHA-256. This is the innermost
cryptographic layer of the defense-in-depth tenant isolation stack (key +
keyspace + grant — `30 §2`). Embedded vaults use the host application's own key;
`calyxd` server vaults receive their key via a provisioned secret. Key material is
zeroized on drop and never cloned into a static.

## Build (checklist of concrete, code-level steps)

- [ ] Add crate deps: `hkdf`, `sha2`, `aes-gcm`, `zeroize` — pinned versions in
  `Cargo.toml`.
- [ ] `struct VaultKey { inner: zeroize::Zeroizing<[u8; 32]> }` — derive `Clone`
  is intentionally omitted; implement `Drop` via `Zeroizing`.
- [ ] `impl VaultKey { pub fn derive(master: &[u8], vault_id: &VaultId) -> Result<Self> }` —
  `HKDF-SHA-256` with `ikm = master`, `salt = b"calyx-vault-key-v1"`,
  `info = vault_id.as_bytes()`; output 32 bytes; return `CALYX_VAULT_KEY_MISSING`
  if `master` is empty.
- [ ] `pub fn from_raw(bytes: [u8; 32]) -> Self` — for tests and embedded vaults that
  supply a pre-derived key directly; wraps into `Zeroizing`.
- [ ] `pub fn aes_gcm_key(&self) -> &aes_gcm::Key<aes_gcm::Aes256Gcm>` — zero-copy
  borrow of the inner 32 bytes as an AES-256-GCM key reference.
- [ ] `pub fn encrypt(&self, nonce: &[u8; 12], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>>`
  — AES-256-GCM encrypt; appends 16-byte tag; returns `CALYX_ENCRYPTION_FAILED` on
  cipher error.
- [ ] `pub fn decrypt(&self, nonce: &[u8; 12], ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>>`
  — AES-256-GCM decrypt + verify tag; returns `CALYX_DECRYPTION_FAILED` (not a
  silent zero-fill — fail closed, A16).
- [ ] Add `CALYX_VAULT_KEY_MISSING`, `CALYX_ENCRYPTION_FAILED`, `CALYX_DECRYPTION_FAILED`
  to `calyx-core/src/error.rs` error catalog if not already present.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `VaultKey::derive(MASTER, VAULT_ID)` with fixed seeds → assert 32-byte
  output equals a hard-coded golden constant (HKDF determinism).
- [ ] unit: `encrypt` then `decrypt` round-trip → plaintext recovered byte-exact.
- [ ] unit: `encrypt` with fixed `(key, nonce, aad, plaintext)` → ciphertext equals
  hard-coded golden (regression for AES-GCM determinism with fixed nonce).
- [ ] proptest: `∀ plaintext ∈ Vec<u8>[0..4096]`: `decrypt(encrypt(pt)) == pt`.
- [ ] edge (≥3): empty master → `CALYX_VAULT_KEY_MISSING`; empty plaintext → encrypts
  to 16-byte tag-only ciphertext (GCM allows empty plaintext); wrong AAD on decrypt →
  `CALYX_DECRYPTION_FAILED`; truncated ciphertext (< 16 bytes) → `CALYX_DECRYPTION_FAILED`.
- [ ] fail-closed: flipped tag byte → `CALYX_DECRYPTION_FAILED` (not garbage plaintext).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** compiled test binary; a synthetic vault with known master key + vault_id.
- **Readback:** `cargo test -p calyx-aster vault_key -- --nocapture 2>&1` must print
  the golden 32-byte derived key and the golden ciphertext hex; assert both match
  hard-coded constants.
- **Prove:** before: no `VaultKey` type; after: golden test passes; flipping one
  ciphertext byte returns `CALYX_DECRYPTION_FAILED`; drop of `VaultKey` zeroes
  inner bytes with a heap-live destructor probe, never by reading a stack pointer
  after `drop(key)`.

  ```rust
  let raw: *mut VaultKey = Box::into_raw(Box::new(key));
  let byte_ptr = unsafe { (*raw).inner.as_ptr() };
  unsafe { std::ptr::drop_in_place(raw) }; // runs Zeroizing::drop, does not free
  let after = unsafe { std::slice::from_raw_parts(byte_ptr, 32) };
  assert_eq!(after, &[0u8; 32]);
  unsafe { drop(Box::from_raw(raw as *mut std::mem::ManuallyDrop<VaultKey>)) };
  ```

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH60 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
