# 19 — MCP API & Tools Reference (calyx-mcp)

**Source files covered:**
- `crates/calyx-mcp/src/lib.rs`
- `crates/calyx-mcp/src/main.rs`
- `crates/calyx-mcp/src/jsonrpc.rs`
- `crates/calyx-mcp/src/protocol.rs`
- `crates/calyx-mcp/src/schema.rs`
- `crates/calyx-mcp/src/server.rs`
- `crates/calyx-mcp/src/tools/mod.rs`
- `crates/calyx-mcp/src/tools/vault.rs`, `tools/vault/lens.rs`, `tools/vault/store.rs`
- `crates/calyx-mcp/src/tools/ingest.rs`, `tools/ingest/anchor.rs`, `tools/ingest/report.rs`
- `crates/calyx-mcp/src/tools/search.rs`, `tools/search/engine.rs`, `tools/search/output.rs`
- `crates/calyx-mcp/src/tools/search/extensions.rs`, `tools/search/extensions/{runtime,render,guard_generate}.rs`
- `crates/calyx-mcp/src/tools/intelligence.rs`, `tools/intelligence/{core,metrics,guard,propose,model}.rs`
- `crates/calyx-mcp/src/tools/provenance.rs`, `tools/provenance/{core,status,ids,quarantine}.rs`
- `crates/calyx-mcp/tests/jsonrpc.rs`, `crates/calyx-mcp/tests/stdio.rs`
- `crates/calyx-mcp/Cargo.toml`
- Cross-check: `docs/dbprdplans/14_MCP_AGENT_INTERFACE.md`

