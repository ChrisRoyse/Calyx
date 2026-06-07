# PH61 · T01 — `erase.rs`: `erase(scope)` removes CX + derived + crypto-shreds key

| Field | Value |
|---|---|
| **Phase** | PH61 — Crypto-shred erasure + STRIDE FSV + secret-scan |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/erase.rs` (≤500) |
| **Depends on** | — (first card; depends on PH60 `VaultContext`, PH09 CX CRUD) |
| **Axioms** | A33, A25, A16 |
| **PRD** | `dbprdplans/30 §4` (right to erasure — crypto-shred per-vault/per-record key) |

## Goal

Implement the `erase(scope, vault_ctx)` operation that satisfies GDPR/CCPA right-to-
erasure by crypto-shredding. Three erase scopes exist: `Vault` (destroy the entire
vault), `Cx(CxId)` (single constellation), and `Subject(SubjectId)` (all
constellations tagged to an identified subject). For each scope the operation removes
the CF records, removes all derived data (cross-terms, index entries, recurrence
occurrences — via a pluggable `EraseHandler` trait), and then **zeroes the
per-vault or per-record `VaultKey`** so that any ciphertext remaining on disk
(including in cold storage or backups) becomes permanently unrecoverable. The Ledger
tombstone write happens in T02; T01 covers all data removal and key destruction.

**A25 clarification (must appear in code and PR):** A25 forbids deleting-to-compress
only. It explicitly permits — and A33 requires — lawful/user-requested deletion.
No agent may refuse a lawful delete citing A25. This comment must be present in
`erase.rs` as a `//` block above the `erase` function.

## Build (checklist of concrete, code-level steps)

- [ ] Add the A25 clarification comment verbatim above the `erase` function:
  ```rust
  // A25 ("never delete to compress") does NOT forbid this operation.
  // Erasure is lawful/user-requested deletion, explicitly required by A33.
  // No reviewer may reject a lawful erase citing A25.
  ```
- [ ] `enum EraseScope { Vault, Cx(CxId), Subject(SubjectId) }` — `serde`, `Debug`.
- [ ] `trait EraseHandler: Send + Sync { fn erase(&self, scope: &EraseScope, vault_id: VaultId) -> Result<()>; }` —
  pluggable cleanup; each crate (Loom, Sextant, Temporal) registers a handler;
  stub implementations that log "no derived data to erase" are valid until those
  crates exist.
- [ ] `struct EraseRegistry { handlers: Vec<Box<dyn EraseHandler>> }` — collects
  registered handlers; `add_handler`, `run_all(scope, vault_id)`.
- [ ] `fn erase_cf_records(scope: &EraseScope, vault_ctx: &VaultContext, wal: &mut Wal) -> Result<usize>` —
  deletes the raw CF key-value records for the given scope within the vault's
  keyspace; returns count of records deleted; uses `vault_ctx.encode_key` to
  construct the correct key range; writes WAL tombstone entries for the deleted keys
  (the WAL provides crash-safety for the erase, not undoability).
- [ ] `fn shred_key(vault_ctx: &mut VaultContext) -> Result<()>` — calls
  `vault_ctx.key` `.inner.zeroize()` to overwrite the in-memory key bytes; writes
  a sentinel `[0u8; 32]` placeholder so subsequent decrypt attempts return
  `CALYX_DECRYPTION_FAILED` (not UB — fail closed, A16).
- [ ] `pub fn erase(scope: EraseScope, vault_ctx: &mut VaultContext, registry: &EraseRegistry, wal: &mut Wal) -> Result<EraseResult>` —
  (1) `erase_cf_records`; (2) `registry.run_all`; (3) `shred_key` (Vault scope only —
  per-record scope shreds only that record's key if per-record keys are implemented,
  otherwise shreds vault key and marks vault sealed); returns `EraseResult { scope, records_deleted, shredded_at }`.
- [ ] `struct EraseResult { scope: EraseScope, records_deleted: usize, shredded_at: Timestamp }`.
- [ ] Add `CALYX_ERASE_ALREADY_TOMBSTONED` to `calyx-core/src/error.rs` (for the
  idempotent re-erase case, handled in T02).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: ingest 3 synthetic CX records; `erase(Cx(cx_1))` → `records_deleted == 1`;
  subsequent direct CF read of cx_1's key → empty (record gone).
- [ ] unit: `erase(Vault)` → all 3 records deleted; `shred_key` zeroes the key inner
  bytes; `vault_ctx.decrypt_value(...)` returns `CALYX_DECRYPTION_FAILED`.
- [ ] unit: `erase(Subject(s))` with 2 CXs tagged to subject `s` and 1 untagged →
  `records_deleted == 2`; untagged CX survives.
- [ ] proptest: `∀ scope` with seeded record sets: `records_deleted` equals the
  number of matching records; no other records are touched (property: erase is
  scope-exact).
- [ ] edge (≥3): `erase(Cx(unknown_id))` → `records_deleted == 0` (no error — absent
  is already erased); `erase(Vault)` on empty vault → `records_deleted == 0`;
  handler returning `Err(...)` → `erase` propagates the error before shredding
  (derived-data removal must succeed before key destruction).
- [ ] fail-closed: after `shred_key`, `decrypt_value` returns `CALYX_DECRYPTION_FAILED`,
  never garbage plaintext.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** synthetic aster CF in a temp directory on aiwonder; known CX bytes;
  `xxd` readback of the CF range after erase.
- **Readback:**
  - `cargo test -p calyx-aster erase_cf -- --nocapture 2>&1` prints
    `records_deleted = 1` (Cx scope) and `records_deleted = 3` (Vault scope).
  - After `erase(Vault)`, run `xxd <aster_cf_path> | grep <known_cx_bytes_hex>`;
    assert: zero matches (plaintext gone from CF, ciphertext tombstoned).
  - After `shred_key`, assert `vault_ctx.key.inner == [0u8; 32]` in the zeroize test.
- **Prove:** before: CX bytes readable from CF; after erase: CF range empty;
  after `shred_key`: `decrypt_value` fails closed; no plaintext in `xxd` output.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH61 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
