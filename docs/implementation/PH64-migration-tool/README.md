# PH64 — Migration tool (sqlite→calyx vault)

**Stage:** S15 — Interfaces: CLI, MCP, Migration  ·  **Crate:** `calyx-cli`  ·
**PRD roadmap:** P11, `15 §5`  ·  **Axioms:** A15, A18

## Objective

Implement `calyx migrate vault <sqlite> <vault.calyx>` — the one-command tool
that migrates a Leapable SQLite/sqlite-vec vault to a Calyx vault. Each row in
the SQLite `chunks` table becomes a 1-slot constellation (the existing 768-d GTE
vector) in the new Aster vault, with lazy panel backfill available afterward.
The migration is verified by a byte-exact-on-content readback: the content bytes
of each constellation must match the source SQLite row's content bytes exactly.
Identifiers `chunk_id` and `database_name` are preserved verbatim. The
`vault-sqlite.ts` code-contract names become the Calyx Vault adapter interface.
The allowed-direct-import tests are ported.

## Dependencies

- **Phases:** PH62 (calyx-cli — `migrate` is a new subcommand; `readback` is the
  verification tool), PH09 (constellation CRUD — ingest writes to Aster), PH18
  (LensId content-addressing — vectors from different models must never be mixed)
- **Provides for:** PH71 (V0→V1→V2 Leapable vault swap is gated on this migration
  tool being proven byte-exact on a real `.db`)

## Current state (build off what exists)

`calyx-cli` now wires a `migrate` command family through the main dispatch path.
The implemented commands are:

```
calyx migrate vault <sqlite.db> <vault.calyx> [--verify] [--backfill-default-panel] [--offline-backfill] [--batch-size <n>]
calyx migrate backfill <sqlite.db> <vault.calyx> [--offline-backfill] [--batch-size <n>]
calyx migrate verify <sqlite.db> <vault.calyx> [--require-backfill]
calyx migrate status <vault.calyx>
calyx migrate readback <sqlite.db> <vault.calyx> <chunk_id>
```

The source contract remains `vault-sqlite.ts` (Leapable TypeScript). The Rust
reader accepts the `chunks(chunk_id, database_name, content, embedding)` schema,
preserves `chunk_id` and `database_name` in constellation metadata, writes the
SQLite GTE vector into slot 0, and can lazily backfill the default text panel
through PH20's persisted `BackfillScheduler`. Backfill writes physical Aster slot
column-family rows for slots 1 through 7 without rewriting the base slot row.

Issue #598 FSV evidence lives on aiwonder at:
`/home/croyse/calyx/data/fsv-issue598-lazy-backfill-20260613T132855Z`.
That run created a real SQLite source DB, migrated it with default-panel
backfill, read SQLite rows back with `sqlite3`, read Calyx migration status and
per-chunk readback, inspected physical Aster CF files for `slot_00` through
`slot_07`, and exercised malformed-schema / malformed-embedding / missing-
backfill verifier edges.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-cli/src/migrate/mod.rs` | `migrate` command family: arg parsing, orchestration, tests |
| `crates/calyx-cli/src/migrate/reader.rs` | SQLite reader: open `.db`, iterate `chunks` table, stream rows |
| `crates/calyx-cli/src/migrate/adapter.rs` | `VaultSqliteAdapter`: maps SQLite row → Constellation; preserves `chunk_id`/`database_name` |
| `crates/calyx-cli/src/migrate/verifier.rs` | Readback verifier: compare Calyx constellation content bytes vs source SQLite row bytes |
| `crates/calyx-cli/src/migrate/backfill.rs` | Default-panel lazy backfill: scheduler batches, slot materialization, offline fallback |
| `crates/calyx-cli/src/migrate/manifest.rs` | Migration manifest, deterministic vault identity, panel/scheduler persistence |
| `crates/calyx-cli/src/migrate/errors.rs` | Migration-specific CLI error codes |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | SQLite reader and chunk schema mapping | — |
| T02 | VaultSqliteAdapter: row → constellation (1-slot) | T01 |
| T03 | Migrate subcommand: orchestration + progress | T02 |
| T04 | Readback verifier: byte-exact content comparison | T03 |
| T05 | Real .db migration FSV on aiwonder | T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Migrate a real Leapable `.db` file on aiwonder:
```
calyx migrate vault /path/to/real-leapable.db /tmp/migrated.calyx --verify --backfill-default-panel
calyx migrate status /tmp/migrated.calyx
calyx migrate readback /path/to/real-leapable.db /tmp/migrated.calyx <chunk_id>
```
The content bytes in the Calyx constellation (the `input_ref` / raw content bytes
stored in the base CF row) must be byte-identical to the corresponding content
from the source SQLite `chunks` row. Verified by the verifier's own output AND by
a cross-check `sqlite3 real-leapable.db 'SELECT content FROM chunks WHERE
chunk_id=X'` vs `calyx migrate readback … <chunk_id>`. For lazy-panel migration,
`status` must show `slot_0` through `slot_7` rows present and physical Aster
`slot_00` through `slot_07` column-family files must be inspected on aiwonder. No
harness assertion counts — read the bytes on aiwonder.

## Risks / landmines

- **Never mix vectors across models** (Leapable invariant, PRD `15 §4`): the
  migrated 768-d GTE vector must land in a slot whose `LensId` is content-addressed
  to the GTE model weights hash. If a second lens is added later, its slot gets a
  different `SlotId`. The `LensId` content-addressing enforces this automatically
  (PH18), but the migration must explicitly assign the correct LensId.
- **Byte-exact on content, not on the vector bytes**: the FSV gate is
  content-byte-exact (the chunk text), not vector-byte-exact (the float array).
  The sqlite-vec float32 encoding may differ from Aster's slot encoding.
- **`chunk_id` / `database_name` are code-contract names**: they appear in
  Leapable's TypeScript API surface (`vault-sqlite.ts`) and must be preserved
  exactly in the Calyx metadata (stored in the `Constellation.input_ref` or as
  scalars). Renaming them would break the control plane.
- **Never persist candidate text** (Leapable invariant, PRD `15 §4`): candidate
  text from the reranker is request-scoped only. The migration reads `chunks.content`
  for grounding but must not store the raw text bytes in a location that persists
  beyond the constellation's `input_ref` (hash + opaque ref, not cleartext).