This crate is the MCP (Model Context Protocol) interface for agent-facing Calyx
operations. It is a binary (`[[bin]] name = "calyx-mcp"`, `path = "src/main.rs"`)
plus a library. It depends on `calyx-anneal`, `calyx-aster`, `calyx-core`,
`calyx-ledger`, `calyx-loom`, `calyx-paths`, `calyx-registry`, `calyx-sextant`,
`calyx-ward`, plus `blake3`, `serde`, `serde_json`, `ulid`. There is **no async
runtime, no socket, and no HTTP dependency** — see [§1](#1-wire-protocol-and-transport).

See [01_system_overview.md](01_system_overview.md) and
[20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md) for how this crate
sits in the stack and shares vault resolution with the CLI.

## 1. Wire protocol and transport

### 1.1 Transport: line-delimited JSON-RPC over stdio

`src/main.rs::main` is the entrypoint. It:

1. Builds an empty `McpServer`, calls `calyx_mcp::tools::register_all(&mut server)`;
   on registration failure it prints `calyx-mcp: <code>: <message>` to **stderr**
   and returns `ExitCode::FAILURE`.
2. On success prints `calyx-mcp: registered <N> tools` to stderr.
3. Loops over `stdin.lock().lines()` (newline-delimited). For each line: trims it,
   skips empties, decodes one JSON-RPC request, dispatches it, and writes one
   JSON-RPC response line to **stdout** followed by `flush()`.
4. On EOF returns `ExitCode::SUCCESS`.

**stdout is protocol-only; every diagnostic goes to stderr** so a stray log line can
never corrupt the response stream (confirmed by `tests/stdio.rs::assert_no_json_on_stderr`).
The transport is therefore **stdio only** — there is no socket/TCP/HTTP listener in
this crate. (The planning doc §4 mentions `calyxd` exposing MCP over an HTTP ingress;
that is a *different* binary — not implemented in `calyx-mcp`. See
[20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md).)

**Notifications:** a request whose `id` is absent or JSON `null` is treated as a
notification (`is_notification` in `main.rs`); it is still dispatched but **no response
is written**, per JSON-RPC 2.0.

**Error resilience:** a malformed input line logs `calyx-mcp: <code>: <message>` to
stderr and the loop continues to the next line (it does **not** emit a JSON-RPC error
for an undecodable line, because no `id` could be recovered). I/O errors on stdin/stdout
return `ExitCode::FAILURE`.

### 1.2 Request decode (`src/jsonrpc.rs`)

| Item | Definition |
|---|---|
| `JsonRpcId` | untagged enum: `String(String)` \| `Number(i64)` \| `Null` |
| `JsonRpcRequest` | `{ jsonrpc: String, method: String, params: Option<Value>, id: Option<JsonRpcId> }` |
| `JsonRpcWire` | `Single(JsonRpcRequest)` \| `Batch(Vec<JsonRpcRequest>)` |
| `CALYX_MCP_JSONRPC_INVALID` | `"CALYX_MCP_JSONRPC_INVALID"` — error code for all decode/validation failures |

`decode_jsonrpc_wire(bytes)` parses to a `serde_json::Value`, then: a JSON object →
`Single`; a JSON array → `Batch` (rejecting an **empty** batch); anything else →
error. `decode_jsonrpc_request(bytes)` requires a `Single` (rejects a batch). `main.rs`
uses `decode_jsonrpc_request`, so **batches are decoded by the library but rejected at
the stdio entrypoint**.

`JsonRpcRequest::validate()` (fail-closed) requires: `jsonrpc == "2.0"` exactly;
non-empty trimmed `method`; method must **not** start with `rpc.` (reserved); `params`,
if present, must be a JSON object or array. Every failure becomes a `CalyxError` with
code `CALYX_MCP_JSONRPC_INVALID`, remediation `"send a valid JSON-RPC 2.0 MCP request
object or non-empty batch"`.

### 1.3 Response framing and error mapping (`src/protocol.rs`)

JSON-RPC numeric error codes:

| Constant | Code | Meaning |
|---|---|---|
| `JSONRPC_METHOD_NOT_FOUND` | `-32601` | Unregistered method **or** unknown tool name |
| `JSONRPC_INVALID_PARAMS` | `-32602` | Structurally wrong `tools/call` payload / tool `InvalidParams` |
| `JSONRPC_INTERNAL_ERROR` | `-32603` | Caught tool panic, or result-serialize failure |
| `JSONRPC_CALYX_ERROR` | `-32000` | Implementation-defined: carries a `CalyxError` |

`JsonRpcError` = `{ code: i32, message: String, data: Option<Value> }`. Constructors:
`method_not_found(name)`, `invalid_params(msg)`, `internal(msg)`, and
`from_calyx(&CalyxError)`. **`from_calyx` is the taxonomy bridge** ([§4](#4-error-mapping-to-the-calyxerror-taxonomy)):
it sets `code = -32000`, `message = error.message`, and
`data = { "calyx_code": error.code, "remediation": error.remediation }`. This is the
only place the stable `CALYX_*` code and remediation reach the wire.

`JsonRpcResponse` = `{ jsonrpc: "2.0", result?: Value, error?: JsonRpcError, id: Option<JsonRpcId> }`
— exactly one of `result`/`error` is `Some`; `id` serializes as `null` when absent.

MCP descriptor / content shapes:

| Type | Shape |
|---|---|
| `ToolDef` | `{ name, description, use_when, inputSchema }` (note: `input_schema` field is renamed to `inputSchema`; `use_when` is a Calyx extension, a one-line agent hint) |
| `ContentBlock` | tagged enum, only variant `Text { text: String }` → `{"type":"text","text":...}` |
| `ToolCallResult` | `{ content: Vec<ContentBlock> }`; `ToolCallResult::text(s)` wraps one text block |

A successful `tools/call` returns a `ToolCallResult` whose single text block holds the
tool's JSON payload **serialized as a string** (i.e. the payload is double-encoded:
JSON-as-text inside the MCP content block). See `server.rs::handle_tools_call`.

### 1.4 Input-schema builders (`src/schema.rs`)

Tools declare `inputSchema` with these helpers (draft-07-compatible object schemas):
`string_schema()`, `number_schema()`, `integer_schema()`, `boolean_schema()`,
`array_schema(items)`, and `object_schema(&[(name, sub_schema, required)])`.
`object_schema` preserves property order, and `required` is **always present** (an empty
array when nothing is required). Tool modules additionally hand-roll `enum_string(&[..])`
→ `{"type":"string","enum":[...]}` and `integer_range(min,max)` →
`{"type":"integer","minimum":..,"maximum":..}` for constrained fields.

### 1.5 The MCP handshake and dispatch (`src/server.rs`)

Constants: `MCP_PROTOCOL_VERSION = "2024-11-05"`, `SERVER_NAME = "calyx-mcp"`,
`CALYX_MCP_TOOL_DUPLICATE = "CALYX_MCP_TOOL_DUPLICATE"`.

`McpServer` holds `tools: BTreeMap<String, Box<dyn Tool>>` (ordered by name).
`register(tool)` fails closed with `CALYX_MCP_TOOL_DUPLICATE` if a name already exists.
`dispatch(request)` routes by `method`:

| Method | Handler | Result |
|---|---|---|
| `initialize` | `handle_initialize` | `{ protocolVersion: "2024-11-05", capabilities: { tools: {} }, serverInfo: { name: "calyx-mcp", version: <CARGO_PKG_VERSION = 0.1.0> } }` |
| `tools/list` | `handle_tools_list` | `{ tools: [ToolDef, …] }` (BTreeMap order → alphabetical by name) |
| `tools/call` | `handle_tools_call` | `ToolCallResult` (success) or JSON-RPC error |
| anything else | — | `-32601` method-not-found |

The `initialize` handler does **not** read or validate the client's `params`
(`protocolVersion`/`clientInfo` are ignored); it always echoes the server's fixed
version. There is no `initialized` notification handler (it would simply be a no-op
notification with no reply).

The `Tool` trait: `fn def(&self) -> ToolDef` and `fn call(&self, params: Value) ->
ToolResult<Value>`, where `ToolResult<T> = Result<T, ToolError>` and
`ToolError = InvalidParams(String) | Calyx(CalyxError)` (with `From<CalyxError>`).

### 1.6 `tools/call` dispatch flow

`handle_tools_call` (in `server.rs`):

1. `params.name` must be a non-empty string → else `-32602`.
2. `params.arguments` defaults to `{}` if absent.
3. Look up the tool by name → else `-32601`.
4. Invoke `tool.call(arguments)` inside `catch_unwind(AssertUnwindSafe(..))` so a
   panicking tool cannot kill the stdio loop.
5. Map the outcome:
   - `Ok(Ok(value))` → serialize to a JSON string; on serialize error → `-32603`
     `"serialize tool result"`; else success carrying `ToolCallResult::text(payload)`.
   - `Ok(Err(InvalidParams(m)))` → `-32602` with message `m`.
   - `Ok(Err(Calyx(e)))` → `from_calyx(e)` → `-32000` with `{calyx_code, remediation}`.
   - `Err(panic)` → `-32603` with generic message `"internal server error"` (no leak).

### 1.7 Auth / consent gating

**There is no authentication, authorization, or consent gate in `calyx-mcp`.** Any
client speaking JSON-RPC on stdin can call any registered tool. The only access control
is `CALYX_HOME`: every tool resolves its vault under the directory named by the
`CALYX_HOME` env var, and missing `CALYX_HOME` yields `CALYX_VAULT_ACCESS_DENIED`
(`vault/store.rs::home_dir`). Beyond that, tools enforce per-operation safety
(path-safety checks, calibration prerequisites, idempotency) but not identity. The
planning doc's "Cloudflare Access-gated, loopback bind" applies to a future `calyxd`
server, not this binary.

## 2. The tool registry

`tools::register_all` chains five group registrations in order: `vault`, `ingest`,
`search` (which also calls `search::extensions::register`), `intelligence`, `provenance`.
**Total: 30 tools.** `tests/stdio.rs::EXPECTED_TOOLS` pins exactly these 30 names and
asserts the startup banner reports 30. Grouped:

| Group | Module | Count | Tools |
|---|---|---|---|
| Vault & panel | `vault.rs` | 6 | create_vault, add_lens, retire_lens, park_lens, list_panel, profile_lens |
| Ingest & measure | `ingest.rs` | 3 | ingest, anchor, measure |
| Search & navigate (core) | `search.rs` | 3 | search, kernel_answer, neighbors |
| Search extensions | `search/extensions.rs` | 7 | agree, disagree, define, guard_generate, traverse, skills, search_skill |
| Intelligence | `intelligence.rs` | 6 | abundance, bits, kernel, guard.calibrate, guard.check, propose_lens |
| Provenance & ops | `provenance.rs` | 5 | provenance, answer_trace, verify_chain, reproduce, anneal.status |

All names are prefixed `calyx.`. Two use a dotted sub-namespace: `calyx.guard.calibrate`,
`calyx.guard.check`, `calyx.anneal.status`.

**Shared infrastructure** (all groups): vault resolution via
`vault/store.rs::resolve_vault_info` — accepts a vault **name**, a `VaultId` string, or
a **direct filesystem path** ending in a vault id; reads `$CALYX_HOME/vaults/index.json`;
`vault_salt(id, name) = "calyx-cli-vault:{id}:{name}"`. Every tool decodes arguments with
`serde_json::from_value` (a deserialize failure → `InvalidParams` → `-32602`).

## 3. Tool reference (every tool)

For each tool: parameters (name / JSON type / required / default / valid values /
notes), the returned JSON structure, side effects, and error conditions. "Returns"
describes the JSON object inside the `tools/call` text block.

### 3.1 Vault & panel tools (`tools/vault.rs`)

#### `calyx.create_vault`
Create a durable Calyx vault. *Use when:* start a new database.

| Param | Type | Req | Default | Valid values | Notes |
|---|---|---|---|---|---|
| `name` | string | yes | — | path-safe | must not contain whitespace, `/`, `\`, or be `.`/`..`/empty (`validate_path_safe`) |
| `panel_template` | string | no | `text-default` | `text-default`, `code-default`, `civic-default`, `media-default` | selects the registry default panel |

**Returns:** `{ vault_id, name, panel_template }`.
**Side effects:** mints a new `VaultId` (ULID); creates `$CALYX_HOME/vaults/<vault_id>/`
as a durable Aster vault (`AsterVault::new_durable`) with the chosen panel; appends an
entry to `$CALYX_HOME/vaults/index.json` (sorted by name) via `write_index`.
**Errors:** `InvalidParams` if name not path-safe, name already in index, vault dir
already exists, or unknown template; `CALYX_VAULT_ACCESS_DENIED` if `CALYX_HOME` unset;
disk/Aster errors propagate as `CalyxError`.

#### `calyx.add_lens`
Add one frozen lens to a vault panel. *Use when:* add a measurement axis.

| Param | Type | Req | Default | Valid values | Notes |
|---|---|---|---|---|---|
| `vault` | string | yes | — | — | name/id/path |
| `name` | string | yes | — | path-safe | lens name = axis key |
| `runtime` | string | yes | — | `tei-http`, `onnx`, `candle`, `algorithmic` | underscores normalized to dashes |
| `endpoint` | string | no | — | — | required for `tei-http`; model_id for onnx/candle |
| `weights` | string | no | — | — | filesystem path; hashed for the frozen contract |
| `shape` | string | no | `Dense(768)` for tei/onnx/candle | `Dense(<n>)` / `Sparse(<n>)`, `n>0` | algorithmic must match its byte-features shape |

**Returns:** `{ lens_id, slot_id, name, state: "active" }`.
**Side effects:** builds a `FrozenLensContract` (reads `weights` file if given),
registers the lens in the vault's `Registry` if absent, runs `SwapController::add_lens`,
and persists panel+registry via `persist_vault_panel_state`. **No external network call
is made at add time** — `onnx`/`candle` lenses are *declared* (their `measure` returns
`CALYX_LENS_UNREACHABLE` in-process; see `lens.rs::DeclaredLens`); `tei-http` validates
only the endpoint string.
**Errors:** `InvalidParams` (bad name/shape/runtime); `CALYX_LENS_UNREACHABLE`
(`tei-http` missing endpoint, or weights file unreadable); `CALYX_LENS_DIM_MISMATCH`
(shape mismatch / non-dense where dense required); vault-resolution errors.

#### `calyx.retire_lens` / `calyx.park_lens`
Retire (permanent) or park (sideline) a panel slot. *Use when:* drop / sideline a lens.
Both share `lifecycle_def` and `set_lens_state`.

| Param | Type | Req | Notes |
|---|---|---|---|
| `vault` | string | yes | — |
| `slot` | integer | yes | `u16` slot id; must exist in the panel |

**Returns:** `{ status: "retired"|"parked", slot }`.
**Side effects:** `SwapController::retire_lens`/`park_lens`, then
`persist_vault_panel_state`.
**Errors:** `CALYX_VAULT_ACCESS_DENIED` if slot does not exist; controller errors propagate.

#### `calyx.list_panel`
List panel slots. *Use when:* see lenses, their bits signal, and state.

| Param | Type | Req |
|---|---|---|
| `vault` | string | yes |

**Returns:** `{ slots: [ { slot, name, state, bits, ci, lens_id } ] }` — `state` ∈
`active|parked|retired`; `bits` is the max stored `bits_about` value for the slot (or
`null`); `ci` is `[low, high]` or `null`.
**Side effects:** read-only (`load_vault_panel_state`).
**Errors:** vault-resolution errors.

#### `calyx.profile_lens`
Profile a candidate lens without committing it. *Use when:* get a capability card first.

| Param | Type | Req | Default | Valid values |
|---|---|---|---|---|
| `runtime` | string | yes | — | `tei-http`, `onnx`, `candle`, `algorithmic` |
| `endpoint` | string | no | — | — |
| `weights` | string | no | — | — |
| `probe` | string | no | 3 built-in probes (`calyx profile alpha/beta/gamma`) | path to a JSONL probe file |

**Returns:** a `CapabilityCard` (from `calyx-registry::profile_lens`).
**Side effects:** builds the lens in a throwaway in-memory `Registry`; reads the probe
file if a path is given. No vault, no persistence.
**Errors:** `InvalidParams` (bad runtime, empty/invalid probe set);
`CALYX_LENS_UNREACHABLE` (unreadable weights/probe file); profiling errors propagate.

### 3.2 Ingest & measure tools (`tools/ingest.rs`)

#### `calyx.ingest`
Ingest text into a vault. *Use when:* store data → constellation (auto multi-lens,
idempotent). `input` and `batch` are **mutually exclusive**; exactly one is required.

| Param | Type | Req | Notes |
|---|---|---|---|
| `vault` | string | yes | — |
| `input` | string | no* | single non-empty text |
| `batch` | array&lt;string&gt; | no* | non-empty list of non-empty texts |

**Returns:** for a single text, an `IngestReport` `{ cx_id, new, ledger_seq }`; for a
batch, `{ results: [IngestReport, …] }`.
**Side effects:** measures the text across all active text-compatible lenses, computes a
content-addressed `cx_id`; **idempotent** — already-present constellations are not
re-stored. New constellations are written (`vault.put` / `put_batch`) then `flush()`ed;
each new one carries an Ingest ledger entry. A *repeat* ingest of an existing cx appends
an idempotent retry ledger row (`mode: "mcp-idempotent-ingest"`) subject to
`RedactionPolicy::check_payload`.
**Errors:** `InvalidParams` (both/neither of input|batch, empty text/batch, or panel has
no active text slots); `CALYX_LENS_UNREACHABLE` if **all** applicable lens runtimes are
unreachable; `CALYX_ASTER_CORRUPT_SHARD` on encode failure.

#### `calyx.anchor`
Attach a grounded outcome to a stored constellation. *Use when:* attach test pass /
thumbs / label.

| Param | Type | Req | Default | Valid values |
|---|---|---|---|---|
| `vault` | string | yes | — | — |
| `cx_id` | string | yes | — | must already exist in the vault |
| `kind` | string | yes | — | `test_pass`, `thumbs_up`, `thumbs_down`, `speaker_match`, `style_hold`, `label` |
| `label` | string | no | — | required (non-empty) iff `kind == "label"` |
| `value` | boolean \| number | yes | — | `oneOf` bool/finite-number; `thumbs_up`→`true`, `thumbs_down`→`false` regardless of value |
| `confidence` | number | no | `1.0` | finite, within `[0,1]` |
| `source` | string | no | `"calyx-mcp"` | — |

**Returns:** `{ status: "anchored", cx_id, ledger_seq }`.
**Side effects:** `vault.anchor_with_ledger_entry` writes the anchor + an Ingest-kind
ledger entry (`mode: "mcp-anchor"`, `anchor_kind: <key>`), then `flush()`. Payload
passes `RedactionPolicy::check_payload`.
**Errors:** `InvalidParams` (bad cx_id parse, unknown kind, missing/empty label for
`label`, non-bool/non-finite value, out-of-range confidence);
`CALYX_VAULT_ACCESS_DENIED` if cx_id does not exist; redaction/Aster errors propagate.

#### `calyx.measure`
Measure text without storing it. *Use when:* guard a candidate.

| Param | Type | Req |
|---|---|---|
| `vault` | string | yes |
| `input` | string | yes (non-empty) |

**Returns:** a constellation report (`ingest/report.rs::constellation_report`):
`{ cx_id, vault_id, panel_version, created_at, modality, input_ref, slots:[{slot,name,state,lens_id,vector}], scalars, anchors, flags }`.
**Side effects:** read-only — opens the vault and panel, measures, but does **not** write.
**Errors:** `InvalidParams` (empty input, no active text slots);
`CALYX_LENS_UNREACHABLE` if all applicable lenses unreachable; vault-resolution errors.

### 3.3 Search & navigate — core (`tools/search.rs`, `search/engine.rs`)

#### `calyx.search`
Everyday multi-lens search. *Use when:* RRF-default, provenance-attached search.

| Param | Type | Req | Default | Valid values | Notes |
|---|---|---|---|---|---|
| `vault` | string | yes | — | — | — |
| `query` | string | yes | — | non-empty | — |
| `k` | integer | no | `10` | `1..=1000` | result cap (`MAX_K=1000`) |
| `fusion` | string | no | `rrf` | `rrf`, `weighted_rrf`, `single_lens`, `kernel_first`, `pipeline` | dash/underscore both accepted |
| `guard` | string | no | `off` | `off`, `in_region` | see error below |
| `explain` | boolean | no | `false` | — | adds `per_lens` breakdown |
| `fresh` | boolean | no | `false` | — | **parsed but unused** (`_fresh_requested`) |
| `filter` | object | no | — | `QueryFilters` JSON | must be a JSON object; validated |

**Returns:** `{ hits: [ { rank, cx_id, score, per_lens?, guard?, provenance:{ledger_seq,chain_hash} } ] }`
(`search/output.rs`). `per_lens` present only with `explain=true`; `guard` is `null`
here (only populated when a `guard_tau` is supplied, which `search` does not).
**Side effects:** read-only. Internally: loads all docs, **verifies the ledger
hash-chain before attaching provenance** (`verify_ledger_before_provenance`), measures
query vectors over active text lenses, builds per-slot ad-hoc indexes (HNSW for dense,
Inverted for sparse, MaxSim for multi; `HNSW_SEED = 0x0050483633543034`), fuses, attaches
stored provenance, renumbers, truncates to `k`.
**Errors:** if `guard == in_region` → **always** `CALYX_GUARD_PROVISIONAL` ("search guard
is not calibrated for this vault") — in-region guarding is not wired into search;
`InvalidParams` (bad fusion/guard/k, non-object filter, empty query, no active lens for
`single_lens`); `CALYX_STALE_DERIVED` (no indexable query/stored vectors);
`CALYX_LEDGER_CHAIN_BROKEN`/`CALYX_LEDGER_CORRUPT` if the provenance chain is broken.
Empty doc set returns `{hits:[]}` (no error).

#### `calyx.kernel_answer`
Answer via the grounded kernel skeleton. *Use when:* grounded kernel answer.

| Param | Type | Req | Default | Notes |
|---|---|---|---|---|
| `vault` | string | yes | — | — |
| `query` | string | yes | — | non-empty |
| `anchor` | string | no | — | anchor-kind selector; `label:<x>` form supported (`parse_anchor_kind`) |
| `explain` | boolean | no | `false` | — |

**Returns** (`KernelAnswerOut`): `{ answer, kernel_cx_ids:[…], recall, gaps:[…] }`. Runs
an internal `kernel_first` search (k=10, guard off) then `engine::kernel_report` over
the grounded docs (those carrying the requested anchor, or any anchor if none given).
**Side effects:** read-only.
**Errors:** `InvalidParams` (empty query, unknown anchor);
`CALYX_KERNEL_UNGROUNDED` if there are no grounded anchors; search errors above;
`CALYX_ASTER_CORRUPT_SHARD` on encode failure.

#### `calyx.neighbors`
Per-lens neighbors of a stored constellation. *Use when:* known-cx neighborhood.

| Param | Type | Req | Default | Valid values |
|---|---|---|---|---|
| `vault` | string | yes | — | — |
| `cx_id` | string | yes | — | must exist |
| `slot` | integer | no | all slots | `u16` slot id |
| `k` | integer | no | `10` | `1..=1000` |

**Returns:** `{ neighbors: [ { cx_id, score, slot } ] }` sorted by score desc then slot
then cx_id, truncated to `k`. Per indexable slot of the seed, builds an index over the
corpus and searches; excludes/keeps results per slot.
**Side effects:** read-only.
**Errors:** `InvalidParams` (bad cx_id, bad k); `CALYX_VAULT_ACCESS_DENIED` if cx_id
absent; `CALYX_STALE_DERIVED` ("no indexable stored vectors") when a specific `slot` was
requested but yields nothing.

### 3.4 Search extensions (`tools/search/extensions.rs`)

These tools build an in-memory `NavRuntime` (`extensions/runtime.rs::load_runtime`):
loads all docs, builds a `SearchEngine` (HNSW/Inverted/MaxSim indexes over the corpus)
and an `AssocGraph` (chain edges by `created_at`, cosine-derived weights). All are
**read-only** (except `agree`/`disagree` cross-lens, which materializes agreement
xterms — see below).

#### `calyx.agree` / `calyx.disagree`
Find constellations consistent with / anomalous relative to a stored one.

| Param | Type | Req | Default | Notes |
|---|---|---|---|---|
| `vault` | string | yes | — | — |
| `cx_id` | string | yes | — | must exist |
| `slot` | integer | no | cross-lens | if omitted, cross-lens consensus; else single-slot |

**Returns:** `{ constellations: [ { cx_id, score, slot } ] }` (top `CONSENSUS_K = 5`).
Cross-lens path uses `calyx_sextant::agree`/`disagree`; on
`CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES` it **falls back** to single-slot consensus on
the first indexable slot. `disagree` sorts ascending by score.
**Side effects:** the cross-lens path calls `xterms::materialize_agreement_xterms` over
the vault/docs (writes derived agreement cross-terms); single-slot path is read-only.
**Errors:** `InvalidParams` (bad cx_id); `CALYX_VAULT_ACCESS_DENIED` (cx absent);
`CALYX_STALE_DERIVED` if no indexable slot exists; sextant errors propagate.

#### `calyx.define`
Cross-lens definition for a lens coordinate.

| Param | Type | Req | Notes |
|---|---|---|---|
| `vault` | string | yes | — |
| `lens` | integer | yes | `u16` slot id |
| `index` | integer | yes | `usize` index into the doc map |

**Returns:** `{ definition: { cx_id, slots:[{slot, vector:{kind,…}}] } }` or an
`empty_definition` `{ lens, index, cx_id:null, slots:[] }` when the index is out of range
or `calyx_sextant::define` fails.
**Side effects:** read-only.
**Errors:** `InvalidParams` (decode); vault-resolution errors. Out-of-range index is
**not** an error — returns the empty shape.

#### `calyx.guard_generate`
Identity-locked generation gate. *Use when:* accept generated text only if inside
calibrated Gτ slots.

| Param | Type | Req | Notes |
|---|---|---|---|
| `vault` | string | yes | — |
| `candidate_text` | string | yes | non-empty |
| `identity_cx` | string | no | defaults to the first doc's cx_id |

**Returns:** `{ verdict: "pass"|"ood", tau, distance, identity_cx }`. Loads the
calibrated guard profile from CF `Guard` key `profile\0default`; derives identity
required-slot vectors and a hashed text vector for the candidate
(`calyx_core::content_address`), then `calyx_ward::guard`.
**Side effects:** read-only.
**Errors:** `InvalidParams` (empty candidate, bad identity cx);
`CALYX_GUARD_PROVISIONAL` if no calibrated profile;
`CALYX_VAULT_ACCESS_DENIED`/`CALYX_STALE_DERIVED` (missing identity/dense slots);
`CALYX_ASTER_CORRUPT_SHARD` (profile decode); ward errors mapped (`CALYX_GUARD_OOD`, etc.).

#### `calyx.traverse`
Asymmetric/causal walk of the association graph.

| Param | Type | Req | Valid values |
|---|---|---|---|
| `vault` | string | yes | — |
| `cx_id` | string | yes | must exist |
| `direction` | string | yes | `forward`, `backward`, `both` |
| `hops` | integer | yes | `1..=MAX_TRAVERSE_HOPS` (from `calyx_sextant`) |

**Returns:** `{ path: [ { cx_id, hop, direction, score, via } ] }`.
**Side effects:** read-only.
**Errors:** `InvalidParams` (bad cx_id, bad direction, hops out of range);
`CALYX_VAULT_ACCESS_DENIED` (cx absent); sextant errors propagate.

#### `calyx.skills`
Hierarchical skill tree for a vault.

| Param | Type | Req |
|---|---|---|
| `vault` | string | yes |

**Returns:** `{ skill_tree: <SkillTree> }` (clustering params: min_cluster_size 2,
min_samples 1, max_constellations 2048, allow_single_cluster true).
**Side effects:** read-only. **Errors:** sextant `skills` errors propagate.

#### `calyx.search_skill`
Search within a named skill scope.

| Param | Type | Req | Notes |
|---|---|---|---|
| `vault` | string | yes | — |
| `skill` | string | yes | non-empty; must exist in the tree |
| `query` | string | yes | non-empty |

**Returns:** `{ hits: [SearchHitOut, …] }` (k=10, no explain). Returns `{hits:[]}` if the
skill is unknown or no usable query vector exists.
**Side effects:** read-only. **Errors:** `InvalidParams` (empty skill/query); sextant errors.

### 3.5 Intelligence tools (`tools/intelligence.rs`)

Shared helpers in `intelligence/core.rs`: `load_context` (vault + panel),
`load_docs`, `parse_anchor`, `write_json_row` (writes a CF row **and flushes**),
`read_json_row`, `row_exists`. Row keys are `<prefix>\0<subject>` (`model.rs`).

#### `calyx.abundance`
DDA abundance report.

| Param | Type | Req |
|---|---|---|
| `vault` | string | yes |

**Returns** (`AbundanceOut`): `{ n, pairs, materialized, n_eff, dpi_ceiling, panel_size }`
where `pairs = C(n,2)`, `n_eff = materialized/n`, `dpi_ceiling = log2(n+1)`, computed over
active slots.
**Side effects:** read-only. **Errors:** vault-resolution errors.

#### `calyx.bits`
Per-lens signal and panel sufficiency. *Use when:* deficit attribution.

| Param | Type | Req | Default |
|---|---|---|---|
| `vault` | string | yes | — |
| `anchor` | string | yes | — |
| `explain` | boolean | no | `false` |

**Returns** (`BitsOut`): `{ anchor, panel_sufficiency, n, dpi_ceiling, per_slot:[{slot,
name, bits, ci:[lo,hi], estimator:"centroid_cosine_v1", state, low_signal}], explain? }`.
With `explain`, adds `{ positive_anchor_count, comparison_count, persisted_cf:"assay",
persisted_key_hex }`. Bits = centroid-gap cosine: `(1 - cos(pos_centroid, neg_centroid))/2`.
**Side effects:** **writes** the report to CF `Assay` (key `bits\0<anchor>`) and flushes.
**Errors:** `CALYX_ASSAY_INSUFFICIENT_SAMPLES` (`< MIN_ANCHORS = 50` anchored outcomes;
remediation "anchor ≥50 outcomes first"); `CALYX_ASSAY_LOW_SIGNAL` (no active slots, or
all slots `< 0.05` bits); `CALYX_ASSAY_REDUNDANT` (any slot pair `corr > 0.6`);
`InvalidParams` (unknown anchor).

#### `calyx.kernel`
Build or read the grounding kernel.

| Param | Type | Req | Default | Notes |
|---|---|---|---|---|
| `vault` | string | yes | — | — |
| `anchor` | string | no | any-anchor | restricts grounding to a kind |
| `rebuild` | boolean | no | `false` | force re-write even if a row exists |

**Returns** (`KernelOut`): `{ kernel_size, recall, total_cx, kernel_cx_ids:[…],
grounding_gaps:[…] }`. Kernel budget = `ceil(total/100)` (≥1).
**Side effects:** **writes** CF `Kernel` (key `kernel\0<anchor|all>`) + flush, but only
when `rebuild=true` or no existing row.
**Errors:** `CALYX_KERNEL_UNGROUNDED` (no grounded anchors); `InvalidParams` (unknown
anchor).

#### `calyx.guard.calibrate`
Calibrate a Gτ boundary. *Use when:* calibrate the Gτ boundary for a domain.

| Param | Type | Req | Notes |
|---|---|---|---|
| `vault` | string | yes | — |
| `domain` | string | yes | — |
| `set` | string | yes | filesystem **path** to a calibration JSONL file |
| `target_far` | number | yes | must be finite |

**Returns** (`GuardProfileOut`): `{ domain, tau, far, frr, n_corpus,
calibration_corpus_size, blocked_injection_rate, per_slot_tau:[{slot,tau}] }`.
Calibration JSONL rows: `{ slot?, score, good?|class|label|kind }` — class strings
`good/pass/match/identity/clean` ⇒ good; `bad/fail/ood/injection/attack/reject` ⇒ bad.
Uses a fixed `GuardId` UUID `018f48a4-9a79-74d2-8a5c-9ad7f6b8c101`; slot kind derived from
`target_far` (Identity/Content/Stylistic).
**Side effects:** **writes two** CF `Guard` rows — `profile\0<domain>` and the default
`profile\0default` — and flushes (via `write_json_row`).
**Errors:** `InvalidParams` (non-finite target_far, empty/malformed calibration set,
unknown class, missing score); `CALYX_DISK_PRESSURE` (unreadable set file);
ward errors mapped (`CALYX_GUARD_PROVISIONAL`, `CALYX_GUARD_OOD`, …).

#### `calyx.guard.check`
Apply a calibrated Gτ boundary. `cx_id` and `text` are mutually exclusive; one required.

| Param | Type | Req | Notes |
|---|---|---|---|
| `vault` | string | yes | — |
| `cx_id` | string | no* | check a stored constellation's slots |
| `text` | string | no* | hash a candidate text against identity-slot dims |

**Returns** (`GuardCheckOut`): `{ verdict:"pass"|"ood", tau, distance }`.
**Side effects:** read-only (loads the default guard profile from CF `Guard`).
**Errors:** `InvalidParams` (both/neither cx_id|text, bad cx_id);
`CALYX_GUARD_PROVISIONAL` (no calibrated profile);
`CALYX_VAULT_ACCESS_DENIED`/`CALYX_STALE_DERIVED` (missing identity/dense slots); ward errors.

#### `calyx.propose_lens`
Propose a lens to close a sufficiency gap.

| Param | Type | Req |
|---|---|---|
| `vault` | string | yes |
| `anchor` | string | yes |

**Returns** (`ProposeLensOut`): `{ name, rationale, predicted_bits_gain, runtime_hint,
estimated_cost:"zero external cost", candidate }`. Computes a `DeficitMap`
(entropy − mutual-info) and calls `calyx_anneal::synthesize`.
**Side effects:** **writes** CF `AnnealOperators` (key `propose-lens\0<anchor>`) + flush.
**Errors:** `InvalidParams` (unknown anchor, candidate-serialize failure); anneal errors.

### 3.6 Provenance & ops tools (`tools/provenance.rs`)

`open_ledger_view` opens an `AsterLedgerCfStore`; a `CALYX_LEDGER_CORRUPT` that signals
"not an Aster vault / requires real Aster ledger state" is remapped to
`CALYX_ASTER_CORRUPT_SHARD`.

#### `calyx.provenance`
Full lineage of a constellation.

| Param | Type | Req | Notes |
|---|---|---|---|
| `vault` | string | yes | — |
| `cx_id` | string | yes | must exist |

**Returns** (`LineageOut`): `{ cx_id, ingest_seq, ledger_chain_hash,
lens_measures:[{slot,lens_id,measured_at}], anchors:[{kind,ledger_seq}] }`. Verifies the
stored base provenance ref matches a ledger entry before reporting.
**Side effects:** read-only. **Errors:** `InvalidParams` (bad cx_id);
`CALYX_VAULT_ACCESS_DENIED` (cx absent); `CALYX_LEDGER_CHAIN_BROKEN` (base ref mismatch);
`CALYX_LEDGER_CORRUPT` (missing ingest/anchor rows).

#### `calyx.answer_trace`
Full lineage of a kernel answer or search result. **No `vault` param** — scans all vault
dirs under `$CALYX_HOME/vaults`.

| Param | Type | Req |
|---|---|---|
| `answer_id` | string | yes (hex) |

**Returns** (`AnswerTraceOut`): `{ answer_id, complete, trusted, answer_seq, kernel_seq,
guard_seq, retrieval_steps:[{hop,cx_id,from_cx_id,score,lens_id,ledger_seq}],
kernel_cx_ids, ledger_refs:[{role,seq,chain_hash}], fusion_weights, guard_result,
freshness_ts, warnings }`.
**Side effects:** read-only across all vaults. **Errors:** `InvalidParams` (bad answer_id);
`CALYX_VAULT_ACCESS_DENIED` (not found in any vault); ledger errors.

#### `calyx.verify_chain`
Tamper-check the ledger hash-chain over a range.

| Param | Type | Req | Default |
|---|---|---|---|
| `vault` | string | yes | — |
| `from_seq` | integer | no | `0` |
| `to_seq` | integer | no | chain end (max seq + 1) |

**Returns** (`VerifyChainOut`): `{ status:"ok", checked, break_at:null }` on success.
**Side effects:** read-only. **Errors:** `InvalidParams` (`from_seq > to_seq`);
`CALYX_LEDGER_CHAIN_BROKEN` (`VerifyResult::Broken`); `CALYX_LEDGER_CORRUPT`
(`VerifyResult::Corrupt`); ledger-open remap as above.

#### `calyx.reproduce`
Replay a claim to verify bit-parity.

| Param | Type | Req |
|---|---|---|
| `vault` | string | yes |
| `answer_id` | string | yes |

**Returns** (`ReproduceOut`): `{ bit_parity:true, original_hash, reproduced_hash }` — only
when parity holds. Reads the latest `Admin`/`reproduce_v1`-tagged ledger row for the
answer and compares blake3 hashes of `original_hits` vs `reproduced_hits`.
**Side effects:** read-only. **Errors:** when parity fails →
`CALYX_REPRODUCE_DRIFT_EXCEEDED` (the tool turns a non-parity report into an error);
`CALYX_REPRODUCE_NONDETERMINISTIC` (answer exists but no reproduce row);
`CALYX_VAULT_ACCESS_DENIED` (answer not found); `CALYX_LEDGER_CORRUPT` (malformed payload).

#### `calyx.anneal.status`
Inspect self-optimization state.

| Param | Type | Req |
|---|---|---|
| `vault` | string | yes |

**Returns** (`AnnealStatusOut`): `{ phase:"stable"|"tuning"|"healing", tripwires:[{name,
state}], proposals:[{type,rationale,name}], last_soak_at, p99_latency_ms,
health:[{component,state,updated_at}], recent_changes:[{seq,action,ts,description}] }`
(recent_changes capped at last 16 Anneal ledger entries).
**Side effects:** read-only (reads tripwire config file + CFs `AnnealOperators`,
`AnnealHealth` + Anneal ledger entries).
**Errors:** `CALYX_STALE_DERIVED` if there is no tripwire/proposal/health/anneal state at
all; ledger/anneal-decode errors propagate.

## 4. Error mapping to the CalyxError taxonomy

The two-layer model: tools return `ToolResult<Value>` = `Result<Value, ToolError>`, and
`ToolError` is exactly `InvalidParams(String)` or `Calyx(CalyxError)`. `handle_tools_call`
maps:

| Layer | Source | Wire result |
|---|---|---|
| `ToolError::InvalidParams` | argument-shape / validation failures inside tools | `-32602` (`message` only, no `data`) |
| `ToolError::Calyx(CalyxError)` | any domain `CalyxError` (`code` + `message` + `remediation`) | `-32000`, `data = {calyx_code, remediation}` |
| panic | `catch_unwind` in dispatch | `-32603` `"internal server error"` |
| serialize failure | result encode | `-32603` |
| unknown tool / method | dispatch | `-32601` |
| line decode / `validate()` | `jsonrpc.rs` (pre-dispatch, in `main.rs`) | logged to stderr, no response |

A `CalyxError` is a struct `{ code: &'static str, message: String, remediation:
&'static str }` (defined in `calyx-core`; see [05_core.md](05_core.md)). `calyx-mcp`
preserves the `code` and `remediation` verbatim through `from_calyx`, so agents read
`error.data.calyx_code` and `error.data.remediation` to self-correct (e.g.
`CALYX_ASSAY_INSUFFICIENT_SAMPLES` → `"anchor ≥50 outcomes first"`, verified in
`protocol.rs` and `jsonrpc.rs` tests).

`CALYX_*` codes observed flowing through the tools in this crate (the union, not the full
core catalog): `CALYX_VAULT_ACCESS_DENIED`, `CALYX_LENS_UNREACHABLE`,
`CALYX_LENS_DIM_MISMATCH`, `CALYX_ASTER_CORRUPT_SHARD`, `CALYX_DISK_PRESSURE`,
`CALYX_STALE_DERIVED`, `CALYX_GUARD_PROVISIONAL`, `CALYX_GUARD_OOD`,
`CALYX_ASSAY_INSUFFICIENT_SAMPLES`, `CALYX_ASSAY_LOW_SIGNAL`, `CALYX_ASSAY_REDUNDANT`,
`CALYX_KERNEL_UNGROUNDED`, `CALYX_LEDGER_CHAIN_BROKEN`, `CALYX_LEDGER_CORRUPT`,
`CALYX_REPRODUCE_DRIFT_EXCEEDED`, `CALYX_REPRODUCE_NONDETERMINISTIC`,
`CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES`. Two codes are **MCP-local** (kept out of the
closed core catalog): `CALYX_MCP_JSONRPC_INVALID` (decode/validate) and
`CALYX_MCP_TOOL_DUPLICATE` (duplicate registration at startup — a programming error).

The Ward layer maps `WardError` → `CalyxError` via `ward_to_tool` (in both
`intelligence/guard.rs` and `extensions/guard_generate.rs`), using the ward `code()` and a
hand-chosen remediation per code.

## 5. Cross-check vs. planning doc (`docs/dbprdplans/14_MCP_AGENT_INTERFACE.md`)

The implemented surface matches the plan's grouped table closely, with these concrete
deltas:

- **Tool count:** plan lists tools in slash-grouped shorthand; the code registers exactly
  **30** discrete tools (the plan's `retire_lens / park_lens`, `neighbors / agree /
  disagree`, `skills / search_skill`, `provenance / answer_trace`,
  `verify_chain / reproduce`, `guard.calibrate / guard.check` are each distinct tools).
- **Transport:** plan §4 describes both embedded stdio MCP **and** a `calyxd` HTTP-ingress
  server. Only **stdio** exists in this crate; there is no socket/HTTP code here.
- **`search.guard=in_region`:** the plan implies in-region guarding on search; the code
  **always rejects** it with `CALYX_GUARD_PROVISIONAL` (not wired). Guarding is reachable
  via the separate `guard.check` / `guard_generate` tools.
- **`search.fresh`:** advertised in the schema but the value is parsed and discarded
  (`_fresh_requested`); search always reads the current snapshot.
- **`add_lens` for onnx/candle:** lenses are *declared*, not executed in-process; their
  `measure()` returns `CALYX_LENS_UNREACHABLE`, so ingest/measure/search over an
  onnx/candle lens fall back to `Absent`/degraded unless a runtime is present.
- **No auth/consent** layer is present despite the plan's "Access-gated" language (that is
  a `calyxd` concern).

## 6. Gaps / not covered

- **No batch JSON-RPC at the binary:** `decode_jsonrpc_wire` supports batches, but
  `main.rs` uses `decode_jsonrpc_request`, which rejects them.
- **`initialize` ignores client params/version** — no protocol-version negotiation; the
  server's `2024-11-05` is fixed.
- **No `resources`, `prompts`, or `logging` MCP capabilities** — `capabilities` advertises
  only `{ tools: {} }`.
- **`search.fresh`** is a no-op (above).
- **`search.guard=in_region`** is unreachable (always errors, above).
- Several output structs are private (`pub(super)`) serde types; their exact field sets
  are documented above but are not part of a published Rust API.
- The xterm materialization in `agree`/`disagree` is the only write performed by an
  otherwise read-only navigation group; its exact persistence target is in
  `tools/search/extensions/xterms.rs` (not expanded here).
