# PH60 · T06 — ZFS encryption probe + operator-guidance strings + `lens_store` cross-vault guard

| Field | Value |
|---|---|
| **Phase** | PH60 — Encryption at rest/in transit + tenant isolation |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/security/zfs.rs` (≤500), `crates/calyx-aster/src/security/lens_store.rs` (≤500) |
| **Depends on** | T01 (`VaultKey`) · PH18 (LensId content-addressing) |
| **Axioms** | A33, A16 |
| **PRD** | `dbprdplans/30 §2` (Crypto at rest — ZFS native encryption); `dbprdplans/30 §2` (AuthZ — vectors never cross tenant) |

## Goal

Implement two security utilities that provide the outermost ZFS encryption layer
and the lens-store cross-vault guard. `zfs.rs` probes whether the calyx ZFS dataset
has encryption enabled, returns a structured `ZfsEncryptionStatus`, and emits
operator-guidance strings if encryption is absent — but never panics or fails the
process when ZFS is unavailable (dev machines without ZFS must still work). The ZFS
dataset encryption step itself is operator/sudo-gated and is documented here rather
than automated.
`lens_store.rs` asserts that the shared lens weight store never materialises
constellation vectors from another vault into the calling vault's address space,
returning `CALYX_LENS_CROSS_VAULT` if the guard is violated.

> **Operator note (ZFS):** enabling encryption requires sudo:
> `zfs create -o encryption=aes-256-gcm -o keylocation=prompt -o keyformat=passphrase tank/calyx`
> This is NOT automated by Calyx code. The Rust probe only reads status and guides.

## Build (checklist of concrete, code-level steps)

### `zfs.rs`

- [ ] `enum ZfsEncryptionStatus { Enabled { algorithm: String }, Disabled, ZfsNotAvailable, DatasetNotFound { dataset: String } }` —
  `Display` impl gives human-readable guidance.
- [ ] `fn probe_zfs_encryption(dataset: &str) -> ZfsEncryptionStatus` — runs
  `zfs get -H -o value encryption <dataset>` via `std::process::Command`; parses
  stdout; if the command fails (ZFS not found, EPERM, etc.) returns
  `ZfsNotAvailable` (never `unwrap` / never panics).
- [ ] `fn operator_guidance(status: &ZfsEncryptionStatus) -> Option<&'static str>` —
  returns `None` if `Enabled`; returns a human-readable sudo command suggestion string
  for `Disabled` and `DatasetNotFound`; returns `None` for `ZfsNotAvailable` (the
  probe couldn't run — not an error condition during dev).
- [ ] `fn assert_encrypted_or_warn(dataset: &str) -> ZfsEncryptionStatus` — calls
  `probe_zfs_encryption`; logs a `WARN` (not a panic or error) if not `Enabled`;
  returns the status for callers to record in the vault manifest.

### `lens_store.rs`

- [ ] `struct LensStoreGuard { requesting_vault: VaultId }`.
- [ ] `pub fn assert_no_cross_vault_vector(guard: &LensStoreGuard, embedding_vault: VaultId) -> Result<()>` —
  if `embedding_vault != guard.requesting_vault` returns `CALYX_LENS_CROSS_VAULT`;
  this is called at every point where a stored vector is about to be copied or
  returned to a caller (the lens weights themselves are vault-agnostic; only the
  materialised vectors are vault-scoped).
- [ ] `fn content_id_is_vault_agnostic(lens_id: &LensId) -> bool` — always returns
  `true` (lens weights are content-addressed and shared; this is a compile-time
  documentation assertion, not a runtime check).
- [ ] Add `CALYX_LENS_CROSS_VAULT` to `calyx-core/src/error.rs`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit (`zfs.rs`): `probe_zfs_encryption` on a non-existent dataset path →
  returns `ZfsNotAvailable` or `DatasetNotFound` (does not panic); assert
  `operator_guidance` returns `Some(s)` with non-empty `s`.
- [ ] unit (`lens_store.rs`): `assert_no_cross_vault_vector(guard_A, vault_A)` →
  `Ok(())`; `assert_no_cross_vault_vector(guard_A, vault_B)` →
  `CALYX_LENS_CROSS_VAULT`.
- [ ] unit: `content_id_is_vault_agnostic` returns `true` for any `LensId` (trivial
  but documents the invariant).
- [ ] edge (≥3): ZFS command returns exit code 1 (permission denied) → `ZfsNotAvailable`
  (not a panic); `probe_zfs_encryption("tank/calyx")` on a machine with ZFS and
  encryption disabled → `Disabled`; same vault in guard and in embedding →
  `Ok(())`.
- [ ] fail-closed (`lens_store`): cross-vault vector access always returns the
  structured error; it is never silently allowed regardless of any config flag.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `zfs get encryption tank/calyx` output on aiwonder (sudo-gated); unit test
  for `lens_store.rs` uses synthetic `VaultId`s.
- **Readback:**
  - ZFS: `cargo test -p calyx-aster zfs -- --nocapture 2>&1` prints the
    `ZfsEncryptionStatus` variant; on aiwonder with ZFS present, run
    `zfs get encryption tank/calyx` (sudo) and confirm Calyx's probe matches.
  - Lens store: `cargo test -p calyx-aster lens_store -- --nocapture 2>&1` prints
    `CALYX_LENS_CROSS_VAULT` for the cross-vault case and `Ok(())` for same-vault.
- **Prove:** before: no ZFS probe; after: `ZfsEncryptionStatus` struct returned
  (never a panic); cross-vault vector access blocked at the guard before bytes are
  copied; operator-guidance string is non-empty for non-encrypted status.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines each (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH60 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
