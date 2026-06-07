# PH64 · T03 — Migrate subcommand: orchestration and progress

| Field | Value |
|---|---|
| **Phase** | PH64 — Migration tool (sqlite→calyx vault) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/cmd/migrate.rs` (≤500) |
| **Depends on** | T02 (adapter), PH62·T02 (create-vault, add-lens engine API), PH09 (VaultStore::put idempotent) |
| **Axioms** | A1, A15, A16, A17 |
| **PRD** | `dbprdplans/15 §5` (migration plan), `dbprdplans/14 §2` (ingest idempotent) |

## Goal

Implement `calyx migrate vault <sqlite-path> <vault.calyx>` — the single command
that orchestrates the full migration: opens the SQLite source, creates a new Calyx
vault, registers the GTE lens, streams all rows through the adapter, writes each
constellation via `VaultStore::put` (idempotent, group-committed), and reports
progress to stderr. The migration is restartable: re-running it on a vault that
already has some constellations skips duplicates (content-addressed CxIds are
idempotent).

## Build (checklist of concrete, code-level steps)

- [ ] `cmd/migrate.rs` — `migrate vault <sqlite> <calyx-path>
  [--gte-lens-id <hex16>] [--gte-endpoint <url>] [--batch-size <n=100>]
  [--dry-run]`:
  1. call `reader::open_sqlite(sqlite_path)` and `reader::validate_schema`
  2. call `reader::row_count` and print `"migrating N rows…"` to stderr
  3. call `Calyx::create_vault(name_from_db_filename, panel_template=None)`
     (or open existing if `calyx-path` already exists)
  4. derive or look up `gte_lens_id`; call `Calyx::add_lens` for the GTE slot
     (idempotent: if the lens already exists, no-op)
  5. iterate `reader::stream_rows` in batches of `--batch-size`; for each batch:
     - call `adapter::from_chunk_row` for each row → `Constellation`
     - call `VaultStore::put(cx)` for each (group-committed batch) — idempotent
     - print progress `"migrated N/total…"` to stderr every 1000 rows
  6. after all rows: print `"migration complete: N_written new, N_skipped
     duplicate"` to stderr; exit 0
- [ ] `--dry-run`: runs steps 1–2 and adapter conversion but skips `VaultStore::put`;
  prints what would be written; useful to validate before committing to disk
- [ ] Restartability: `VaultStore::put` returns `CxId` with `new: false` for
  duplicates; the migration counts and reports duplicates without error
- [ ] Error handling: mid-migration `CALYX_LENS_UNREACHABLE` → print error to
  stderr, abort migration, leave partial vault intact (not deleted — the vault
  is partially migrated and restartable); `CALYX_ASTER_TORN_WAL` from a crash
  mid-batch → replay on restart recovers last group-committed batch
- [ ] Every migration write triggers a Ledger entry (A15); the Ledger records the
  `chunk_id` and `database_name` in the entry's metadata

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] integration: create a 10-row in-memory SQLite DB → run migrate → vault
  contains exactly 10 constellations; `VaultStore::get` on each `CxId` returns
  the corresponding `ChunkRow.content` hash in `input_ref`
- [ ] integration: run migrate on the same DB twice → second run exits 0 with
  `"10 duplicate"` in stderr; vault still has exactly 10 constellations (not 20)
- [ ] unit: `--dry-run` on a 5-row DB → no vault directory created on disk;
  stderr shows 5 rows would be migrated
- [ ] unit: `migrate` with `--batch-size 3` on a 10-row DB → `VaultStore::put`
  called in 4 batches (3+3+3+1); all 10 rows present in vault
- [ ] edge: SQLite with 0 rows → migration completes with `"0 new, 0 duplicate"`;
  migration to a path where the vault directory already has an incompatible
  manifest → `CALYX_ASTER_CORRUPT_SHARD` (not silent overwrite)
- [ ] fail-closed: SQLite row with `embedding.len() != 768` →
  `CALYX_LENS_DIM_MISMATCH` on stderr, migration aborts; the rows already written
  before the bad row are preserved (restartable)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the vault directory created at `<calyx-path>` after running `calyx
  migrate vault sample-leapable.db /tmp/migrated.calyx` on aiwonder
- **Readback:**
  - `calyx readback --vault-tree /tmp/migrated.calyx` → directory listing shows
    `cf/`, `wal/`, `ledger/`, `manifest/` present
  - `calyx readback --cf-row /tmp/migrated.calyx --cf base --key <first_cx_id_hex>` →
    non-empty hex dump of the first constellation
  - Count: `sqlite3 sample-leapable.db "SELECT COUNT(*) FROM chunks"` must match
    the migration output `"N new"` count
- **Prove:** the directory structure exists after migration; the first CF row is
  readable and non-empty; row count in the vault equals row count in SQLite;
  re-running migration prints `"N duplicate"` and adds no new CF rows (idempotent)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH64 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
