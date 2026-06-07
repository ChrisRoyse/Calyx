# PH63 · T08 — Full MCP workflow FSV (before/after agent run)

| Field | Value |
|---|---|
| **Phase** | PH63 — calyx-mcp (stdio embedded tool surface) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-mcp` |
| **Files** | `crates/calyx-mcp/src/main.rs` (finalize, ≤500) |
| **Depends on** | T02, T03, T04, T05, T06, T07 (all tools registered) |
| **Axioms** | A15, A16, A17 |
| **PRD** | `dbprdplans/14 §3` (before/after workflow), `dbprdplans/25_STAGE15_INTERFACES.md` (FSV gate) |

## Goal

Run the full before/after MCP workflow (PRD `14 §3`) on aiwonder: every tool in
the `14 §2` surface is reachable via stdio JSON-RPC; `search` returns provenance;
an error returns an actionable remediation; the entire multi-lens system is
configured via tool calls, not code. This card closes the PH63 GitHub issue.

## Build (checklist of concrete, code-level steps)

- [ ] Finalize `main.rs`: ensure all five tool-group modules (`vault`, `ingest`,
  `search`, `intelligence`, `provenance`) are registered with `McpServer::register`
  at startup; log the registered tool count to stderr at startup (not stdout)
- [ ] Add a `tools/list` integration path: `tools/list` response contains exactly
  the expected 28 tools from PRD `14 §2` by name; each has a non-empty
  `description`, a non-empty `use_when`, and a non-null `input_schema`
- [ ] `initialize` method handler: returns `{"protocolVersion":"2024-11-05",
  "capabilities":{"tools":{}},"serverInfo":{"name":"calyx-mcp","version":
  env!("CARGO_PKG_VERSION")}}` per MCP spec
- [ ] Integration smoke test (`#[test]`): spawn `calyx-mcp` as a subprocess; pipe
  the seven-step workflow from the README FSV gate; assert each response is valid
  JSON-RPC with no `error` field; assert `search` result contains `provenance`;
  assert the `bits` error (n<50) response contains `"CALYX_ASSAY_INSUFFICIENT_SAMPLES"`
  and `"anchor ≥50 outcomes first"` in `data.remediation`
- [ ] Tool count test: `tools/list` returns exactly 28 tool names (count them;
  match against the PRD 14 §2 table verbatim)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] integration: `initialize` → `tools/list` → `create_vault` → `add_lens` →
  `ingest` → `anchor` → `search` — all 7 calls succeed in sequence; no `error`
  field in any response
- [ ] unit: `tools/list` contains all 28 expected tool names from PRD 14 §2
  (exact string match: `calyx.create_vault`, `calyx.add_lens`, `calyx.retire_lens`,
  `calyx.park_lens`, `calyx.list_panel`, `calyx.profile_lens`, `calyx.ingest`,
  `calyx.anchor`, `calyx.measure`, `calyx.search`, `calyx.kernel_answer`,
  `calyx.neighbors`, `calyx.agree`, `calyx.disagree`, `calyx.define`,
  `calyx.guard_generate`, `calyx.traverse`, `calyx.skills`, `calyx.search_skill`,
  `calyx.abundance`, `calyx.bits`, `calyx.kernel`, `calyx.guard.calibrate`,
  `calyx.guard.check`, `calyx.propose_lens`, `calyx.provenance`,
  `calyx.answer_trace`, `calyx.verify_chain`, `calyx.reproduce`,
  `calyx.anneal.status`)
- [ ] error remediation: `bits` with n=30 → response `error.data.remediation` =
  `"anchor ≥50 outcomes first"` byte-exact; `kernel_answer` with ungrounded vault
  → `error.data.remediation` = `"add anchors (grounding_gaps)"` byte-exact
- [ ] idempotency: pipe the `ingest` call twice for same text → second response
  `"new":false`; no duplicate MCP error
- [ ] edge: concurrent requests (two requests pipelined before first response) →
  responses are matched by `id` without mixing; `id: null` (notification) →
  no response written to stdout for that request
- [ ] fail-closed: a `tools/call` for `calyx.search` with provenance containing
  a tampered Ledger seq → `CALYX_LEDGER_CHAIN_BROKEN` (not a silent wrong hash)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the stdout bytes of `calyx-mcp` during the full PRD `14 §3` workflow
  on aiwonder; read them directly from the pipe
- **Readback:** run the complete 7-step before/after workflow on aiwonder:
  ```
  ( echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"calyx.create_vault","arguments":{"name":"mcp-fsv","panel_template":"text-default"}}}'
    echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"calyx.add_lens","arguments":{"vault":"mcp-fsv","name":"gte-768","runtime":"tei-http","endpoint":"http://localhost:8088"}}}'
    echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"calyx.ingest","arguments":{"vault":"mcp-fsv","input":"Why does X fail under load?"}}}'
    echo '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"calyx.anchor","arguments":{"vault":"mcp-fsv","cx_id":"<from_id3>","kind":"test_pass","value":true}}}'
    echo '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"calyx.search","arguments":{"vault":"mcp-fsv","query":"fail under load"}}}'
    echo '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"calyx.bits","arguments":{"vault":"mcp-fsv","anchor":"test_pass"}}}'
    echo '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"calyx.provenance","arguments":{"vault":"mcp-fsv","cx_id":"<from_id3>"}}}' ) |
  ./calyx-mcp 2>mcp-stderr.log | tee mcp-stdout.log
  ```
  Inspect `mcp-stdout.log`: id=5 response has `hits[0].provenance` present;
  id=6 response has `error.data.calyx_code:"CALYX_ASSAY_INSUFFICIENT_SAMPLES"`
  (n<50 after one anchor); id=7 response has `ledger_chain_hash`.
  Inspect `mcp-stderr.log`: no JSON-RPC messages leaked (only startup log).
- **Prove:** stdout bytes contain exactly 7 JSON-RPC response lines; the
  `search` response `provenance.ledger_seq` is non-zero; the `bits` error
  remediation is actionable; `mcp-stderr.log` has no `{` characters (no JSON
  leaked to stderr)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH63 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
