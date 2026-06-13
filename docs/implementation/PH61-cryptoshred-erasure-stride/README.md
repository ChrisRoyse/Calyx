# PH61 — Crypto-shred erasure + STRIDE FSV + secret-scan

**Stage:** S14 — Security & Privacy by Construction  ·  **Crate:** `calyx-aster`, `calyx-ledger`, `calyx-core` (cross-cutting)  ·
**PRD roadmap:** P13  ·  **Axioms:** A33, A25, A2, A16

## Objective

Implement right-to-erasure as a first-class, GDPR/CCPA-satisfying operation via
crypto-shredding: `erase(vault | cx | subject)` removes all constellations and derived
data (cross-terms, index entries, recurrence occurrences) and destroys the
per-vault/per-record encryption key so that cold copies, backups, and the append-only
Ledger payloads become permanently unrecoverable. The Ledger keeps an erasure tombstone
(actor, timestamp, scope) for audit continuity — no recoverable content survives.

A25 ("never delete data, never lose intelligence") forbids deleting-to-compress only.
It explicitly permits — and A33 requires — lawful/user-requested deletion. **No agent
may refuse a lawful delete citing A25** — this must be understood by every code reviewer.

Alongside erasure, this phase FSV-proves all six STRIDE defenses from `30 §1`, wires
retention/TTL, consent/purpose tags, PII redaction, and supply-chain integrity
(pinned crates, `cargo audit`, SBOM). Pre-commit secret-scan (never a value in
repo/issue) is wired and verified clean on aiwonder. Cold-start honesty (provisional
until anchors arrive, never faked) is documented and asserted in tests.

> **Operator note:** ZFS dataset encryption (provisioned in PH60) remains
> operator/sudo-gated. PH61 crypto-shred works even when ZFS encryption is absent:
> the per-vault/per-record Rust-layer key is always present; crypto-shredding it
> makes the data unrecoverable regardless of whether ZFS encryption is also active.

## Dependencies

- **Phases:** PH60 (per-vault `VaultKey` + `VaultContext` — the key to shred must
  exist before we can shred it); PH36 (Merkle-verified Ledger — erasure tombstone must
  be written to the real hash-chain Ledger, not just the stub ring from PH60)
- **Provides for:** PH65 (calyxd must call `erase` on DELETE requests), PH67 (DR
  restore FSV verifies no recoverable content after erase + backup), PH62 (CLI
  exposes `calyx erase` command), PH70 (intelligence FSV must confirm erase does
  not corrupt surviving vaults)

## Current state (build off what exists)

