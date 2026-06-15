# 16. MCP Server & Daemon (calyx-mcp, calyxd)

This reference documents two Rust crates **strictly from their source**:

- **`calyx-mcp`** — the MCP (Model Context Protocol) server library + stdio binary that exposes Calyx tools over JSON-RPC 2.0.
- **`calyxd`** — the Calyx daemon/server: a Prometheus `/metrics` exporter, a periodic ledger chain-verify loop, a CUDA/VRAM startup preflight, a healthcheck probe, a byte-level vault read-back verifier, and a loopback MCP-over-socket transport.

Where the source does not establish a fact, it is marked **"Not determined from source"**. Every claim is traced to a file path and function/type, with the owning crate noted.

> Sibling references: see [15_cli_reference.md](15_cli_reference.md) for the `calyx` CLI (the consumer of the `calyxd` library), and [11_ledger_provenance.md](11_ledger_provenance.md) for the ledger chain semantics the daemon verifies.

## Source files covered

**`crates/calyx-mcp`** (8 files):
- `src/lib.rs` — crate surface / re-exports
- `src/main.rs` — stdio JSON-RPC entrypoint binary (`calyx-mcp`)
- `src/server.rs` — `McpServer` tool registry + dispatch
- `src/jsonrpc.rs` — inbound JSON-RPC wire decoding
- `src/protocol.rs` — JSON-RPC response framing, errors, MCP descriptors
- `src/schema.rs` — JSON Schema constructors for tool inputs
- `tests/jsonrpc.rs`, `tests/stdio.rs` — integration tests (not documented in detail)
- `Cargo.toml`

**`crates/calyxd`** (20 files):
- `src/main.rs` — daemon binary (`calyxd`) entrypoint + CLI arg parsing
- `src/lib.rs` — library surface
- `src/config.rs` — `CalyxConfig` TOML runtime config
- `src/error.rs` — `DaemonError` taxonomy
- `src/server.rs` — loopback HTTP `/metrics` server
- `src/mcp_server.rs` — loopback MCP-over-socket transport (`CalyxMcpServer`)
- `src/verify_loop.rs` — periodic chain-verify driver (binary-only module)
- `src/verify.rs` — `verify_restore` byte-level vault read-back
- `src/health.rs` — daemon-readiness healthcheck
- `src/cuda_probe.rs` — CUDA device preflight
- `src/vram.rs` — VRAM budget enforcer (NVML)
- `src/metrics.rs` — `ChainVerifyMetrics` family + module root
- `src/metrics/calyx.rs` — `CalyxMetrics` full metric surface
- `src/metrics/hazards.rs` — 25 PH59 hazard gauges
- `src/metrics/zfs.rs` — ZFS integrity collector
- `examples/mcp_fsv_server.rs` — manual MCP transport FSV harness
- `tests/*.rs` — integration tests (not documented in detail)
- `Cargo.toml`

---

# Part A — MCP Server (`calyx-mcp`)

## A.1 Crate role and dependencies

`calyx-mcp` is described in `src/lib.rs` as "MCP interface for agent-facing Calyx operations." Its only non-serde dependency (`Cargo.toml`) is `calyx-core` (for `CalyxError`/`Result`); it also depends on `serde` and `serde_json`. It produces one binary, `calyx-mcp`, from `src/main.rs`.

The wire stack is split across four modules (`src/lib.rs` doc):
- `jsonrpc` decodes inbound requests,
- `protocol` frames responses and MCP descriptors,
- `schema` builds tool input schemas,
- `server` holds the tool registry and dispatch.

## A.2 Transport (stdio)

The production transport is **newline-delimited JSON-RPC over stdio**, implemented in `src/main.rs::main`:

1. Constructs an empty `McpServer` via `McpServer::new()`. **The binary registers no tools** (`src/main.rs` line 16 comment: "The scaffold registers no tools yet; tool groups land in later PH63 tasks").
2. Locks `stdin`/`stdout`. Iterates `stdin.lock().lines()`.
3. For each line: trims it; **skips empty lines** (`continue`).
4. Decodes the line via `decode_jsonrpc_request(trimmed.as_bytes())`. On decode error it logs `calyx-mcp: <code>: <message>` to **stderr** and continues to the next line (a malformed line never aborts the loop).
5. Records whether the request is a **notification** (`request.id.is_none()`).
6. Dispatches via `server.dispatch(request)`.
7. If it was a notification, writes **no reply** (`continue`), per JSON-RPC 2.0.
8. Otherwise serializes the response with `serde_json::to_string`, `writeln!`s it to stdout, and `flush`es. A stdout write/flush error returns `ExitCode::FAILURE`.

Stdout is **protocol-only**; every diagnostic goes to stderr "so a stray log line can never corrupt the response stream" (`src/main.rs` doc).

**Startup:** immediate (build the empty server, begin reading stdin). **Shutdown:** EOF on stdin → `ExitCode::SUCCESS` (clean). A stdin read error → log to stderr, `ExitCode::FAILURE`. A response serialize error logs to stderr but does **not** terminate the loop.

| Aspect | Value | Source |
|---|---|---|
| Transport | stdio, newline-delimited JSON | `src/main.rs` |
| Framing | one JSON-RPC object per line | `src/main.rs` |
| Batch support over stdio | No — `decode_jsonrpc_request` rejects batches | `src/jsonrpc.rs::decode_jsonrpc_request` |
| Notifications | no reply when `id` absent | `src/main.rs`, `src/server.rs` |
| Diagnostics | stderr only | `src/main.rs` |
| Clean shutdown | stdin EOF → exit 0 | `src/main.rs` |

