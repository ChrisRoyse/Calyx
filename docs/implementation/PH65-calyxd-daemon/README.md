# PH65 — calyxd daemon (loopback, healthcheck)

**Stage:** S16 — Server & Deployment  ·  **Crate:** `calyxd`  ·
**PRD roadmap:** P9  ·  **Axioms:** A16, A18

## Objective

Build the `calyxd` server daemon — the same `calyx` core, served — binding
loopback only, with MCP over the gated ingress and a real healthcheck that probes
CUDA init and a live read, writing `"pass"` to
`/zfs/hot/logs/calyx-health/latest.json`. The daemon must fail loud on any CUDA
init failure (`CALYX_FORGE_DEVICE_UNAVAILABLE`) — no silent CPU fallback is
permitted in server mode. VRAM budget must honor the 3 resident TEI containers
already using the RTX 5090. `calyx.toml` is the single authoritative config file.

> **Operator/sudo note (binding, `01 §3`):** sudo is password-backed and
> available through the local env var name when a phase is authorized to perform
> gated host work. Systemd unit installation and ZFS dataset creation are handled
> in PH66 and must read back the resulting service/dataset state. This phase
> runs and tests from `CALYX_HOME` on the NVMe root, with no dependency on those
> steps. Do not touch existing
> leapable/contextgraph/PostgreSQL state on the box.

## Dependencies

- **Phases:** PH24 (Sextant search — provides the live Aster vault + search pipeline),
  PH13 (CUDA sm_120 backend — Forge CUDA init path `calyxd` must probe),
  PH62 (calyx-cli — `calyx healthcheck` CLI entrypoint lives here)
- **Provides for:** PH66 (systemd unit wraps this binary; healthcheck wired into
  `ExecStartPost`), PH67 (running daemon is the restore target for DR drill)

## Current state (build off what exists)

`crates/calyxd/src/main.rs` is a 5-line stub:
```rust
fn main() { println!("calyxd skeleton"); }
```
Greenfield. `infra/aiwonder/` does not yet exist.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyxd/src/main.rs` | Binary entry point: parse `calyx.toml`, init Forge (CUDA probe/fail-loud), bind loopback listener, start MCP handler loop, run healthcheck on startup |
| `crates/calyxd/src/config.rs` | `CalyxConfig` struct: deserialize `calyx.toml` (bind addr, vault path, VRAM budget MiB, log dir, healthcheck path, TEI endpoints) |
| `crates/calyxd/src/health.rs` | `healthcheck` logic: probe CUDA init via Forge, issue a real read from the vault, write result JSON to `latest.json`; `CalyxHealthResult` with `.status` literal `"pass"`/`"fail"` |
| `crates/calyxd/src/server.rs` | Loopback TCP listener + MCP-over-socket dispatch; routes requests to the calyx core search/ingest pipeline |
| `crates/calyxd/src/vram.rs` | VRAM budget enforcer: reads configured budget, queries current allocation, gates Forge dispatch; emits `CALYX_FORGE_VRAM_BUDGET` if over |
| `crates/calyxd/src/error.rs` | `DaemonError` enum mapping to `CALYX_*` codes; `CALYX_FORGE_DEVICE_UNAVAILABLE`, `CALYX_FORGE_VRAM_BUDGET`, `CALYX_DAEMON_BIND_FAILED` |
| `infra/aiwonder/calyx.toml` | Reference config file (all keys documented, no secrets; secrets via env) |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `CalyxConfig` + `calyx.toml` reference config | — |
| T02 | CUDA init probe + fail-loud (`CALYX_FORGE_DEVICE_UNAVAILABLE`) | T01 |
| T03 | VRAM budget enforcer (honor resident TEI) | T02 |
| T04 | `healthcheck` command: CUDA probe + real read + JSON write | T02, T03 |
| T05 | Loopback bind + MCP dispatch server | T01, T02 |
| T06 | `calyxd` main: wire config → Forge init → loopback → healthcheck | T04, T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

On aiwonder:
1. `calyxd --config infra/aiwonder/calyx.toml &` starts without error, binds
   `127.0.0.1:<port>`.
2. `calyx healthcheck --wait 30` exits 0 and
   `cat /zfs/hot/logs/calyx-health/latest.json` contains `.status == "pass"`.
3. CUDA-init-failure path: with `CALYX_FORCE_CUDA_FAIL=1` (test env var), daemon
   exits non-zero with `CALYX_FORGE_DEVICE_UNAVAILABLE`; health JSON has
   `.status == "fail"` with the error code present; `systemctl`/log shows the
   error (not a silent fallback to CPU).
4. All byte verifications performed manually on aiwonder and evidence attached to
   the PH65 GitHub issue.

## Risks / landmines

- **CUDA init races:** Forge CUDA context init on sm_120 can fail transiently
  after driver reload; healthcheck must retry up to `--wait` seconds (not just
  once) before emitting fail.
- **VRAM contention:** TEI containers (:8088/:8089/:8090) plus dcgm-exporter
  already hold VRAM. Budget config must be measured, not guessed.
- **`latest.json` path before ZFS provisioning:** until PH66 operator steps run,
  write to `$CALYX_HOME/logs/calyx-health/latest.json` and symlink; after
  provisioning, config points to `/zfs/hot/logs/calyx-health/latest.json`.
- **MCP over loopback only:** do NOT expose on `0.0.0.0` at any point; a bind to
  a non-loopback address is a hard `CALYX_DAEMON_BIND_FAILED` (fail closed).
- **`EXDEV` on ZFS:** any tmp file the daemon creates must be staged inside the
  destination dataset (avoid cross-dataset renames, `01 §4`).
