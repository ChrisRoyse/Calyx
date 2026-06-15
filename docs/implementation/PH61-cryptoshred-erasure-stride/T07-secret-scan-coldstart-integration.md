# PH61 · T07 — Secret-scan + cold-start honesty + full phase FSV integration

| Field | Value |
|---|---|
| **Phase** | PH61 — Crypto-shred erasure + STRIDE FSV + secret-scan |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster`, `calyx-core` (cross-cutting) |
| **Files** | `crates/calyx-aster/src/tests/ph61_integration.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05, T06 |
| **Axioms** | A33, A25, A2, A16 |
| **PRD** | `dbprdplans/30 §4` (no agent may refuse lawful delete citing A25); `dbprdplans/30 §5` (cold-start honesty); `dbprdplans/30 §2` (secret-scan — pre-commit, never a value in repo/issue) |

## Goal

Run the full PH61 FSV integration: (1) the complete erase → tombstone → crypto-shred
flow verified on aiwonder with raw-disk readback; (2) cold-start honesty — a vault
with no anchors tags all Assay/Lodestar/Ward outputs as `provisional` and refuses
high-stakes paths, then transitions to non-provisional as anchors arrive; (3)
secret-scan clean — `gitleaks detect` returns exit 0 on the full repo on aiwonder,
with synthetic test fixture secrets exempted via `.gitleaksignore`. This card also
wires the pre-commit hook that blocks commits containing secrets.

## Build (checklist of concrete, code-level steps)

### Cold-start honesty (`calyx-core/src/cold_start.rs`, new, ≤500 lines)

- [ ] `enum VaultTrustState { Provisional, Grounded { anchor_count: usize } }`.
- [ ] `struct ColdStartGuard { state: VaultTrustState }`.
- [ ] `impl ColdStartGuard { pub fn new() -> Self }` — starts `Provisional`.
- [ ] `pub fn record_anchor(&mut self)` — increments `anchor_count`; transitions to
  `Grounded` once `anchor_count >= 1`.
- [ ] `pub fn assert_grounded(&self, operation: &str) -> Result<()>` — returns
  `Ok(())` if `Grounded`; returns `CALYX_PROVISIONAL_VAULT` if `Provisional`,
  with `operation` name in the error payload (fail closed, A16). High-stakes
  operations (Ward guard enforcement, Lodestar kernel answers with claimed
  confidence) must call this before returning results as non-provisional.