## A.3 Inbound decoding (`src/jsonrpc.rs`)

Types:
- `JsonRpcId` — untagged enum: `String(String)`, `Number(i64)`, `Null`.
- `JsonRpcRequest` — `{ jsonrpc: String, method: String, params: Option<Value>, id: Option<JsonRpcId> }`. `params`/`id` are `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- `JsonRpcWire` — `Single(JsonRpcRequest)` or `Batch(Vec<JsonRpcRequest>)`.

Functions:
- `decode_jsonrpc_wire(bytes)` — parses to a `serde_json::Value`. An object → `Single`; a **non-empty** array → `Batch` (an empty batch is an error: "JSON-RPC batch must not be empty"); any other JSON type → error "JSON-RPC wire value must be object or batch".
- `decode_jsonrpc_request(bytes)` — wraps `decode_jsonrpc_wire`; a `Batch` is rejected with "expected a single JSON-RPC request, got a batch". This is the entrypoint the stdio loop uses.
- `JsonRpcRequest::validate()` — fail-closed checks: `jsonrpc` must be exactly `"2.0"`; `method` must be non-empty (trimmed); `method` must **not** start with `rpc.` (reserved); if `params` is present it must be an object or array.

All decode failures produce a `CalyxError` with code constant `CALYX_MCP_JSONRPC_INVALID` and remediation "send a valid JSON-RPC 2.0 MCP request object or non-empty batch" (`src/jsonrpc.rs::jsonrpc_error`).

## A.4 Response framing and errors (`src/protocol.rs`)

JSON-RPC error code constants:

| Constant | Value | Meaning | Source |
|---|---|---|---|
| `JSONRPC_METHOD_NOT_FOUND` | `-32601` | method/tool not registered | `src/protocol.rs` |
| `JSONRPC_INVALID_PARAMS` | `-32602` | structurally wrong `tools/call` payload | `src/protocol.rs` |
| `JSONRPC_INTERNAL_ERROR` | `-32603` | caught tool panic or reply serialize failure | `src/protocol.rs` |
| `JSONRPC_CALYX_ERROR` | `-32000` | a `CalyxError` from a tool | `src/protocol.rs` |

`JsonRpcError { code: i32, message: String, data: Option<Value> }`. Constructors: `method_not_found(name)`, `invalid_params(msg)`, `internal(msg)`, and `from_calyx(&CalyxError)`. `from_calyx` maps to code `-32000`, copies `error.message`, and sets `data = { "calyx_code": <code>, "remediation": <remediation> }` so agents can extract the stable `CALYX_*` code for recovery.

`JsonRpcResponse { jsonrpc: "2.0", result: Option<Value>, error: Option<JsonRpcError>, id: Option<JsonRpcId> }`. Exactly one of `result`/`error` is `Some`. `result`/`error` are `skip_serializing_if = "Option::is_none"`, but `id` is always serialized (`null` when absent). Built via `JsonRpcResponse::success(id, result)` / `JsonRpcResponse::error(id, error)`.

MCP descriptor types:
- `ToolDef { name, description, use_when, input_schema }`. `input_schema` serializes as `inputSchema` (camelCase, MCP-required). `use_when` is a Calyx extension — a one-line agent hint.
- `ContentBlock` — tagged enum with one variant `Text { text: String }` serialized as `{"type":"text","text":...}`.
- `ToolCallResult { content: Vec<ContentBlock> }`. `ToolCallResult::text(payload)` wraps a single text block (the serialized JSON tool payload).

## A.5 Tool registry and dispatch (`src/server.rs`)

Constants:
- `MCP_PROTOCOL_VERSION = "2024-11-05"` (echoed by `initialize`).
- `SERVER_NAME = "calyx-mcp"`.
- `CALYX_MCP_TOOL_DUPLICATE = "CALYX_MCP_TOOL_DUPLICATE"` (local code for a duplicate registration).

The `Tool` trait (`Send + Sync`):
- `fn def(&self) -> ToolDef`
- `fn call(&self, params: Value) -> Result<Value, CalyxError>`

`McpServer { tools: BTreeMap<String, Box<dyn Tool>> }` (ordered by name). Methods:
- `new()` / `Default` — empty server.
- `register(tool)` — fails closed with `CALYX_MCP_TOOL_DUPLICATE` if the name already exists; otherwise inserts. (Duplicate names can never silently shadow.)
- `tool_count()` — number of tools.
- `dispatch(request)` — routes by method, always returns a `JsonRpcResponse`.

### A.5.1 Dispatched methods

`dispatch` (`src/server.rs::dispatch`) handles exactly three methods; anything else → `-32601` method-not-found.

| Method | Behavior | Return shape | Source |
|---|---|---|---|
| `initialize` | Returns protocol/capabilities/serverInfo | `{ protocolVersion: "2024-11-05", capabilities: { tools: {} }, serverInfo: { name: "calyx-mcp", version: <CARGO_PKG_VERSION> } }` | `handle_initialize` |
| `tools/list` | Lists all registered tool descriptors | `{ tools: [ToolDef, ...] }` | `handle_tools_list` |
| `tools/call` | Invokes a named tool | `ToolCallResult` (`{ content: [{type:"text", text:<json>}] }`) on success; JSON-RPC error otherwise | `handle_tools_call` |
| *(any other)* | Method not found | error `-32601` | `dispatch` |

### A.5.2 `tools/call` handling steps (`handle_tools_call`)

1. Extract `params` (default `Value::Null` if absent).
2. Read `params.name` as a non-empty string. If missing/empty/non-string → error `-32602` ("tools/call requires a non-empty string `name`").
3. Read `params.arguments` (default `{}` if absent).
4. Look up the tool by name. If not found → error `-32601` method-not-found.
5. Invoke `tool.call(arguments)` inside `catch_unwind(AssertUnwindSafe(...))` so a panicking tool cannot crash the loop.
6. Outcomes:
   - `Ok(Ok(value))` → serialize `value` to a string; success response wrapping `ToolCallResult::text(payload)`. A serialize failure → `-32603` internal error.
   - `Ok(Err(calyx))` → error via `JsonRpcError::from_calyx` (`-32000`, with `calyx_code`/`remediation` in `data`).
   - `Err(panic)` → `-32603` with the deliberately generic message `"internal server error"`.

### A.5.3 Registered production tools

**None.** The shipped binary (`src/main.rs`) and the `calyxd` MCP transport (Part B) both construct an `McpServer` with **zero tools registered**. The only `Tool` implementors in this codebase are test fixtures (`EchoTool`, `FailingTool`, `PanicTool` in `src/server.rs` tests) and an FSV-only `AdderTool` (`calyxd/examples/mcp_fsv_server.rs`). Source comments state production tool groups are deferred to later PH63 tasks.

| MCP tool | Params | Return | Side effects | Status / Source |
|---|---|---|---|---|
| *(none in production)* | — | — | — | `src/main.rs` line 16; `calyxd/src/mcp_server.rs` doc |
| `echo` *(test only)* | `msg: string` (required) | `{ echoed: <msg> }` | none | `src/server.rs` tests |
| `adder` *(example/FSV only)* | `a: integer`, `b: integer` (both required) | `{ sum: a+b }` | none | `calyxd/examples/mcp_fsv_server.rs` |

> Because no production tools exist, Rule 4's "every MCP tool" table is, from source, empty. The registry, schema, and dispatch machinery are fully built; the tool catalog is not yet wired.

## A.6 Input-schema constructors (`src/schema.rs`)

Helpers that build draft-07-compatible JSON Schema fragments so tools declare `inputSchema` consistently:
- `string_schema()` → `{"type":"string"}`
- `number_schema()` → `{"type":"number"}`
- `integer_schema()` → `{"type":"integer"}`
- `boolean_schema()` → `{"type":"boolean"}`
- `array_schema(items)` → `{"type":"array","items":<items>}`
- `object_schema(&[(name, schema, required)])` → `{"type":"object","properties":{...},"required":[...]}`. Property order follows the slice order; `required` always present (empty array when none required).

---

# Part B — `calyxd` Daemon

## B.1 Crate role and dependencies

`calyxd` is described in `src/main.rs` as "Calyx daemon: Ledger chain-verify metrics on a loopback `/metrics` endpoint." Its `Cargo.toml` depends on `calyx-aster`, `calyx-core`, `calyx-forge`, `calyx-mcp`, `calyx-ledger`, `nvml-wrapper`, `prometheus`, `serde`, `serde_json`, and `toml`. The binary is `calyxd` (`src/main.rs`).

**Feature flag** (`Cargo.toml`): `cuda = ["calyx-forge/cuda"]`. Default features are empty. Without `--features cuda`, server mode's CUDA preflight fails loud at startup (no CPU fallback).

The library (`src/lib.rs`) exposes: `config`, `cuda_probe`, `error`, `health`, `mcp_server`, `metrics`, `server`, `verify`, `vram`. These are the single source of truth consumed by `calyx-cli` and by the binary; `verify_loop` is binary-private (`mod verify_loop;` in `main.rs`).

## B.2 Entry point and run modes (`src/main.rs`)

`main()` parses args (everything after `argv[0]`) via `parse_args`, then branches:

1. **Arg-parse failure** → print `calyxd: <error>` + `USAGE` to stderr, exit code **2**.
2. **`--validate-config`** → `validate_config(config_path)`.
3. **`--config <path>` (without `--validate-config`)** → `run_server(path, once, audit_vram)`.
4. **Otherwise** → `run(config)` (the plain vault/ledger verify+metrics mode).

The internal parsed `Config` struct holds: `targets: Vec<VerifyTarget>`, `bind: SocketAddr`, `interval: Duration`, `once: bool`, `config_path: Option<PathBuf>`, `validate_config: bool`, `audit_vram: bool`.

### B.2.1 Command-line flags

Source: `USAGE` constant + `parse_args` in `src/main.rs`.

| Flag | Argument | Default | Effect | Source |
|---|---|---|---|---|
| `--vault <dir>` | directory (repeatable) | — | add a `TargetKind::Vault` verify target | `parse_args` |
| `--ledger <dir>` | directory (repeatable) | — | add a `TargetKind::LedgerDir` verify target | `parse_args` |
| `--bind <addr>` | `addr:port` | `127.0.0.1:7700` | loopback listen address (parse error → `CALYX_DAEMON_CONFIG_INVALID`) | `parse_args` |
| `--interval-secs <n>` | u64 ≥ 1 | `60` | seconds between verify cycles; `0` rejected ("must be >= 1") | `parse_args` |
| `--once` | — | false | run one verify cycle, print metrics text, exit | `run` |
| `--config <path>` | path | none | load `calyx.toml`; enables server mode (or validate-config) | `parse_args` |
| `--validate-config` | — | false | parse+validate `--config`, print it, exit | `validate_config` |
| `--audit-vram` | — | false | with `--config`: CUDA preflight + NVML VRAM audit, then exit | `run_server` |

Validation rules in `parse_args`:
- A flag requiring a value with none present → `CALYX_DAEMON_CONFIG_INVALID` ("`<flag>` requires a value").
- Unknown argument → `CALYX_DAEMON_CONFIG_INVALID` ("unknown argument `<other>`").
- If **not** validate-config, **no** `--config`, and **no** targets → error "at least one --vault or --ledger target is required".
- `--audit-vram` without `--config` → error "--audit-vram requires --config <calyx.toml>".

### B.2.2 Mode: plain verify + metrics (`run`)

`run(config)` (`src/main.rs::run`):
1. `target.validate()` each target (must be an existing directory, else `CALYX_DAEMON_CONFIG_INVALID`).
2. Build per-target labels (the target's path display string).
3. Create `ChainVerifyMetrics::new(&labels)` (shared `Arc`).
4. **Run one verify cycle synchronously** (`run_cycle`) before binding, so a scrape never sees an unverified gauge.
5. Build `CalyxMetrics::new(chain, &labels)` (the full surface, sharing the same `chain` Arc) and refresh ZFS metrics (`refresh_zfs_metrics`).
6. If `--once`: encode the surface to text, print to stdout, return.
7. Otherwise: `MetricsServer::bind(bind, surface)`, print a serving banner, `spawn_loop` (periodic chain-verify) and `spawn_zfs_metrics_loop`, then `server.run()` (never returns).

`run` returns `Result<(), DaemonError>`; on `Err`, `main` prints to stderr and exits **2**.

### B.2.3 Mode: server (`run_server`, requires `--config`)

`run_server(config_path, once, audit_vram)` (`src/main.rs`), exit code **1** on any preflight failure:
1. `CalyxConfig::from_file(path)` — on error exit **2**.
2. `cuda_probe::probe_cuda_device()` — fatal CUDA preflight; on error exit **1**. Logs `INFO calyxd: CUDA device ready device=... vram=...MiB compute=...`.
3. `vram::NvmlVramUsage::init()` — NVML init; on error exit **1**.
4. `vram::VramBudget::from_config(cfg.vram_budget_mib, &device, nvml)` — on error exit **1**.
5. `budget.startup_vram_audit()` — logs `INFO calyxd: VRAM audit tei_used=... calyx_budget=... device_total=... available=...`.
6. Probe dispatch readiness with a fixed `PROBE_DISPATCH_MIB = 256`: logs `INFO calyxd: dispatch readiness available=... probe=256MiB admitted=<bool>`. This is observability, **not** a hard gate (the comment notes per-dispatch enforcement is future PH65 T05/T06 work).
7. If `--audit-vram`: exit **0** (success) here — no vault needed.
8. Otherwise build a `Config` with one Vault target at `cfg.vault_path_resolved()`, `bind = cfg.bind_addr`, **`interval = 60s` (hard-coded here, not from `cfg`)**, and call `run(...)`.

Note: in server mode the MCP-over-socket transport (`mcp_server::CalyxMcpServer`) is **not** invoked by `main`; the comment in `run_server` says "T05/T06 add the MCP dispatch surface" (future). The MCP transport exists as library API only.

### B.2.4 Mode: validate-config (`validate_config`)

`validate_config(path)`:
- No path → `CALYX_DAEMON_CONFIG_INVALID` ("--validate-config requires --config <path>"), exit **2**.
- `CalyxConfig::from_file(path)`; on success prints `calyxd: config <path> OK`, the `{config:#?}` debug dump (holds no secrets), and `vault_path_resolved`, then exit **0**. On error: stderr + exit **2**.

## B.3 Configuration (`src/config.rs`)

`CalyxConfig` is `#[serde(deny_unknown_fields)]` (a typo'd key fails closed). Constructed only via `from_file(path)` / `from_toml_str(text)`, both of which run `validate()` before returning, so any instance upholds: loopback `bind_addr` and `0 < vram_budget_mib <= 30000`.

| Key | Type | Required | Default | Source |
|---|---|---|---|---|
| `bind_addr` | `SocketAddr` | no | `127.0.0.1:7700` | `default_bind_addr` |
| `vault_path` | `PathBuf` | **yes** | — | `CalyxConfig` |
| `vram_budget_mib` | `u32` | **yes** | — (must be `1..=30000`) | `CalyxConfig` |
| `log_dir` | `PathBuf` | **yes** | — | `CalyxConfig` |
| `health_log_path` | `PathBuf` | no | `/zfs/hot/logs/calyx-health/latest.json` | `default_health_log_path` |
| `tei_endpoints` | `Vec<String>` | no | `[]` | `CalyxConfig` |
| `healthcheck_timeout_secs` | `u32` | no | `30` | `default_healthcheck_timeout_secs` |

Constants: `VRAM_BUDGET_MIB_CEILING = 30_000`; `VAULT_PATH_HOME_VAR = "CALYX_HOME"`.

Validation (`validate`):
- Non-loopback `bind_addr` → `CALYX_DAEMON_BIND_FAILED` (must be `127.0.0.1` or `[::1]`; `[::]` unspecified is rejected).
- `vram_budget_mib == 0` or `> 30000` → `CALYX_FORGE_VRAM_BUDGET`.
- TOML syntax error / missing required key → `CALYX_DAEMON_CONFIG_INVALID`.

`vault_path_resolved()` expands `$CALYX_HOME` / `${CALYX_HOME}` from the environment (pure helper `resolve_home`); when unset the raw path is returned unchanged. Secrets never appear in the config (doc): they enter via env vars / Infisical-rendered `calyx.env`.

## B.4 Error taxonomy (`src/error.rs`)

`DaemonError` enum; `Display` always renders `<code>: <detail> (remediation: <hint>)`.

| Variant | Constructor | Code | Source |
|---|---|---|---|
| `BindFailed` | `bind_failed` | `CALYX_DAEMON_BIND_FAILED` | `error.rs` |
| `ConfigInvalid` | `config_invalid` | `CALYX_DAEMON_CONFIG_INVALID` | `error.rs` |
| `VramBudget` | `vram_budget` | `CALYX_FORGE_VRAM_BUDGET` | `error.rs` |
| `DeviceUnavailable` | `device_unavailable` | `CALYX_FORGE_DEVICE_UNAVAILABLE` | `error.rs` |
| `HealthFailed` | `health_failed` | `CALYX_DAEMON_HEALTH_FAIL` | `error.rs` |

Each variant carries a fixed `remediation()` hint. Daemon-local MCP codes live elsewhere: `CALYX_DAEMON_FRAME_INVALID` and `CALYX_DAEMON_CONN_PANIC` (`src/mcp_server.rs`).

## B.5 HTTP `/metrics` server (`src/server.rs`)

`MetricsServer` — a loopback-only thread-per-connection HTTP/1.1 listener serving exactly `GET /metrics`.

Constants: `REQUEST_HEAD_LIMIT = 8192` bytes; `IO_TIMEOUT = 5s` (read + write); `CONTENT_TYPE = "text/plain; version=0.0.4"` (Prometheus exposition format v0.0.4).

- `bind(addr, metrics)` — **refuses any non-loopback IP before touching the OS** → `CALYX_DAEMON_BIND_FAILED`; otherwise `TcpListener::bind`.
- `local_addr()` — the actually-bound address (resolves port 0).
- `run(self) -> !` — accept loop; each connection served on its own thread (one stuck client cannot block the next scrape). Accept errors are logged, not fatal.

Request handling (`handle_connection` → `route`):
1. Set read/write timeouts (5s).
2. `read_request_line` reads the full request head (through the blank line), bounded by 8192 bytes. Reading the whole head matters: closing a socket with unread bytes sends TCP RST not FIN. An unreadable/oversized head → `400 Bad Request`.
3. `route` splits the request line:
   - `GET /metrics` → `200 OK` with the encoded metric text; an encode error → `500 Internal Server Error`.
   - `GET <other path>` → `404 Not Found` ("only /metrics is served").
   - any other method → `405 Method Not Allowed`.
4. `write_response` writes a minimal HTTP/1.1 response with `Content-Type`, `Content-Length`, `Connection: close`.

| Path / Method | Status | Body | Source |
|---|---|---|---|
| `GET /metrics` | `200 OK` | Prometheus v0.0.4 text | `route` |
| `GET /metrics` (encode error) | `500` | error text | `route` |
| `GET <other>` | `404` | "only /metrics is served" | `route` |
| non-GET | `405` | "only GET /metrics is served" | `route` |
| unreadable head | `400` | "bad request" | `handle_connection` |

## B.6 MCP-over-socket transport (`src/mcp_server.rs`)

`CalyxMcpServer` — a **loopback-only, length-prefixed JSON-RPC** transport that hands each decoded request to a shared `calyx_mcp::McpServer`. It is **transport only**: it does not reimplement MCP methods, panic isolation, or `CalyxError` mapping (those live in `calyx-mcp`), and it does **not** register production tools. The whole workspace is synchronous (no tokio); this uses `std::net::TcpListener` thread-per-connection.

Constants:
- `MAX_FRAME_BYTES = 4 * 1024 * 1024` (4 MiB) — hard ceiling; a larger length prefix is refused before allocation (DoS guard).
- `IO_TIMEOUT = 5s` (per-connection read/write).
- `DRAIN_TIMEOUT = 5s` (in-flight connections drain window on shutdown).
- `CALYX_DAEMON_FRAME_INVALID`, `CALYX_DAEMON_CONN_PANIC` — daemon-local codes.

### B.6.1 Wire format

Each message: a **4-byte big-endian `u32` length prefix** + exactly that many bytes of UTF-8 JSON (one JSON-RPC request or response). A clean EOF at a frame boundary is a normal close (`FrameRead::Eof`).

Fail-closed framing (`read_frame` / `read_full_or_eof`):
- Length `0` → `CALYX_DAEMON_FRAME_INVALID` ("zero-length frame...").
- Length `> MAX_FRAME_BYTES` → `CALYX_DAEMON_FRAME_INVALID` (refused before allocation).
- Truncated prefix (partial then EOF) → `CALYX_DAEMON_FRAME_INVALID` ("truncated frame prefix").
- Truncated body (`read_exact` short) → error ("read N-byte frame body").
Any framing error closes the connection (the byte stream can no longer be trusted).

### B.6.2 API and lifecycle

- `bind(addr, dispatcher: Arc<McpServer>)` — refuses non-loopback IP → `CALYX_DAEMON_BIND_FAILED`; else binds. (Accepts IPv4 and IPv6 loopback.)
- `from_config(cfg, dispatcher)` — binds `cfg.bind_addr` (re-asserts loopback at the OS boundary).
- `local_addr()` — bound address (resolves `:0`).
- `shutdown_handle()` — returns a cloneable `ShutdownHandle` (`shutdown` flag + `active` connection counter + addr). **Obtain before `run`** (which consumes `self`).
- `run(self) -> Result<(), DaemonError>` — accept loop; each connection on its own thread, panics isolated (a connection panic logs `CALYX_DAEMON_CONN_PANIC` and the loop survives). Blocks until shutdown fires, then waits up to `DRAIN_TIMEOUT` for `active == 0`.

`ShutdownHandle::shutdown()` sets the stop flag and opens a throwaway loopback `TcpStream::connect(addr)` to wake the blocked `accept()`. `active_connections()` reports in-flight handlers.

### B.6.3 Per-connection serving (`serve_connection`)

1. Set 5s read/write timeouts.
2. Loop: `read_frame`; on `Eof` return `Ok(())`.
3. Decode the payload via `decode_jsonrpc_request`:
   - **Ok**: record notification-ness (`id.is_none()`); `dispatcher.dispatch(request)`; if notification, skip the reply; else `write_response` (length-prefixed).
   - **Err(calyx)** (malformed JSON inside a valid frame): a per-message error, not a stream error — write back a JSON-RPC error (`from_calyx`, id `null`, carrying `CALYX_MCP_JSONRPC_INVALID`) and keep serving the next frame.

> The MCP method/tool semantics are identical to Part A.5 because the same `McpServer::dispatch` is used. As shipped, the dispatcher passed in carries no production tools.

## B.7 Periodic chain-verify (`src/verify_loop.rs`, binary-only)

Types:
- `TargetKind::{Vault, LedgerDir}` — Vault = Aster vault dir (`cf/ledger` SSTs + WAL); LedgerDir = standalone directory ledger.
- `VerifyTarget { kind, path }`. `label()` = path display string. `validate()` = path must already be a directory else `CALYX_DAEMON_CONFIG_INVALID`.

Flow:
- `VerifyTarget::verify()` opens the store fresh from disk each cycle (no cached state) — `AsterLedgerCfStore::open` (Vault) or `DirectoryLedgerStore::open` (LedgerDir) — then `verify_full_chain`, which scans `0..head` (`head = max_seq + 1`, or 0 when empty) via `calyx_ledger::verify_chain`. Maps the `VerifyResult` to a `VerifyOutcome` (`Intact{entries}` / `Broken{at_seq}` / `Corrupt{at_seq,reason}` / `Error{detail}`).
- `run_cycle(targets, metrics)` — verifies each target, records the outcome at the current unix time, and logs it (`log_outcome`: `intact` → stdout; `broken`/`corrupt`/`error` → stderr with the `CALYX_*` code).
- `spawn_loop(targets, metrics, interval)` — spawns a thread that sleeps `interval` then `run_cycle` forever.

A broken/corrupt/unverifiable chain is **not** a process exit — the gauge holds 0 until the chain verifies intact (the alert is the metric).

## B.8 Metrics surface

### B.8.1 Module root — chain-verify family (`src/metrics.rs`)

`VerifyOutcome` (above) with `label()` and `ok_value()` (1 only for `Intact`). `OUTCOME_LABELS = ["intact","broken","corrupt","error"]`.

`ChainVerifyMetrics::new(vault_labels)` registers four families into its own `Registry` and pre-initializes every series (so they exist from the first scrape). `record(vault, outcome, now_secs)` updates them. `encode_text()` emits Prometheus v0.0.4 text.

| Metric | Type | Labels | Meaning | Source |
|---|---|---|---|---|
| `calyx_ledger_chain_verify_ok` | gauge | `vault` | 1 iff last verify proved intact | `metrics.rs` |
| `calyx_ledger_chain_verify_last_run_timestamp_seconds` | gauge | `vault` | unix ts of last run | `metrics.rs` |
| `calyx_ledger_chain_verify_entries` | gauge | `vault` | entries proven intact (0 unless intact) | `metrics.rs` |
| `calyx_ledger_chain_verify_runs_total` | counter | `vault`, `outcome` | runs by outcome | `metrics.rs` |

### B.8.2 Full surface — `CalyxMetrics` (`src/metrics/calyx.rs`)

`CalyxMetrics::new(chain: Arc<ChainVerifyMetrics>, vault_labels)` owns a **second** registry for the T03 families and concatenates the two exposition blocks in `encode_text()` (chain-verify families first, then its own). Latency histogram buckets: `[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]` seconds.

`SearchStrategy::{SingleLens, Rrf, WeightedRrf, Sparse}` → labels `single_lens`/`rrf`/`weighted_rrf`/`sparse`. Status label is `ok`/`err`.

| Metric | Type | Labels | Meaning | Source |
|---|---|---|---|---|
| `calyx_ingest_duration_seconds` | histogram | `vault` | ingest batch latency | `calyx.rs` |
| `calyx_ingest_total` | counter | `vault`,`status` | ingest ops by status | `calyx.rs` |
| `calyx_search_duration_seconds` | histogram | `vault`,`strategy` | search latency by strategy | `calyx.rs` |
| `calyx_search_recall_tripwire` | gauge | `vault` | 1 healthy / 0 tripped (pre-init = 1) | `calyx.rs` |
| `calyx_search_total` | counter | `vault`,`strategy`,`status` | search ops | `calyx.rs` |
| `calyx_guard_far` | gauge | `vault`,`slot` | guard false-accept rate | `calyx.rs` |
| `calyx_guard_frr` | gauge | `vault`,`slot` | guard false-reject rate | `calyx.rs` |
| `calyx_assay_n_eff` | gauge | `vault`,`panel` | DDA effective sample size | `calyx.rs` |
| `calyx_kernel_recall_ratio` | gauge | `vault`,`scope` | kernel recall vs brute force | `calyx.rs` |
| `calyx_anneal_ab_variant_total` | counter | `experiment`,`variant` | A/B exposures | `calyx.rs` |
| `calyx_anneal_ab_improvement_ratio` | gauge | `experiment` | A/B improvement ratio | `calyx.rs` |
| `calyx_vram_budget_used_mib` | gauge | — | VRAM used (MiB) | `calyx.rs` |
| `calyx_vram_budget_limit_mib` | gauge | — | VRAM ceiling (MiB) | `calyx.rs` |
| `calyx_hazard_<id>` (×25) | gauge | `hazard` | 1 firing / 0 nominal | `metrics/hazards.rs` |
| `calyx_zfs_pool_healthy` | gauge | `pool` | 1 healthy | `metrics/zfs.rs` |
| `calyx_zfs_cksum_errors_total` | gauge | `pool` | CKSUM error count | `metrics/zfs.rs` |
| `calyx_zfs_scrub_age_seconds` | gauge | `pool` | last scrub age | `metrics/zfs.rs` |
| `calyx_zfs_dataset_checksum_enabled` | gauge | `dataset` | 1 if checksum != off | `metrics/zfs.rs` |

Recording methods on `CalyxMetrics`: `observe_ingest`, `observe_search`, `set_recall_tripwire`, `set_guard_rates`, `set_assay_n_eff`, `set_kernel_recall_ratio`, `record_anneal_exposure`, `set_anneal_improvement`, `set_vram_budget`, `set_hazard` (unknown id → fail-closed `Err`), `record_zfs_integrity`. Statically-known series (vault, ingest status, search strategy/status) are pre-initialized; dynamic-cardinality families (guard slot, panel, scope, experiment) appear on first observation. Duplicate family registration **panics at init** (`register` helper).

### B.8.3 Hazard gauges (`src/metrics/hazards.rs`)

`HAZARD_IDS` — exactly **25** PH59 hazard ids (in register order): `compaction_storm, flush_stall, tombstone_buildup, fsync_spike, wal_bloat, mvcc_version_pileup, vram_oom, heap_oom, nan_propagation, quant_drift, codebook_staleness, ann_corruption, hot_shard_skew, lock_contention, cache_stampede, slow_lens_hol, disk_full, arc_thrash, clock_skew, anneal_thrash, panel_explosion, secret_leakage, nondeterminism, whole_host_loss, upgrade_skew`. Each is a **distinct metric name** `calyx_hazard_<id>` with a `hazard` label (one line per hazard). `set(id, triggered)` on an unknown id is a fail-closed error.

### B.8.4 ZFS integrity collector (`src/metrics/zfs.rs`)

`DEFAULT_ZFS_DATASETS = ["hotpool/calyx", "archive/calyx", "archive/calyx-restic"]`. `ZFS_SCRUB_MAX_AGE_SECONDS = 40 days`; unknown scrub age is recorded as `ZFS_SCRUB_MAX_AGE_SECONDS + 1` (fail-closed). `collect_zfs_integrity` shells out to `zfs get -H -o value checksum <ds>`, `zpool status -x <pool>`, `zpool status -v <pool>`, and `date -d <scrub-date> +%s`, parsing checksum/health/CKSUM-count/scrub-age. `collect_default_zfs_integrity()` uses the defaults at the current time. Pools are derived from dataset names (segment before `/`). In `main`, `refresh_zfs_metrics` is called once at startup and then by `spawn_zfs_metrics_loop` every `interval`; a collection failure logs to stderr and does not abort.

## B.9 CUDA preflight (`src/cuda_probe.rs`)

`probe_cuda_device() -> Result<CudaDeviceInfo, DaemonError>`. `CudaDeviceInfo { device_name, vram_total_mib: u32, compute_cap: String }`.

- Env `FORCE_FAIL_ENV = "CALYX_FORCE_CUDA_FAIL"`: **only the exact string `"1"`** forces `CALYX_FORGE_DEVICE_UNAVAILABLE` (deterministic FSV injection). Any other value runs the real probe.
- **With** feature `cuda`: `calyx_forge::init_cuda(0, false)`, reading compute capability + total VRAM (MiB). Init failure → `CALYX_FORGE_DEVICE_UNAVAILABLE`.
- **Without** feature `cuda`: always `Err(CALYX_FORGE_DEVICE_UNAVAILABLE)` ("rebuild with `--features cuda`"). There is **no CPU fallback** — server mode refuses to start.

## B.10 VRAM budget enforcer (`src/vram.rs`)

Reads live usage via **NVML** (`nvml-wrapper`), not `cudaMemGetInfo` — NVML reports the true board total and device-wide used bytes consistent with `nvidia-smi`. `BYTES_PER_MIB = 1024*1024`. TEI endpoints documented in errors: `:8088 (general), :8089 (reranker), :8090 (legal)`.

- `VramReading { total_mib, used_mib }`; trait `VramUsage::read()` (production = `NvmlVramUsage`, tests inject a mock).
- `NvmlVramUsage::init()` loads `libnvidia-ml.so.1` explicitly (driver-only hosts lack the unversioned symlink). Failure → `CALYX_FORGE_DEVICE_UNAVAILABLE`. `read()` uses device index 0's `memory_info()`.
- `VramBudget::from_config(cfg_budget_mib, device, usage)` — fail-closed at construction: budget `0` → error; budget `> device board total` → `CALYX_FORGE_VRAM_BUDGET`; resident `used_mib > budget` → `CALYX_FORGE_VRAM_BUDGET` (names the TEI endpoints).
- `allocated_mib()` (live used), `available_mib()` (`budget - used`, saturating at 0), `check_can_allocate(required_mib)` (`used + required > budget` → fail closed), `startup_vram_audit()` → `VramAuditReport { tei_used_mib, calyx_budget_mib, device_total_mib }`.

## B.11 Healthcheck (`src/health.rs`)

`run_healthcheck(cfg) -> CalyxHealthResult` — never panics, never returns `Err`; every failure is encoded into the result (fail-closed). Two-part probe:
1. **CUDA init** (`probe_cuda_device`), plus a **VRAM budget audit** against live NVML usage (only when CUDA is up).
2. **Real Aster vault read-back** via `verify::verify_restore` (not a ping). Any failure → `CALYX_DAEMON_HEALTH_FAIL` preserving the cause.

`CalyxHealthResult { status: "pass"|"fail", timestamp_utc, cuda_device: Option<String>, vram_budget_mib, vault_read_ok, error_code: Option<String>, error_detail: Option<String> }` (serde `Serialize`). First failure wins for `error_code`, but per-subsystem fields still record every outcome. `is_pass()` = `status == "pass"`.

- `write_health_result(result, path)` — atomic write: create parent dir, write to a temp sibling (`.<name>.<pid>.tmp`) in the **same directory** (no cross-dataset `EXDEV`), then `rename` into place; on rename failure, best-effort remove the temp.
- `run_with_wait(wait_secs, attempt)` — retries up to `wait_secs + 1` times at 1-second intervals (`wait_secs == 0` ⇒ exactly one attempt), returning the first pass or the last failure with the attempt count.
- `iso8601_from_unix_secs` — dependency-free civil-date formatting (Howard Hinnant's algorithm), `YYYY-MM-DDThh:mm:ssZ`.

## B.12 Vault read-back verifier (`src/verify.rs`)

`verify_restore(vault_path) -> Result<VerifyRestoreReport, DaemonError>` — byte-level read-back of a restored Aster vault with **zero write side-effects** (read-only; never creates/truncates/locks/replays; avoids `CfRouter::open` / `DurableVault::open`).

`VerifyRestoreReport { vault_path, constellation_count, anchor_count, ledger_entry_count, ledger_tip_hash (hex), chain_intact, wal_bytes_present, first_cx_id: Option<hex>, error: Option<String> }`. `success()` = no error AND chain intact AND `constellation_count > 0` AND `anchor_count > 0` AND `wal_bytes_present > 0`. `failure_reasons()` names every unmet criterion (or just the scan error if one aborted verification).

Fail-closed contract:
- Missing / non-directory path → `CALYX_DAEMON_CONFIG_INVALID`.
- Directory with neither `cf/` nor `wal/` → `CALYX_DAEMON_CONFIG_INVALID`.
- Any scan/chain failure → `Ok(report)` with `chain_intact == false` and `error` holding the exact `CALYX_*` code.

Steps: stat WAL bytes (`wal_total_bytes` over `wal/*.wal`); `scan_vault` replays the WAL overlay (`replay_dir`; a **torn tail fails closed** — a restored snapshot must replay cleanly), merges SSTs (newest-file-wins) with WAL rows (latest-write-wins) per CF, reads the first constellation back completely (base row decodes, key matches embedded CxId, every slot column present + decodable), and merges ledger rows (divergent bytes for one seq → `ledger_corrupt`). Then `verify_chain(0..head)`; the tip hash is the last entry's `entry_hash` (all-zero hex when empty). Optional rebuildable dirs `ann/`, `kernel/`, `guard/` are logged-if-absent but never fail. `RestoredLedgerRows` is a read-only `LedgerCfStore` whose `put_new` rejects all appends.

## B.13 Ports and endpoints summary

| Surface | Default address | Protocol | Source |
|---|---|---|---|
| HTTP `/metrics` | `127.0.0.1:7700` (`--bind` or `config.bind_addr`) | HTTP/1.1, `GET /metrics` only, loopback-only | `server.rs`, `config.rs` |
| MCP-over-socket | `cfg.bind_addr` (loopback-only); library API, not wired into `main` | length-prefixed JSON-RPC, loopback-only | `mcp_server.rs` |
| MCP-over-stdio (`calyx-mcp`) | n/a (stdio) | newline-delimited JSON-RPC | `calyx-mcp/src/main.rs` |
| TEI endpoints (documented) | `:8088` general, `:8089` reranker, `:8090` legal | external (not served by calyxd) | `vram.rs`, `config.rs` |

> **Loopback-only is enforced** in three independent places: `CalyxConfig::validate` (config parse), `MetricsServer::bind`, and `CalyxMcpServer::bind`. A non-loopback address is `CALYX_DAEMON_BIND_FAILED` and the server never starts. Source comments note Cloudflare Tunnel + Caddy are the sole external ingress.

## B.14 Exit codes (`calyxd` binary)

| Code | Condition | Source |
|---|---|---|
| 0 | clean run (`--once`, `--validate-config` OK, `--audit-vram` OK, server `run` returns Ok) | `main.rs` |
| 1 | server-mode preflight failure (config load in `run_server`, CUDA, NVML, VRAM budget/audit) | `run_server` |
| 2 | arg-parse failure, `run` error, `validate_config` error | `main`, `run`, `validate_config` |

The HTTP `/metrics` `run` loop and the MCP `run` accept loop are otherwise long-running; a broken ledger chain is reported via the gauge, never via exit.
