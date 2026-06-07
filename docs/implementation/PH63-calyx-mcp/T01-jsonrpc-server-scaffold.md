# PH63 · T01 — JSON-RPC protocol and server scaffold

| Field | Value |
|---|---|
| **Phase** | PH63 — calyx-mcp (stdio embedded tool surface) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-mcp` |
| **Files** | `crates/calyx-mcp/src/main.rs` (≤500), `crates/calyx-mcp/src/server.rs` (≤500), `crates/calyx-mcp/src/protocol.rs` (≤500), `crates/calyx-mcp/src/schema.rs` (≤500) |
| **Depends on** | — (greenfield; calyx-core for error types) |
| **Axioms** | A16, A17 |
| **PRD** | `dbprdplans/14 §4` (transport), `dbprdplans/18 §7` (wire format) |

## Goal

Establish the MCP JSON-RPC 2.0 framing layer: stdin reader (newline-delimited
JSON), stdout writer, tool registry, and the three mandatory MCP methods
(`initialize`, `tools/list`, `tools/call`). All engine output goes to stdout as
JSON-RPC responses; all debug/log output goes to stderr — a misrouted `println!`
would corrupt the protocol stream. The schema helpers enable each tool module to
register a typed schema + one-line "use when" without repeating framing code.

## Build (checklist of concrete, code-level steps)

- [ ] `protocol.rs`: JSON-RPC 2.0 types with `serde`:
  - `JsonRpcRequest { jsonrpc: "2.0", id: Option<Value>, method: String,
    params: Option<Value> }`
  - `JsonRpcResponse { jsonrpc: "2.0", id: Option<Value>, result: Option<Value>,
    error: Option<JsonRpcError> }`
  - `JsonRpcError { code: i32, message: String, data: Option<Value> }` — Calyx
    errors use code `-32000` with `data: {"calyx_code":"CALYX_*",
    "remediation":"…"}` so agents extract structured remediation
  - `ToolDef { name: String, description: String, use_when: String,
    input_schema: Value }` (the "use when" is the one-line agent hint)
  - `ToolCallResult { content: Vec<ContentBlock> }` where `ContentBlock` is
    `{"type":"text","text":"…"}` carrying JSON payload as a string
- [ ] `schema.rs`: `fn object_schema(props: &[(&str, Value, bool)]) -> Value`
  (name, schema, required); `fn string_schema()`, `fn number_schema()`,
  `fn boolean_schema()`, `fn array_schema(items: Value) -> Value`; used by all
  tool modules to declare typed schemas without repeating boilerplate
- [ ] `server.rs`: `struct McpServer { tools: BTreeMap<String, Box<dyn Tool>> }`;
  `trait Tool { fn def(&self) -> ToolDef; fn call(&self, params: Value) ->
  Result<Value, CalyxError> }`; `fn dispatch(&self, req: JsonRpcRequest) ->
  JsonRpcResponse`; handles `initialize` (returns server info + capabilities),
  `tools/list` (returns all `ToolDef`s), `tools/call` (dispatches to registered
  tool)
- [ ] `main.rs`: read stdin line-by-line; deserialize each line as
  `JsonRpcRequest`; call `server.dispatch(req)`; serialize response + write `\n`
  to stdout; log parse errors to stderr; loop until EOF
- [ ] Error mapping: `Tool::call` returning `Err(CalyxError)` → `JsonRpcError{
  code: -32000, message: calyx_error.message, data: {"calyx_code": code,
  "remediation": remediation}}`; unknown method → `JsonRpcError{code: -32601}`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `JsonRpcRequest` round-trips through serde for both string and integer
  `id` variants; `id: null` round-trips as `None`
- [ ] unit: `dispatch` on `tools/list` with zero registered tools returns
  `{"result":{"tools":[]}}` with the correct `id`
- [ ] unit: `dispatch` on unknown method `"foo"` returns error code `-32601`
- [ ] unit: `dispatch` on `tools/call` for a registered tool that returns
  `Err(CalyxError::assay_insufficient_samples("n=30"))` → response contains
  `"calyx_code":"CALYX_ASSAY_INSUFFICIENT_SAMPLES"` and
  `"remediation":"anchor ≥50 outcomes first"` in `error.data`
- [ ] edge: malformed JSON on stdin line → stderr log, next line processed (server
  does not crash); empty line on stdin → ignored; EOF → clean exit 0
- [ ] fail-closed: a tool that panics is caught (no `unwrap` in dispatch) →
  returns `JsonRpcError{code:-32603}` with `"internal server error"` message;
  never crashes the stdio loop

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the raw stdout bytes written by `calyx-mcp` when driven by a known
  JSON-RPC request sequence on aiwonder
- **Readback:** pipe a `tools/list` request to `calyx-mcp` and capture stdout:
  `echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' |
  ./calyx-mcp 2>/dev/null` → stdout contains valid JSON-RPC response with
  `"result":{"tools":[…]}`; `xxd` the stdout bytes to inspect framing
- **Prove:** stdout bytes form valid JSON-RPC 2.0 responses; stderr is empty for
  a valid request (no debug leakage to stdout); the `id` in the response matches
  the `id` in the request; a `tools/call` for an unknown tool returns error `-32601`
  with nothing on stderr

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH63 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
