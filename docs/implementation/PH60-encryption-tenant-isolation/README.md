# PH60 — Encryption at rest/in transit + tenant isolation

**Stage:** S14 — Security & Privacy by Construction  ·  **Crate:** `calyx-aster`, `calyxd`  ·
**PRD roadmap:** P13  ·  **Axioms:** A33, A16

## Objective

Establish per-vault encryption keys, per-vault keyspaces, and a strict default-deny
grant model so that one vault = one tenant boundary with no possibility of cross-tenant
data leakage. ZFS native encryption on the calyx datasets is the outermost layer
(operator/sudo to enable); per-vault keying + keyspace partitioning + grant checks form
the inner two layers (defense in depth). TLS/mTLS or Cloudflare Access secures all
`calyxd` transports. Shared lens weights are referenced by content-id but constellation
vectors never cross vault boundaries. Per-tenant quotas prevent noisy-neighbor
starvation. Cross-cutting hardening applied continuously; finalized here.

> **Operator note:** ZFS dataset encryption (`zfs set encryption=...`) requires sudo.
> Development runs from `CALYX_HOME` on the plain ZFS dataset until the operator
> provisions the encrypted dataset. The Rust code must still compile and the grant/keyspace
> logic must pass all tests without ZFS present. ZFS FSV steps are sudo-gated and
> documented in the FSV exit gate section.

## Dependencies

- **Phases:** PH09 (Constellation CRUD + ingest — vault object, CF layout, and WAL
  must exist so we can attach per-vault key metadata and keyspace partitioning);
  PH03 (error catalog — `CALYX_VAULT_ACCESS_DENIED` and related codes)
- **Provides for:** PH61 (crypto-shred erasure depends on per-vault key infrastructure),
  PH65 (calyxd daemon inherits TLS config), PH66 (systemd provisioning wires ZFS
  encrypted datasets), PH67 (DR restore verifies encrypted vault is unreadable without key)

## Current state (build off what exists)

