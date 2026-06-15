# PH63 — calyx-mcp (stdio embedded tool surface)

**Stage:** S15 — Interfaces: CLI, MCP, Migration  ·  **Crate:** `calyx-mcp`  ·
**PRD roadmap:** A17, `14` (all sections)  ·  **Axioms:** A15, A16, A17

## Objective

Build the `calyx-mcp` stdio JSON-RPC server that exposes the full tool surface
described in PRD `14 §2` — self-describing, sensible defaults, constraint-over-
procedure, provenance by default. The primary user is an AI agent. The entire
calculus-of-association stack (lenses, DDA, bits, kernel, guard, search,
provenance, self-tuning) is reachable in a handful of typed, self-describing,
provenance-returning tool calls so a multi-lens system is configured, not coded.
Each tool has a typed JSON schema and a one-line "use when" description so an
agent discovers capability without docs. `search` works with one argument; errors
carry `code` + `remediation` so an agent self-corrects.

## Dependencies

- **Phases:** PH62 (calyx-cli and the full Calyx engine API wired — MCP wraps the
  same API), PH24 (search/provenance), PH35 (Ledger — provenance by default)
- **Provides for:** PH65 (calyxd server exposes the same MCP over HTTP), PH71
  (Leapable vault swap driven by MCP tool calls)

## Current state (build off what exists)

`calyx-mcp` is a 9-line stub crate (`crates/calyx-mcp/`). Greenfield from here.
The Calyx engine API used by this crate is the same one wired in PH62 (`calyx-cli`).
The `CalyxError` struct with `{code, message, remediation}` is already in
`calyx-core`. JSON-RPC framing and tool registration are greenfield.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-mcp/src/main.rs` | Stdio server entry point: read newline-delimited JSON-RPC on stdin, write on stdout |
| `crates/calyx-mcp/src/server.rs` | `McpServer`: dispatch loop, tool registry, `initialize` / `tools/list` / `tools/call` handlers |
| `crates/calyx-mcp/src/protocol.rs` | JSON-RPC 2.0 types: `Request`, `Response`, `Error`, `ToolDef`, `ToolCallResult`; serde |
| `crates/calyx-mcp/src/tools/vault.rs` | `create_vault`, `add_lens`, `retire_lens`, `park_lens`, `list_panel`, `profile_lens` |
| `crates/calyx-mcp/src/tools/ingest.rs` | `ingest`, `anchor`, `measure` |
| `crates/calyx-mcp/src/tools/search.rs` | `search`, `kernel_answer`, `neighbors`, `agree`, `disagree`, `define`, `guard_generate`, `traverse`, `skills` |
| `crates/calyx-mcp/src/tools/intelligence.rs` | `abundance`, `bits`, `kernel`, `guard_calibrate`, `guard_check`, `propose_lens` |
| `crates/calyx-mcp/src/tools/provenance.rs` | `provenance`, `answer_trace`, `verify_chain`, `reproduce`, `anneal_status` |
| `crates/calyx-mcp/src/schema.rs` | JSON Schema builder helpers; `ToolSchema` struct |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | JSON-RPC protocol and server scaffold | — |
| T02 | Vault/panel tool group | T01 |
| T03 | Ingest/measure tool group | T02 |
| T04 | Search/navigate tool group (search, kernel_answer, neighbors) | T03 |
| T05 | Search/navigate extensions (agree, disagree, define, guard_generate, traverse, skills) | T04 |
| T06 | Intelligence extraction tool group | T04 |
| T07 | Provenance/ops tool group | T06 |
| T08 | Full MCP workflow FSV (before/after agent run) | T07 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

An agent runs the before/after workflow (PRD `14 §3`) via MCP on aiwonder:
```
# wire calyx-mcp as an MCP server (stdio), then via the MCP client:
tools/call create_vault  {"name":"mcp-test","panel_template":"text-default"}
tools/call add_lens      {"vault":"mcp-test","name":"gte-768","runtime":"tei-http","endpoint":"http://localhost:8088"}
tools/call ingest        {"vault":"mcp-test","input":"Why does X fail under load?"}
tools/call anchor        {"vault":"mcp-test","cx_id":"<id>","kind":"test_pass","value":true}
tools/call search        {"vault":"mcp-test","query":"fail under load"}
tools/call bits          {"vault":"mcp-test","anchor":"test_pass"}
tools/call provenance    {"vault":"mcp-test","cx_id":"<id>"}
```
The MCP responses are read from stdout (the JSON-RPC response bytes). `search`
result contains `provenance` field. An error (e.g. bits with n<50) returns
`{"code":"CALYX_ASSAY_INSUFFICIENT_SAMPLES","remediation":"anchor ≥50 outcomes
first"}` as an actionable MCP error. Evidence = raw stdout JSON bytes captured on
aiwonder.

## Risks / landmines

- **Stdio framing:** MCP over stdio uses newline-delimited JSON-RPC; a debug
  `println!` in the engine code will corrupt the protocol stream. All engine
  logging must go to stderr or a log file, never stdout.
- **Schema accuracy:** tool schemas drive agent behavior; an inaccurate schema
  (wrong required fields, wrong types) causes agent failure without obvious error.
  Each schema is tested by round-tripping a known call through `serde_json`.
- **Sensible defaults:** `search` must work with `{"vault":"…","query":"…"}`
  only — all other fields have defaults (k=10, fusion=rrf, provenance=true,
  guard=off). Never require an agent to specify fusion math.
- **Provenance by default:** every result carries `provenance`; it is not an opt-
  in flag at the MCP layer (A15). The CLI has `--no-provenance` for debug; MCP
  does not expose that escape.
