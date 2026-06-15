# PH64 · T05 — Real .db migration FSV on aiwonder

| Field | Value |
|---|---|
| **Phase** | PH64 — Migration tool (sqlite→calyx vault) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/migrate/verifier.rs` (extend), `crates/calyx-cli/src/cmd/migrate.rs` (extend) |
| **Depends on** | T04, T03, T02, T01 (full migration pipeline) |
| **Axioms** | A15, A16, A18 |
| **PRD** | `dbprdplans/15 §5` FSV gate, `dbprdplans/25_STAGE15_INTERFACES.md` FSV gate |

## Goal

Run the full migration FSV on a real Leapable `.db` file on aiwonder and collect
byte-level proof. Migrate the real `.db` → Calyx vault → verify every
constellation is byte-exact on content → read the raw CF bytes and compare them
against direct SQLite queries. Also port the allowed-direct-import tests from the
TypeScript `vault-sqlite.ts` test suite to Rust. This card closes the PH64 GitHub
issue.

## Build (checklist of concrete, code-level steps)

- [ ] Port the `vault-sqlite.ts` allowed-direct-import test invariants to
  `migrate/adapter.rs` as Rust integration tests (within `#[cfg(test)]`):
  - Invariant 1: `chunk_id` and `database_name` appear verbatim in the migrated
    constellation metadata — verified by `vault.get(cx_id).metadata["chunk_id"]
    == original_row.chunk_id`
  - Invariant 2: content-addressed `CxId` is stable: migrating the same SQLite
    row twice produces the same `CxId` (idempotency)
  - Invariant 3: no cleartext content in the vault: the `Constellation.input_ref`
    is `ContentHash(h)`, not `PlainText(s)` — verified by asserting
    `matches!(constellation.input_ref, InputRef::ContentHash(_))`
  - Invariant 4: vectors from different models (different `gte_lens_id`) land in
    different slots — verified by creating two adapters with different `LensId`s
    and asserting the `slot_id`s differ
- [ ] Add `calyx migrate verify` to the CLI subcommand dispatch in `cmd/mod.rs`
  (if not already there from T04)
- [ ] Add FSV helper: `calyx migrate status <vault.calyx>` prints a summary of
  the migrated vault: row count, slot count, Ledger chain status
  (`verify_chain` result), and first/last `chunk_id` (from constellation metadata)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] integration (ported from vault-sqlite.ts): all 4 allowed-direct-import
  invariants pass on a seeded in-memory SQLite DB
- [ ] integration: migrate a 100-row in-memory SQLite DB → `verify_migration`
  returns `matched=100, mismatched=0`; `vault.get` on each `CxId` succeeds;
  `constellation.metadata["chunk_id"]` equals the source `chunk_id`
- [ ] integration: run `migrate vault --verify` on a 50-row DB → exit 0,
  stdout contains `"verified 50/50 rows: byte-exact on content"`
- [ ] regression: all existing PH62/PH63 tests still green after the new
  `migrate` subcommand is added to the dispatch table
- [ ] edge: SQLite with a `NULL` embedding → `CALYX_CLI_IO_ERROR` with row
  number; SQLite with non-UTF-8 `database_name` → `CALYX_CLI_IO_ERROR` with
  row number and the raw bytes in the message
- [ ] fail-closed: `migrate verify` when vault has 0 constellations but SQLite
  has N rows → `mismatched=N`, exit 2 (not a silent pass)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** a real Leapable `.db` file at `$CALYX_HOME/testdata/sample-leapable.db`
  on aiwonder (or the smallest real production Vault available for testing)
- **Readback:** run the full sequence on aiwonder (not in CI):
  ```bash
  # 1. Migrate
  calyx migrate vault \
      $CALYX_HOME/testdata/sample-leapable.db \
      /tmp/ph64-fsv.calyx \
      --verify

  # 2. Read a known row's bytes from the Calyx vault
  # (use chunk_id from the SQLite source to find cx_id)
  CHUNK_ID=$(sqlite3 $CALYX_HOME/testdata/sample-leapable.db \
      "SELECT chunk_id FROM chunks LIMIT 1")
  # cx_id is printed by the migration output; or compute it:
  # CX_ID = blake3(content ‖ \x00\x00\x00\x01 ‖ \x00*16) first 16 bytes hex
  calyx readback --cf-row /tmp/ph64-fsv.calyx --cf base --key $CX_ID_HEX

  # 3. Cross-check content hash
  CONTENT=$(sqlite3 $CALYX_HOME/testdata/sample-leapable.db \
      "SELECT content FROM chunks WHERE chunk_id='$CHUNK_ID'")
  echo -n "$CONTENT" | b3sum --no-names   # expected hash
  # Extract input_ref.content_hash from the CF row hex dump → must match

  # 4. Verify chain
  calyx readback --ledger /tmp/ph64-fsv.calyx --seq 1

  # 5. Full verify
  calyx migrate verify \
      $CALYX_HOME/testdata/sample-leapable.db \
      /tmp/ph64-fsv.calyx
  ```
- **Prove:**
  - Step 1 exits 0; stderr shows `"verified N/N rows: byte-exact on content"` where
    N = `sqlite3 … "SELECT COUNT(*) FROM chunks"`
  - Step 2 produces a non-empty hex dump
  - Step 3 cross-check: `b3sum` output matches the `input_ref.content_hash`
    extracted from the hex dump — byte-for-byte
  - Step 4 produces a non-empty Ledger entry hex dump (Ledger chain present)
  - Step 5 exits 0
  - Screenshot/log of all five steps attached to the PH64 GitHub issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH64 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
