# PH62 — calyx-cli (vault/lens/ingest/search/readback)

**Stage:** S15 — Interfaces: CLI, MCP, Migration  ·  **Crate:** `calyx-cli`  ·
**PRD roadmap:** A17, A15  ·  **Axioms:** A15, A16, A17

## Objective

Implement the `calyx` binary with every subcommand needed to operate a vault from
the command line: vault creation, lens management, ingest, anchor, search,
intelligence extraction (kernel/bits/guard/provenance), and the **FSV readback
tools** that print raw bytes from Aster column families, WAL records, and Ledger
entries. The primary user is an AI agent (A17); the readback tools are the byte-
level proof instrument used by every other phase's FSV gate — they print actual
persisted bytes, never a green-checkmark harness verdict. Usable from Stage 4
onward; finalizes as engines land.

## Dependencies

- **Phases:** PH24 (RRF/WeightedRRF fusion + provenance hits — search must exist before
  the CLI search command is wired), PH03 (CALYX_* error catalog in calyx-core)
- **Provides for:** PH63 (calyx-mcp builds on the same Calyx engine API), PH64
  (migration tool extends the CLI), PH65 (calyxd healthcheck via CLI), PH71
  (vault swap workflow driven by calyx CLI commands)

## Current state (build off what exists)

`calyx-cli` exists as a working skeleton (`crates/calyx-cli/src/main.rs`, 153
lines). `readback --hex <file>` and `readback --vault-tree <dir>` are
implemented, tested, and green. The `CalyxError` struct with `{code, message,
remediation}` is complete in `calyx-core/src/error.rs` with all 23 PRD-18 codes.
What remains: every other subcommand (`create-vault`, `add-lens`, `retire-lens`,
`park-lens`, `ingest`, `anchor`, `search`, `kernel`, `bits`, `guard`,
`provenance`, `healthcheck`) plus extending `readback` to reach into Aster CF
rows, WAL records, and Ledger entries; structured JSON error output; idempotent
ingest; explain/provenance on search.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-cli/src/main.rs` | Entry point, top-level subcommand dispatch, structured error output |
| `crates/calyx-cli/src/cmd/mod.rs` | Subcommand enum; re-exports |
| `crates/calyx-cli/src/cmd/vault.rs` | `create-vault`, `add-lens`, `retire-lens`, `park-lens`, `list-panel`, `profile-lens` |
| `crates/calyx-cli/src/cmd/ingest.rs` | `ingest` (idempotent, multi-lens), `anchor`, `measure` |
| `crates/calyx-cli/src/cmd/search.rs` | `search` (RRF default, `--explain`, `--provenance`), `kernel-answer` |
| `crates/calyx-cli/src/cmd/intelligence.rs` | `bits`, `kernel`, `guard`, `abundance`, `propose-lens` |
| `crates/calyx-cli/src/cmd/provenance.rs` | `provenance`, `verify-chain`, `anneal-status` |
| `crates/calyx-cli/src/cmd/readback.rs` | Extended `readback`: CF rows, WAL records, Ledger entries (byte dumps) |
| `crates/calyx-cli/src/cmd/healthcheck.rs` | `healthcheck` (engine probe + JSON pass/fail) |
| `crates/calyx-cli/src/error.rs` | CLI error → structured `{code, message, remediation}` JSON; exit-2 on error |
| `crates/calyx-cli/src/output.rs` | Output helpers: JSON, table, hex-dump, byte-exact provenance formatting |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Error wiring and output layer | — |
| T02 | Vault and lens subcommands | T01 |
| T03 | Ingest, anchor, and measure subcommands | T02 |
| T04 | Search with explain and provenance | T03 |
| T05 | Intelligence subcommands (bits/kernel/guard/abundance) | T04 |
| T06 | Provenance, verify-chain, anneal-status | T05 |
| T07 | Readback extension (CF rows / WAL records / Ledger entries) | T01 |
| T08 | Full workflow FSV + healthcheck | T06, T07 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

The full workflow runs end-to-end on aiwonder:
```
calyx create-vault aiwonder-test --panel-template text-default
calyx add-lens aiwonder-test --name gte-768 --runtime tei-http --endpoint http://localhost:8088
calyx ingest aiwonder-test --text "Why does X fail under load?"
calyx anchor aiwonder-test <cx_id> --kind test-pass --value true
calyx search aiwonder-test "fail under load" --explain --provenance
calyx readback <vault.calyx>/cf/base/<cx_id_hex>   # prints raw CF bytes
calyx readback --wal <vault.calyx>/wal/00000001.wal  # prints real WAL record bytes
calyx readback --ledger <vault.calyx>/ledger/00000001 # prints Ledger entry bytes
```
The `readback` commands print the actual persisted bytes that match a direct CF
read (verified by `xxd` cross-check on aiwonder). No harness verdict counts.

## Risks / landmines

- **readback must print bytes, not a harness result:** every FSV gate across all
  stages depends on `calyx readback` producing the raw bytes from the correct
  CF/WAL/Ledger file. Never make it return "OK" — it must dump raw bytes.
- **Structured errors must be machine-readable:** the primary user is an AI agent;
  `{code, message, remediation}` must serialize to JSON on stderr (exit 2). A
  human-only error message defeats A17.
- **Subcommand surface grows with stages:** commands for Oracle, Temporal, and
  Universal data layer are out of scope here — design the dispatch table to be
  additive without requiring structural refactors.
- **≤500 lines per file:** `main.rs` already exists at 153 lines; new commands
  land in `cmd/` submodules, not in main.rs.
