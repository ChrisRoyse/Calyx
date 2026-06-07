# PH61 · T02 — `erase.rs`: Ledger tombstone write + no-content invariant

| Field | Value |
|---|---|
| **Phase** | PH61 — Crypto-shred erasure + STRIDE FSV + secret-scan |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster`, `calyx-ledger` |
| **Files** | `crates/calyx-aster/src/erase.rs` (≤500, extends T01), `crates/calyx-ledger/src/tombstone.rs` (≤500) |
| **Depends on** | T01 (`erase` base) · PH36 (`calyx-ledger` hash-chain CF) |
| **Axioms** | A33, A25, A16, A15 |
| **PRD** | `dbprdplans/30 §4` (Ledger keeps erasure tombstone — no recoverable content; provenance of that an erasure happened survives) |

## Goal

After `erase(scope)` removes CX records and crypto-shreds the key, the append-only
Ledger must receive exactly one new entry: an erasure tombstone that records who,
when, and what was erased — with **no recoverable content bytes** (no plaintext, no
ciphertext of the erased constellation, no original input hash that could be used to
reconstruct). The tombstone is hash-chained like every other Ledger entry, making the
provenance of the erasure itself tamper-evident. The Ledger's content-encryption model
means even the tombstone entry's payload is encrypted with the vault key — but because
the vault key is shredded in T01, the tombstone's *own* payload also becomes
unreadable over time. However the `seq`, `actor_stamp`, and the tombstone-type marker
are written to the Merkle checkpoint before the key is shredded, ensuring the *fact*
of erasure is audit-verifiable even without the key.

## Build (checklist of concrete, code-level steps)

- [ ] `struct ErasureTombstone { seq: u64, vault_id: VaultId, scope: EraseScope, actor: ActorId, erased_at: Timestamp, records_deleted: usize }` —
  **no** content bytes, no constellation payload, no input hash; just provenance
  metadata. `serde`.
- [ ] `impl ErasureTombstone { pub fn as_ledger_payload(&self) -> Vec<u8> }` — canonical
  msgpack/JSON encoding of the struct; this becomes the `payload` field of the
  `LedgerEntry`. The payload is small (< 128 bytes) because it carries no content.
- [ ] `fn write_tombstone(tombstone: &ErasureTombstone, ledger: &mut dyn LedgerAppend) -> Result<LedgerRef>` —
  constructs a `LedgerEntry { kind: EntryKind::Erasure, actor, payload: tombstone.as_ledger_payload(), .. }`;
  appends to the Ledger CF via the hash-chain; returns the `LedgerRef { seq, hash }`.
- [ ] Update `erase()` in `erase.rs` (T01) to call `write_tombstone` **before**
  `shred_key` (order matters: the tombstone must be durably written and the Merkle
  checkpoint updated before the key is destroyed so the *fact* of erasure survives).
- [ ] `fn is_tombstoned(vault_id: VaultId, scope: &EraseScope, ledger: &dyn LedgerRead) -> Result<bool>` —
  scans the Ledger for a matching `EntryKind::Erasure` entry; used by the idempotent
  re-erase check to return `CALYX_ERASE_ALREADY_TOMBSTONED` without re-deleting.
- [ ] `pub fn erase(scope: EraseScope, ...) -> Result<EraseResult>` (update): early
  return `Err(CALYX_ERASE_ALREADY_TOMBSTONED)` if `is_tombstoned` → true (idempotent).
- [ ] Add `EntryKind::Erasure` to `calyx-ledger`'s entry kind enum.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `write_tombstone` with fixed fields → `LedgerRef.seq` matches the expected
  sequence number; the tombstone payload round-trips through `as_ledger_payload`.
- [ ] unit: after `erase(Cx(cx_1))`, scan the Ledger tail; assert exactly one
  `Erasure` entry present with matching `scope = CxId(cx_1)`, `records_deleted = 1`.
- [ ] unit: `is_tombstoned` returns `true` after tombstone write; `false` before.
- [ ] unit: re-call `erase(Cx(cx_1))` after first erase → `CALYX_ERASE_ALREADY_TOMBSTONED`.
- [ ] proptest: tombstone payload bytes contain zero occurrences of the original
  constellation's bytes (property: `payload.windows(4).all(|w| !original_bytes.windows(4).contains(w))`
  for known synthetic payload); confirms no content leaks into the tombstone.
- [ ] edge (≥3): `erase(Vault)` → tombstone scope is `Vault`, not individual CxIds;
  Ledger CF is append-only — tombstone entry cannot be deleted (attempt to overwrite
  returns `CALYX_LEDGER_APPEND_ONLY`); `write_tombstone` crash-safety: if process
  crashes after tombstone write but before `shred_key`, `erase` replay must not
  re-write another tombstone (idempotent via `is_tombstoned`).
- [ ] fail-closed: tombstone write failure (simulated via a failing `LedgerAppend`
  mock) → `erase` aborts *before* `shred_key`; data is not deleted without an audit
  trail (the key is only shredded after the tombstone is durable).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Ledger CF on aiwonder after a real `erase` call; `calyx ledger-tail` output.
- **Readback:**
  1. `cargo test -p calyx-aster erase_tombstone -- --nocapture 2>&1` prints the
     tombstone seq, actor, and `records_deleted`; asserts payload is < 128 bytes
     and contains no constellation content.
  2. `calyx ledger-tail --vault vault-a --last 5` on aiwonder must show a row
     with `kind=Erasure, scope=CxId(<cx_id>), actor=<actor>` — no original payload
     bytes visible in the hex dump.
  3. `calyx verify-chain --vault vault-a --range 0..N` must succeed (chain intact
     including the tombstone entry) — confirms the tombstone is hash-chained.
- **Prove:** before: no erasure entries in Ledger; after erase: exactly one `Erasure`
  entry at the tail; chain verification passes; tombstone payload bytes are absent
  from any `xxd` output of the original constellation's CF range.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines each (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH61 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