`calyx-aster` has `VaultContext` (key + keyspace + grant + quota) after PH60.
`calyx-ledger` has the hash-chain CF + Merkle checkpoints after PH36.
`calyx-core/src/error.rs` has the growing error catalog.
`erase.rs`, `retention.rs`, `consent.rs`, `redaction.rs`, `supply_chain.rs`, and
`secret_scan.rs` are all greenfield. STRIDE FSV integration test is greenfield.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-aster/src/erase.rs` | `erase(scope, vault_ctx, ledger) -> EraseResult`; removes CX + derived; crypto-shreds key; writes Ledger tombstone |
| `crates/calyx-aster/src/retention.rs` | Per-collection TTL config; `RetentionPolicy`; `apply_retention(vault_ctx, now)` scans and marks expired for erase |
| `crates/calyx-core/src/consent.rs` | `ConsentTag`, `PurposeBasis`; `check_consent(tag, purpose) -> Result<()>`; `CALYX_CONSENT_VIOLATION` |
| `crates/calyx-aster/src/redaction.rs` | PII redaction: `redact_input(raw) -> RedactedInput`; hash-only mode; `CALYX_PII_REDACTION_REQUIRED` |
| `crates/calyx-aster/src/supply_chain.rs` | SBOM manifest generation; `cargo audit` invocation; content-addressed lens weight verification; pinned-version assertion |
| `crates/calyx-aster/src/stride_fsv.rs` | STRIDE defense integration test harness: six named test functions, one per threat category, FSV-asserting |
| `crates/calyx-aster/src/tests/ph61_integration.rs` | Full erase + tombstone + crypto-shred FSV test; STRIDE sweep; secret-scan check |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `erase.rs`: `erase(scope)` removes CX + derived + crypto-shreds key | — |
| T02 | `erase.rs`: Ledger tombstone write + no-content invariant | T01, PH36 |
| T03 | `retention.rs`: TTL policy + `apply_retention` scan | T01 |
| T04 | `consent.rs` + `redaction.rs`: consent/purpose tags + PII hash-only mode | — |
| T05 | `supply_chain.rs`: SBOM + `cargo audit` + content-addressed lens weight check | — |
| T06 | `stride_fsv.rs`: six STRIDE defenses FSV-proven | T01, T02, PH60 |
| T07 | Secret-scan + cold-start honesty + full phase FSV integration | T01–T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Four proofs, all byte-level on aiwonder:

1. **Crypto-shred: no recoverable content.**
   Ingest a known constellation into vault-a; call `calyx erase --vault vault-a --cx <cx_id>`.
   Then: (a) read the raw Aster CF bytes via `xxd` — no constellation plaintext visible;
   (b) read the most-recent real restic backup snapshot for vault-a (simulated
   backup files do not close this gate) — the same CF range in the backup returns
   only ciphertext, key is gone;
   (c) read the Ledger tail — a `Tombstone` entry at the correct seq is present, containing
   `{ kind: Erased, scope: CxId, actor, at }` with no original payload bytes.

2. **Cross-vault read denied + audited** (re-proven after PH61 wires real Ledger).
   Same procedure as PH60 FSV gate 1, but now the audit entry is a real hash-chained
   Ledger entry (verify with `calyx verify-chain --vault vault-a --last 5`).

3. **At-rest + in-transit encryption verified.**
   (sudo-gated) `zfs get encryption tank/calyx` confirms `aes-256-gcm`.
   `openssl s_client -connect localhost:7700` confirms TLS handshake (PH65 wires this;
   PH61 documents the requirement and verifies the `TlsConfig` type is wired).

4. **Secret-scan clean.**
   `cargo install gitleaks` (pinned version); `gitleaks detect --source . --log-opts HEAD`
   returns exit 0 (no secrets detected). Verified on aiwonder; output screenshot
   attached to the PH61 GitHub issue.

## Risks / landmines

- **Append-only Ledger vs erasure invariant:** the tombstone must be written to the
  Ledger CF *before* the key is zeroized; if the process crashes between the tombstone
  write and the key zeroize, the key still exists and a replay must re-attempt the
  shred. The erase operation is idempotent: re-running on an already-tombstoned CX is
  a no-op returning `Ok(())`.
- **Zeroize proof safety:** PH61 tests may read an explicitly zeroized live key before
  drop, but any drop-triggered wipe proof must use the PH60 heap-live
  `drop_in_place` probe. A post-`drop` stack-pointer read is undefined behavior and
  cannot be FSV evidence.
- **Backup copies:** crypto-shred renders backup ciphertext permanently unreadable, but
  the operator must ensure backups are encrypted with the same vault key (or the vault
  key itself is included in the backup and can be shredded separately). Document the
  backup key lifecycle in the PH67 card.
- **A25 / lawful delete — no code review may block this:** the PR description for
  every card in this phase must include: "A25 forbids deleting-to-compress, not
  lawful/user deletion. This erase op is explicitly permitted by A33. No reviewer
  may reject citing A25."
- **Derived data completeness:** `erase` must remove cross-terms in the Loom CF,
  HNSW index entries in the Sextant CF, and recurrence occurrences in the temporal CF.
  These CFs may not all exist at PH61 time; the erase implementation must be designed
  as a pluggable `EraseHandler` trait so later phases (Loom, Sextant, Temporal) can
  register their cleanup logic. Stub handlers that log "nothing to erase" are acceptable
  until those crates exist.
- **Secret-scan false positives:** test fixture bytes that happen to look like API keys
  must be clearly marked as synthetic and exempted via `.gitleaksignore` rather than
  excluded from the scan entirely.
- **`≤500 lines` hard limit:** `erase.rs` may need to split into `erase/scope.rs` and
  `erase/shred.rs` if derived-data cleanup logic grows.
