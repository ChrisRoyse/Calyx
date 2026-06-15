# PH65 · T06 — `calyxd` main: wire config → Forge init → loopback → healthcheck

| Field | Value |
|---|---|
| **Phase** | PH65 — calyxd daemon (loopback, healthcheck) |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `calyxd` |
| **Files** | `crates/calyxd/src/main.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05 (all prior PH65 cards) |
| **Axioms** | A16, A18 |
| **PRD** | `dbprdplans/16 §2`, `16 §4`, `16 §8` |

## Goal

Wire the complete startup sequence in `main.rs`: parse `calyx.toml` → probe CUDA
(fail loud on `CALYX_FORGE_DEVICE_UNAVAILABLE`) → audit VRAM budget → open vault
→ bind loopback server → run startup healthcheck → enter the accept loop. The
resulting binary is the `calyxd` artifact that PH66 will wrap in the systemd
unit. This is integration of all prior T01–T05 pieces, not new logic. The main
entry point also handles signals (SIGTERM/SIGINT → graceful shutdown via
`CancellationToken`).

## Build (checklist of concrete, code-level steps)

- [ ] `main()` startup order (strictly sequential, any step's `Err` → print
  structured error + exit 1, no partial init left running):
  1. Parse `--config <path>` CLI arg (default `calyx.toml` in CWD)
  2. `CalyxConfig::from_file(path)?`
  3. `probe_cuda_device()?` → `CudaDeviceInfo`
  4. `VramBudget::from_config(&cfg, &device)?`
  5. Open the `CalyxVault` at `cfg.vault_path`
  6. `CalyxServer::bind(&cfg, vault.clone(), budget.clone())?`
  7. Spawn SIGTERM/SIGINT handler that fires the `CancellationToken`
  8. Run startup healthcheck: `run_healthcheck(&cfg)` → write result to
     `cfg.health_log_path`; if status == `"fail"` → exit 1 (server does not
     accept connections if health is already bad at startup)
  9. `server.run(cancel_token).await`
- [ ] `--validate-config` flag: run steps 1–2 only, print parsed config (no
  secrets), exit 0 — used for operator verification before service install
- [ ] `--audit-vram` flag: run steps 1–4, print `VramAuditReport`, exit 0 —
  used for pre-deployment capacity check
- [ ] Structured startup banner: `INFO calyxd {version} starting device={name}
  vram_budget={mib}MiB bind={addr} vault={path}`
- [ ] On clean shutdown: write `{"status":"shutdown","timestamp_utc":"…"}` to
  `health_log_path` so downstream monitoring sees a clean state
- [ ] `Cargo.toml` for `calyxd`: add `tokio` with `full` features, `serde`,
  `serde_json`, `toml`, `tracing`, `tracing-subscriber`; link `calyx-core`,
  `calyx-aster`, `calyx-forge` as workspace deps

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] integration: `--validate-config` with a valid `calyx.toml` → exits 0,
  stdout contains `bind_addr`, `vault_path`, `vram_budget_mib` values, no
  secret values (assert no `password`, `token`, `key` substrings in output)
- [ ] integration: `--validate-config` with a non-loopback bind addr in config
  → exits 1, stderr contains `CALYX_DAEMON_BIND_FAILED`
- [ ] integration: `CALYX_FORCE_CUDA_FAIL=1` → exits 1, stderr contains
  `CALYX_FORGE_DEVICE_UNAVAILABLE`, no server socket opened (assert port not in
  `ss` output)
- [ ] integration: full start against a seeded vault on aiwonder → `ss` shows
  loopback bind, `latest.json` has `"status":"pass"` (this is the FSV gate)
- [ ] edge: `--config` path does not exist → exits 1 with
  `CALYX_DAEMON_CONFIG_INVALID` and the missing path in the message
- [ ] edge: SIGTERM sent after startup → graceful shutdown log line written,
  `latest.json` updated to `{"status":"shutdown"}`
- [ ] fail-closed: vault path in config does not exist → exits 1 before binding
  (server never opens a socket if vault is unreadable)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** running process listed by `ss`, `latest.json` on disk with
  `"status":"pass"`, `systemctl`/manual log showing startup banner
- **Readback:**
  ```bash
  # On aiwonder — full integration FSV:
  source /home/croyse/calyx/repo/env.sh
  cargo build -p calyxd --release 2>&1 | tail -5
  ./target/release/calyxd --config infra/aiwonder/calyx.toml &
  CALYXD_PID=$!
  sleep 5
  ss -tlnp | grep $CALYXD_PID          # must show 127.0.0.1:7700 only
  cat /home/croyse/calyx/logs/calyx-health/latest.json | python3 -m json.tool
  # "status": "pass", "vault_read_ok": true, "cuda_device": "NVIDIA GeForce RTX 5090"
  kill $CALYXD_PID
  cat /home/croyse/calyx/logs/calyx-health/latest.json | python3 -m json.tool
  # "status": "shutdown"
  ```
- **Prove:** `ss` shows loopback-only bind; `latest.json` has `"status":"pass"`
  with non-null `cuda_device`; after SIGTERM, JSON has `"status":"shutdown"`.
  All three outputs attached to PH65 issue as the phase FSV gate.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH65 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
