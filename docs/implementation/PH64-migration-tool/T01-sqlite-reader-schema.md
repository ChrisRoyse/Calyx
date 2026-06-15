# PH64 · T01 — SQLite reader and chunk schema mapping

| Field | Value |
|---|---|
| **Phase** | PH64 — Migration tool (sqlite→calyx vault) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/migrate/reader.rs` (≤500), `crates/calyx-cli/src/migrate/mod.rs` (≤500) |
| **Depends on** | — (depends only on the `rusqlite` crate and calyx-core types) |
| **Axioms** | A16, A18 |
| **PRD** | `dbprdplans/15 §5` (migration plan), `dbprdplans/15 §4` (invariants: chunk_id, database_name) |

## Goal

Implement the SQLite reader that opens a Leapable `.db` file, validates the
expected schema (`chunks` table with `chunk_id`, `database_name`, `content`,
`embedding` columns), and streams rows as typed `ChunkRow` structs. This is the
only module that touches SQLite; all downstream modules work with `ChunkRow`.
The reader fails closed on schema mismatch, never silently drops rows.

## Build (checklist of concrete, code-level steps)

- [ ] `migrate/mod.rs`: `pub mod reader; pub mod adapter; pub mod verifier;` with
  re-exports; no wildcard imports
- [ ] `migrate/reader.rs`: `struct ChunkRow { chunk_id: String, database_name:
  String, content: Vec<u8>, embedding: Vec<f32>, row_num: u64 }` — all fields
  named verbatim from the `vault-sqlite.ts` contract (`chunk_id`, `database_name`)
- [ ] `fn open_sqlite(path: &Path) -> Result<rusqlite::Connection, CliError>`:
  opens in read-only mode; `CALYX_CLI_IO_ERROR` if file not found
- [ ] `fn validate_schema(conn: &Connection) -> Result<(), CliError>`: queries
  `pragma table_info(chunks)` and asserts the columns `chunk_id`, `database_name`,
  `content`, `embedding` are all present; `CALYX_CLI_USAGE_ERROR` with message
  `"SQLite db missing expected chunks schema; remediation: verify this is a
  Leapable vault .db file"` if any column is absent
- [ ] `fn row_count(conn: &Connection) -> Result<u64, CliError>`: `SELECT COUNT(*)
  FROM chunks`; used for progress reporting
- [ ] `fn stream_rows(conn: &Connection) -> impl Iterator<Item = Result<ChunkRow,
  CliError>>`: `SELECT chunk_id, database_name, content, embedding FROM chunks
  ORDER BY rowid`; decodes `embedding` from sqlite-vec blob format (4-byte
  little-endian f32 per dimension); `CALYX_CLI_IO_ERROR` on decode failure with
  the row number in the message
- [ ] Embedding decode: sqlite-vec stores float32 as raw LE bytes; `embedding`
  column is a `BLOB` of `dim * 4` bytes; decode as `Vec<f32>` with bounds check
  (exactly `768 * 4 = 3072` bytes for the GTE lens); `CALYX_CLI_USAGE_ERROR` if
  wrong size (dimension mismatch is a hard error — never mix vectors, PRD 15 §4)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: create an in-memory SQLite DB with the correct schema, insert 3 rows,
  call `stream_rows` → yields 3 `ChunkRow`s in rowid order with correct field values
- [ ] unit: `validate_schema` on a DB with a missing `embedding` column →
  `CliError` containing `"CALYX_CLI_USAGE_ERROR"` and `"Leapable vault"` in message
- [ ] unit: embedding blob of exactly 3072 bytes (`768 * 4`) → decoded to
  `Vec<f32>` with 768 elements; first 4 bytes `[0x00, 0x00, 0x80, 0x3F]` → `1.0f32`
- [ ] unit: embedding blob of 3068 bytes (wrong size) → `CALYX_CLI_USAGE_ERROR`
  with message containing the row number
- [ ] proptest: for any `Vec<f32>` of length 768, encoding to LE bytes then
  decoding via `stream_rows` returns the same float values within f32 precision
- [ ] edge: empty `chunks` table → `stream_rows` yields 0 rows, no error;
  `chunk_id` or `database_name` as empty string → `ChunkRow` preserves the empty
  string (not substituted); non-UTF-8 `chunk_id` → `CALYX_CLI_IO_ERROR` with
  row number

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** a real Leapable `.db` file on aiwonder (e.g.
  `$CALYX_HOME/testdata/sample-leapable.db`)
- **Readback:** run a small Rust binary (or `cargo test -- --nocapture`) that
  calls `stream_rows` on the real `.db` and prints the first 3 rows' `chunk_id`
  and `embedding[0]` to stdout; cross-check `embedding[0]` against
  `sqlite3 sample-leapable.db "SELECT hex(embedding) FROM chunks LIMIT 1"` —
  first 8 hex chars = LE bytes of `embedding[0]`
- **Prove:** the first `embedding[0]` float printed by the reader matches the
  value decoded manually from the SQLite hex dump; row count printed by `row_count`
  matches `sqlite3 … "SELECT COUNT(*) FROM chunks"`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH64 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
