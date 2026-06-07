# PH64 · T04 — Readback verifier: byte-exact content comparison

| Field | Value |
|---|---|
| **Phase** | PH64 — Migration tool (sqlite→calyx vault) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/migrate/verifier.rs` (≤500) |
| **Depends on** | T03 (migration complete), T01 (reader), PH62·T07 (readback --cf-row) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/15 §5` (byte-exact-on-content gate), `dbprdplans/25_STAGE15_INTERFACES.md` (FSV gate) |

## Goal

Implement the readback verifier that confirms a migration is byte-exact on content:
for every row in the source SQLite `chunks` table, it reads the corresponding
constellation from the Calyx vault, extracts the content hash from `input_ref`,
and compares it against `blake3(row.content)`. Mismatches fail closed with the
row number, `chunk_id`, and both hashes in the error message. This is the
mechanical FSV gate — the verifier is used directly on aiwonder, not as a CI
harness.

## Build (checklist of concrete, code-level steps)

- [ ] `migrate/verifier.rs`: `struct VerifyResult { total: u64, matched: u64,
  mismatched: u64, errors: Vec<VerifyError> }` where `VerifyError { row_num: u64,
  chunk_id: String, expected_hash: [u8;32], actual_hash: [u8;32] }`
- [ ] `fn verify_migration(sqlite: &Connection, vault: &dyn VaultStore,
  adapter: &VaultSqliteAdapter) -> Result<VerifyResult, CliError>`:
  - For each row from `reader::stream_rows(sqlite)`:
    - Derive `cx_id = adapter.derive_cx_id(&row.content)` (same formula as
      `from_chunk_row`)
    - Call `vault.get(cx_id, vault.snapshot())` → `Constellation`; if `Err` →
      record as `VerifyError` with `expected_hash = blake3(row.content)`,
      `actual_hash = [0u8;32]` (not found)
    - Extract `expected = blake3(row.content)`; `actual` = `constellation.
      input_ref.content_hash()`; compare byte-by-byte
    - Mismatch → append `VerifyError`; do not abort (verify all rows)
  - Returns `VerifyResult` with counts; does NOT print — caller prints
- [ ] `calyx migrate verify <sqlite> <vault.calyx>` subcommand (extend `cmd/
  migrate.rs`): calls `verify_migration` and prints:
  - On success: `"verified N/N rows: byte-exact on content"` to stdout, exit 0
  - On any mismatch: prints each `VerifyError` as `"MISMATCH row=N chunk_id=X
    expected=<hex64> actual=<hex64>"` to stdout; then `"FAILED: M mismatches"`
    to stderr, exit 2; `CALYX_ASTER_CORRUPT_SHARD` error code
- [ ] The verifier checks content-hash byte-exactness, not float-vector
  byte-exactness (the FSV gate is "byte-exact on content" per PRD `15 §5`)
- [ ] Also add `--verify` flag to `migrate vault` so that a single command migrates
  and immediately verifies: `calyx migrate vault <sqlite> <vault.calyx> --verify`
  → runs migration then `verify_migration`; exits 2 if any mismatch

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `verify_migration` on a correctly migrated 5-row vault → `VerifyResult{
  total:5, matched:5, mismatched:0, errors:[]}`
- [ ] unit: manually corrupt one constellation's `input_ref` (change one byte) →
  `VerifyResult{mismatched:1}` and `errors[0].chunk_id` matches the tampered row
- [ ] unit: `cx_id` not found in vault → `VerifyError` with `actual_hash=[0u8;32]`
  and `"MISMATCH"` in the output (not a panic)
- [ ] unit: `verify_migration` verifies all rows even when one mismatch is found
  (does not short-circuit)
- [ ] proptest: for any `Vec<u8>` content, `blake3(content)` as computed by the
  verifier equals `blake3::hash(content).as_bytes()` (hash function consistency)
- [ ] edge: SQLite with 0 rows → `VerifyResult{total:0, matched:0}`, exit 0;
  vault with one extra constellation not from the SQLite source → not flagged
  as an error (verifier only checks SQLite→Calyx direction)
- [ ] fail-closed: all N rows mismatched → exit 2; the exit-2 is enforced by the
  subcommand, not by the verifier struct (verifier is pure data)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the `VerifyResult` output from running `calyx migrate verify
  sample-leapable.db /tmp/migrated.calyx` on aiwonder
- **Readback:**
  - `calyx migrate verify sample-leapable.db /tmp/migrated.calyx` → stdout
    shows `"verified N/N rows: byte-exact on content"`, exit 0
  - Manual cross-check for one row: `sqlite3 sample-leapable.db "SELECT content
    FROM chunks WHERE chunk_id='abc123'"` → pipe through `b3sum --no-names` →
    `<expected_hash>`. Then `calyx readback --cf-row /tmp/migrated.calyx --cf
    base --key <cx_id_hex>` → extract `input_ref.content_hash` from hex dump →
    must equal `<expected_hash>` byte-for-byte
- **Prove:** `verify_migration` prints `"byte-exact on content"` for the real
  `.db` on aiwonder; the manual cross-check shows the content hash in the CF
  row matches `blake3(sqlite_content_bytes)` exactly; exit code is 0

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH64 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