`calyx-aster` has Constellation CRUD, CF layout, WAL, MVCC, and manifest after PH09.
`crates/calyx-aster/src/vault.rs` exists as a stub. `calyx-core` has the error catalog
stub (PH03). `calyxd` is not yet scaffolded for TLS (that comes in PH65); PH60 lays the
TLS configuration types and the grant model so PH65 can wire them in.
`security/`, `vault/keyspace.rs`, `vault/grant.rs`, and `vault/quota.rs` are greenfield.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-aster/src/vault/key.rs` | `VaultKey` struct; derive/load from host-app key material; per-vault AES-256-GCM key derivation (HKDF); `CALYX_VAULT_KEY_MISSING` |
| `crates/calyx-aster/src/vault/keyspace.rs` | Per-vault full-ULID keyspace prefix encoding; stateless `KeyspaceGuard`; standalone shared `VaultWriteLock`; ensures no CF key from vault A can be read as vault B key |
| `crates/calyx-aster/src/vault/grant.rs` | `GrantEntry`, `GrantStore`; `check_grant(src_vault, dst_vault, actor) -> Result<()>`; `CALYX_VAULT_ACCESS_DENIED`; Ledger-stub grant log write |
| `crates/calyx-aster/src/vault/quota.rs` | Per-tenant ingest/query/VRAM/IO quota counters; `QuotaGuard`; backpressure when exceeded; `CALYX_QUOTA_EXCEEDED` |
| `crates/calyx-aster/src/vault/mod.rs` | Re-exports; `VaultContext` aggregating key + keyspace + grant + quota |
| `crates/calyx-core/src/security.rs` | `TlsConfig`, `MtlsConfig` types; `AuthN` enum (MtlsToken, CloudflareAccess, InProcess); no anonymous write predicate |
| `crates/calyx-aster/src/security/zfs.rs` | ZFS encryption probe (`zfs get encryption`) returning `ZfsEncryptionStatus`; operator-guidance strings; never panics if ZFS absent |
| `crates/calyx-aster/src/security/lens_store.rs` | Shared lens weight store access gated by content-id only; asserts vectors never copied cross-vault; `CALYX_LENS_CROSS_VAULT` guard |
| `crates/calyx-aster/src/tests/` | Unit + proptest + FSV-support tests |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `VaultKey`: per-vault key derivation (HKDF) + AES-256-GCM context | — |
| T02 | `KeyspaceGuard` + `VaultWriteLock`: full-ULID key-prefix + cross-vault read block | T01 |
| T03 | `GrantStore`: grant entry + `check_grant` + `CALYX_VAULT_ACCESS_DENIED` + Ledger-stub audit | T02 |
| T04 | `QuotaGuard`: per-tenant counters + backpressure + `CALYX_QUOTA_EXCEEDED` | T02 |
| T05 | `TlsConfig` / `MtlsConfig` / `AuthN` types + no-anonymous-write predicate | — |
| T06 | ZFS encryption probe + operator-guidance strings + `lens_store` cross-vault guard | T01 |
| T07 | Integration: `VaultContext` wires key + keyspace + grant + quota; cross-vault FSV test | T03, T04, T05, T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Two proofs, both byte-level on aiwonder:

1. **Cross-vault read denied + audited.**
   Create two vaults (`vault-a`, `vault-b`); ingest one constellation into each.
   Attempt to read `vault-b`'s constellation from `vault-a`'s context (no grant).
   Confirm the call returns `CALYX_VAULT_ACCESS_DENIED`.
   Run `calyx ledger-tail --vault vault-a --last 5`; confirm an `AccessDenied`
   audit entry is present for the attempted cross-vault read.

2. **Another tenant's bytes are unreadable with raw disk access (encrypted).**
   Requires sudo on aiwonder: provision the calyx ZFS dataset with
   `zfs create -o encryption=aes-256-gcm -o keylocation=prompt -o keyformat=passphrase tank/calyx/vault-b`.
   Without loading the key (`zfs load-key`), attempt `zfs mount` and confirm it
   fails. Confirm `xxd`-reading the raw ZFS vdev for vault-b's known constellation
   block offset returns ciphertext (no plaintext constellation bytes visible).
   Read `zfs get encryption tank/calyx/vault-b` and confirm `aes-256-gcm`.

## Risks / landmines

- **ZFS sudo-gating on aiwonder:** `zfs create` / `zfs set encryption` / `zfs load-key`
  require root or `zfs` group membership. The Rust code must compile and all
  non-ZFS tests must pass without sudo. FSV step 2 is explicitly sudo-gated and
  can only be run by the operator; document in the GitHub issue.
- **Key material lifetime:** `VaultKey` must never be cloned into a `static`; it must
  be passed explicitly by reference so that drop order is deterministic and zeroize
  runs. Use `zeroize` crate on key material. Zeroize-on-drop FSV tests must keep
  the allocation live with `Box::into_raw` + `std::ptr::drop_in_place` and reclaim
  with `ManuallyDrop`; post-drop stack-pointer reads are undefined behavior and
  forbidden.
- **No anonymous write (A16 + A33):** `AuthN::InProcess` is valid for embedded vaults
  (the host app owns identity); server mode must require mTLS token or Cloudflare
  Access token before any mutation. A missing token → `CALYX_AUTHN_REQUIRED`.
- **Lens store shared by content-id only:** the same `LensId` (content-addressed weights)
  may serve multiple vaults, but each vault's constellation vectors are encrypted under
  that vault's key and are never materialized in another vault's address space.
- **Grant log is a stub until PH36 (Ledger):** PH60 writes a best-effort stub entry;
  PH61 (depends on PH36) wires the real hash-chained Ledger. Do not block PH60
  on PH36 — the stub must at minimum write the `AccessDenied` event to an ephemeral
  in-memory ring so the FSV integration test can read it.
- **`≤500 lines` hard limit:** if `grant.rs` grows to accommodate serialisation and
  in-memory index, split into `grant/entry.rs` and `grant/store.rs`.
