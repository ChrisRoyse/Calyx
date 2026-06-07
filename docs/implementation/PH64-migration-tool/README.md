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

`calyx-cli` crate exists with all subcommands from PH62. `calyx-mcp` is wired.
`calyx migrate vault` is a greenfield subcommand within `calyx-cli`. No Rust
migration code exists yet. The source contract is `vault-sqlite.ts` (Leapable
TypeScript — defines the chunk schema and identifier invariants); this must be
mapped to a Rust interface without copying the TS code.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-cli/src/cmd/migrate.rs` | `migrate vault` subcommand: arg parsing, orchestration |
| `crates/calyx-cli/src/migrate/reader.rs` | SQLite reader: open `.db`, iterate `chunks` table, stream rows |
| `crates/calyx-cli/src/migrate/adapter.rs` | `VaultSqliteAdapter`: maps SQLite row → Constellation; preserves `chunk_id`/`database_name` |
| `crates/calyx-cli/src/migrate/verifier.rs` | Readback verifier: compare Calyx constellation content bytes vs source SQLite row bytes |
| `crates/calyx-cli/src/migrate/mod.rs` | Module facade; `pub use` re-exports |

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
calyx migrate vault /path/to/real-leapable.db /tmp/migrated.calyx
calyx readback --cf-row /tmp/migrated.calyx --cf base --key <cx_id_hex>
```
The content bytes in the Calyx constellation (the `input_ref` / raw content bytes
stored in the base CF row) must be byte-identical to the corresponding content
from the source SQLite `chunks` row. Verified by the verifier's own output AND by
a cross-check `sqlite3 real-leapable.db 'SELECT content FROM chunks WHERE
chunk_id=X'` vs `calyx readback --cf-row … --key <cx_id_hex>`. No harness
assertion counts — read the bytes on aiwonder.

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
