# Stage 15 — Interfaces: CLI, MCP, Migration (PH62–PH64)

Zero hand-written multi-embedder plumbing — the whole stack reachable through a
small, typed, self-describing CLI + MCP surface, plus the Leapable Vault
migration tool. Lands in `calyx-cli` + `calyx-mcp`. The primary user is an AI
agent (A17). *Usable from Stage 4 onward; finalize as engines land.*

---

## PH62 — calyx-cli (vault/lens/ingest/search/readback)
- **Objective.** The `calyx` binary: create vault, add/retire/park lens, ingest,
  anchor, search, plus the **FSV readback tools** that print bytes.
- **Deps.** PH24.
- **Deliverables.** `calyx` subcommands (`create-vault`, `add-lens`, `ingest`,
  `anchor`, `search`, `kernel`, `bits`, `guard`, `provenance`, `readback`,
  `healthcheck`); structured `{code,message,remediation}` errors.
- **Key tasks.** `readback` prints actual Aster CF / WAL / Ledger bytes for FSV
  (not a green check); idempotent ingest; explain/provenance on search.
- **FSV gate.** the full workflow (create → add_lens → ingest → anchor → search)
  runs on aiwonder; `calyx readback <cx>` prints the real persisted bytes that
  match a direct CF read.
- **Axioms/PRD.** A17, A15, `14 §2`, `28 §2`.

## PH63 — calyx-mcp (stdio embedded tool surface)
- **Objective.** The MCP tool surface (the doctrine §5 "one call" ergonomics) —
  self-describing, sensible defaults, constraint-over-procedure, provenance by
  default.
- **Deps.** PH62.
- **Deliverables.** `calyx-mcp` stdio server exposing the `14 §2` tools
  (vault/panel, ingest/measure, search/navigate, intelligence extraction,
  provenance/ops); JSON-RPC; markdown descriptions + JSON payloads.
- **Key tasks.** typed schema + one-line "use when" per tool; defaults so
  `search` works with one arg; errors carry `code`+`remediation`; idempotent
  ingest.
- **FSV gate.** an agent can run the before/after workflow (PRD `14 §3`) via MCP;
  `search` returns provenance; an error returns an actionable remediation (read
  the MCP responses).
- **Axioms/PRD.** A17, `14` (all), DOCTRINE §5.

## PH64 — Migration tool (sqlite→calyx vault)
- **Objective.** `calyx migrate vault <sqlite> <vault.calyx>` — chunks → 1-slot
  constellations, then lazy panel backfill; verified by row/byte readback.
- **Deps.** PH62.
- **Deliverables.** the migration adapter (the `vault-sqlite.ts` contract →
  Calyx Vault adapter), readback verifier (constellations vs source SQLite rows),
  preserved identifiers (`chunk_id`, `database_name`).
- **Key tasks.** byte-exact-on-content migration; preserve code-contract names;
  port the allowed-direct-import tests.
- **FSV gate.** migrate a **real `.db`** on aiwonder → readback constellations
  vs source SQLite rows = **byte-exact on content** (not a harness verdict).
- **Axioms/PRD.** P11, `15 §5`, A15.

---

## Stage 15 exit
The entire calculus-of-association stack — lenses, DDA, bits, kernel, guard,
search, provenance, self-tuning — is reachable in a dozen typed, self-describing,
provenance-returning CLI/MCP calls, and a real SQLite vault migrates byte-exact —
the "configured, not coded" win and the bridge to Leapable (Stage 19).
