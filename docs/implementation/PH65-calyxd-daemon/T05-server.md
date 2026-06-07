# PH65 · T05 — Loopback bind + MCP dispatch server

| Field | Value |
|---|---|
| **Phase** | PH65 — calyxd daemon (loopback, healthcheck) |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `calyxd` |
| **Files** | `crates/calyxd/src/server.rs` (≤500) |
| **Depends on** | T01 (CalyxConfig — bind_addr), T02 (DaemonError) |
| **Axioms** | A16, A17 |
| **PRD** | `dbprdplans/16 §2`, `16 §5` |

## Goal

Implement the loopback TCP listener and MCP-over-socket dispatch layer for
`calyxd`. The server binds exclusively to the configured loopback address (never
`0.0.0.0`); Cloudflare Tunnel + Caddy are the sole external ingress. The MCP
handler routes tool calls to the calyx core search/ingest pipeline. Any attempted
bind to a non-loopback address is a hard `CALYX_DAEMON_BIND_FAILED` — the server
does not start. The same `calyx` core logic (vault open, search, ingest) that the
CLI uses is invoked directly — there is no separate code path.

## Build (checklist of concrete, code-level steps)

- [ ] `CalyxServer` struct: holds `TcpListener` bound to `cfg.bind_addr`, a
  `Arc<CalyxVault>` (the open vault), and a `Arc<VramBudget>`
- [ ] `CalyxServer::bind(cfg: &CalyxConfig, vault: Arc<CalyxVault>, budget:
  Arc<VramBudget>) -> Result<Self, DaemonError>`: calls
  `TcpListener::bind(cfg.bind_addr)`; before binding, asserts
  `cfg.bind_addr.ip().is_loopback()` → else `Err(DaemonError::BindFailed)`
- [ ] `CalyxServer::run(&self) -> Result<(), DaemonError>`: accept loop; each
  accepted connection spawned as an async task (tokio); connection handler reads
  a length-prefixed JSON MCP request, dispatches to the appropriate handler,
  writes a length-prefixed JSON MCP response
- [ ] MCP tool dispatch: `search` → `calyx_core::search(vault, query)`;
  `ingest` → `calyx_core::ingest(vault, payload)`; unknown tool →
  `CALYX_MCP_UNKNOWN_TOOL` error response (not a panic; connection closed cleanly)
- [ ] Graceful shutdown: `CalyxServer` accepts a `CancellationToken`; on
  cancellation, stop accepting new connections, drain in-flight within a 5-second
  window, then exit
- [ ] Connection-level error isolation: a panic in one connection handler must not
  crash the server; use `tokio::task::catch_unwind` or equivalent; log the panic
  with `CALYX_DAEMON_CONN_PANIC` code and close the connection

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `CalyxServer::bind` with a loopback addr → Ok; with `0.0.0.0:7700`
  → `Err(CALYX_DAEMON_BIND_FAILED)` containing the refused address string
- [ ] unit: `CalyxServer::bind` with `[::1]:7700` (IPv6 loopback) → Ok (valid)
- [ ] integration: bind to `127.0.0.1:0` (OS-assigned port); connect with a TCP
  client; send a valid MCP `search` JSON request; receive a well-formed response
  with a non-empty `results` array — use a seeded synthetic vault with 3 known
  constellations
- [ ] integration: send an unknown tool name → receive a response with
  `error_code == "CALYX_MCP_UNKNOWN_TOOL"` and connection remains open for the
  next request
- [ ] edge: client sends malformed JSON → server sends error response, does not
  crash, accepts next connection
- [ ] edge: client disconnects mid-request → server logs and moves on; no resource
  leak (assert connection count returns to 0 after close)
- [ ] fail-closed: attempt bind on port 80 (privileged, will fail) → `Err` with
  OS error detail in `CALYX_DAEMON_BIND_FAILED` message; process does not start

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ss -tlnp | grep 7700` showing `127.0.0.1:7700` with the calyxd PID;
  MCP JSON response bytes in the integration test
- **Readback:**
  ```bash
  # On aiwonder — server binds loopback only:
  cargo run -p calyxd -- --config infra/aiwonder/calyx.toml &
  sleep 2
  ss -tlnp | grep calyxd
  # Must show 127.0.0.1:7700 (or configured port), NOT 0.0.0.0

  # MCP round-trip:
  cargo test -p calyxd server::integration -- --nocapture 2>&1 | tail -30
  ```
- **Prove:** `ss` output shows loopback-only bind; integration test log shows
  MCP response bytes with a non-empty results array — both in the attached
  evidence on the PH65 issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH65 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