- [ ] `pub fn search_always_ok(&self) -> bool` — always `true`; search is permitted
  from day 0 regardless of trust state (cold-start honesty: "search works immediately
  on day 0", `30 §5`).
- [ ] Add `CALYX_PROVISIONAL_VAULT` to `calyx-core/src/error.rs`.

### Secret-scan wiring (`.pre-commit-config.yaml` or equivalent gate script)

- [ ] Create `scripts/secret-scan.sh` — runs `gitleaks detect --source . --log-opts HEAD`
  and exits non-zero if secrets detected. This file contains no secrets; it only
  invokes the tool.
- [ ] Document in a comment in `secret-scan.sh`: "Never commit a credential value.
  Use Infisical for secrets; env-var names in code only (`30 §2`)."
- [ ] Wire `scripts/secret-scan.sh` as a pre-commit hook in `.claude/settings.json`
  `pre_commit_hooks` (or equivalent Calyx gate mechanism) so it runs before every
  commit on aiwonder.

### Integration test (`tests/ph61_integration.rs`)

- [ ] **Full erase + tombstone + no-content proof:**
  - Ingest known constellation bytes into vault-a.
  - Call `erase(Cx(cx_id), ...)`.
  - Assert `EraseResult.records_deleted == 1`.
  - Assert `is_tombstoned(vault_a, Cx(cx_id), &ledger) == true`.
  - Assert `vault_ctx.decrypt_value(...)` on the erased record returns
    `CALYX_DECRYPTION_FAILED` (key shredded).
  - Assert Ledger tombstone entry payload bytes contain zero occurrences of the
    original constellation content bytes.

- [ ] **Backup unreadability proof (real restic):**
  - Seed the issue #597 fixture with `CALYX_ISSUE597_FSV_ROOT=<root> cargo test -p
    calyx-aster --test issue597_real_restic_crypto_shred -- --ignored --nocapture`.
  - Run a real `restic backup` of the seeded `vault-a` directory on aiwonder.
  - Restore the snapshot to a separate ZFS staging directory.
  - Grep the restored bytes for the erased plaintext sentinel → absent.
  - Re-run the fixture with `CALYX_ISSUE597_RESTORED_VAULT=<restored-vault>` and
    verify the restored ciphertext decrypts with a shredded key as
    `CALYX_DECRYPTION_FAILED`.
  - Print: `"restored_decrypt_with_shredded_key=Err(CALYX_DECRYPTION_FAILED)"`.

- [ ] **Cold-start honesty:**
  - Create a `ColdStartGuard`; assert `search_always_ok() == true`.
  - Assert `assert_grounded("oracle_answer")` → `CALYX_PROVISIONAL_VAULT`.
  - Call `record_anchor()`; assert `assert_grounded("oracle_answer")` → `Ok(())`.

- [ ] **A25 non-conflict assertion:**
  - The integration test file must include this comment block:
    ```rust
    // A25 forbids deleting-to-compress, NOT lawful/user-requested deletion.
    // erase() is A33-required. No reviewer may refuse this citing A25.
    // See: dbprdplans/30 §4 (binding clarification).
    ```

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] integration: all assertions above use hard-coded synthetic data (known CxId,
  known master key, known constellation bytes 32-byte fixed payload); no random
  data without a seeded RNG.
- [ ] unit (`cold_start.rs`): `new()` → Provisional; `record_anchor()` → Grounded;
  `assert_grounded` before and after; `search_always_ok` always `true`.
- [ ] edge (≥3): erase then `apply_retention` on the same CX → idempotent
  (`CALYX_ERASE_ALREADY_TOMBSTONED`, not a second erase); `ColdStartGuard` after
  multiple `record_anchor` calls → `anchor_count` increments correctly; secret-scan
  on a branch that introduces a `.env` file with a fake API key → `gitleaks detect`
  exits non-zero (confirmed by negative FSV test documented in GitHub issue comments).
- [ ] fail-closed: all eight `CALYX_*` codes from PH61 enumerated and tested in at
  least one fail-closed assertion each in this file.

## FSV (read the bytes on aiwonder — the truth gate)

Four byte-level proofs on aiwonder (matches the PH61 FSV exit gate):

1. **No recoverable content after erase:**
   `cargo test -p calyx-aster ph61_integration -- --nocapture 2>&1` must print:
   - `erase result: records_deleted=1 ✓`
   - `tombstone present: true ✓`
   - `decrypt after shred: Err(CALYX_DECRYPTION_FAILED) ✓`
   - `restored_decrypt_with_shredded_key=Err(CALYX_DECRYPTION_FAILED)`
   - `tombstone payload contains no content bytes: true ✓`

2. **Cross-vault denied + Ledger-audited (real Ledger, not stub ring):**
   `calyx ledger-tail --vault vault-a --last 10` on aiwonder shows an `AccessDenied`
   entry that is part of the verified hash chain.

3. **At-rest + in-transit encryption verified (ZFS + TLS types compiled):**
   `cargo build -p calyx-core --lib` — `TlsConfig`, `MtlsConfig`, `AuthN` compile;
   (sudo) `zfs get encryption tank/calyx` → `aes-256-gcm`.

4. **Secret-scan clean:**
   `gitleaks detect --source . --log-opts HEAD` on aiwonder exits 0.
   Screenshot attached to PH61 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] All four FSV proofs evidenced (output screenshots / `xxd` outputs) attached to
  the PH61 GitHub issue
- [ ] Secret-scan pre-commit hook wired and verified clean on aiwonder
- [ ] `SECURITY` predicate in `BUILD_DONE` (`19 §5`) is satisfied:
  `STRIDE defenses FSV-proven ∧ cross-vault read denied+audited ∧ at-rest+in-transit encryption verified ∧ erase() crypto-shreds (content unrecoverable incl. backups/Ledger payload; tombstone remains) ∧ secret-scan clean`
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
