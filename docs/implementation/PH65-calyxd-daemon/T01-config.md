# PH65 · T01 — `CalyxConfig` + `calyx.toml` reference config

| Field | Value |
|---|---|
| **Phase** | PH65 — calyxd daemon (loopback, healthcheck) |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `calyxd` |
| **Files** | `crates/calyxd/src/config.rs` (≤500), `infra/aiwonder/calyx.toml` |
| **Depends on** | — |
| **Axioms** | A16, A18 |
| **PRD** | `dbprdplans/16 §2`, `16 §4`, `01 §4` |

## Goal

Define `CalyxConfig` — the single authoritative runtime config struct — and the
reference `calyx.toml` that populates it. Every tunable (bind addr, vault path,
VRAM budget, log dir, healthcheck output path, TEI endpoints) is declared here
with a documented key. Secrets never appear in the file; they enter via
environment variables referenced in the config or from Infisical-rendered
`calyx.env`. No hardcoded constant elsewhere in the daemon bypasses this.

## Build (checklist of concrete, code-level steps)

- [ ] `CalyxConfig` struct with `serde::Deserialize`: fields `bind_addr:
  SocketAddr` (default `127.0.0.1:7700`), `vault_path: PathBuf`,
  `vram_budget_mib: u32`, `log_dir: PathBuf`, `health_log_path: PathBuf`
  (default `/zfs/hot/logs/calyx-health/latest.json`), `tei_endpoints:
  Vec<String>` (documenting `:8088`/`:8089`/`:8090`), `healthcheck_timeout_secs:
  u32` (default 30)
- [ ] `CalyxConfig::from_file(path: &Path) -> Result<Self, DaemonError>`: reads
  TOML, validates `bind_addr` is loopback (`127.0.0.1` or `[::1]`); if not →
  `CALYX_DAEMON_BIND_FAILED` with remediation message
- [ ] Validation: `vram_budget_mib` must be > 0 and ≤ 30 000 (RTX 5090 is
  32 607 MiB; leave headroom for TEI); emit `CALYX_FORGE_VRAM_BUDGET` error code
  if violated at config parse time
- [ ] `infra/aiwonder/calyx.toml`: fully documented reference file with every
  key commented; placeholder values that will work on aiwonder; no secret values
- [ ] `CalyxConfig::vault_path_resolved(&self) -> PathBuf`: returns the path
  with env-var interpolation (`$CALYX_HOME`) expanded, so config files remain
  portable across dev and production layouts

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: parse a minimal inline TOML string → assert `bind_addr ==
  127.0.0.1:7700`, `vram_budget_mib == 8192`, paths round-trip
- [ ] unit: `from_file` with a temp file containing a non-loopback bind addr
  (`0.0.0.0:7700`) → `Err(DaemonError::BindFailed)` with error code
  `CALYX_DAEMON_BIND_FAILED`
- [ ] unit: `vram_budget_mib = 0` → validation error at parse time
- [ ] unit: `vram_budget_mib = 31000` (over 30 000 ceiling) → error at parse time
- [ ] edge: missing `vault_path` key → descriptive error (not a panic)
- [ ] edge: TOML syntax error → error wraps the parse failure, not a silent
  default
- [ ] edge: `$CALYX_HOME` env-var interpolation in `vault_path` expands correctly
  when the var is set; returns raw path when var is absent
- [ ] fail-closed: config with `[::1]:7700` (IPv6 loopback) → accepted (valid
  loopback); `[::]:7700` → `CALYX_DAEMON_BIND_FAILED`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `infra/aiwonder/calyx.toml` in the repo; parsed config struct in
  memory validated at daemon startup
- **Readback:** `cargo test -p calyxd config -- --nocapture 2>&1 | tail -20`
  shows all unit tests pass on aiwonder; `cargo run -p calyxd -- --config
  infra/aiwonder/calyx.toml --validate-config` prints parsed config (no secrets)
  and exits 0
- **Prove:** config parse succeeds with the reference file; non-loopback bind
  addr in a test TOML yields `CALYX_DAEMON_BIND_FAILED` in the test output —
  both visible in the `--nocapture` run

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH65 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
