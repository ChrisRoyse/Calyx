# PH65 · T02 — CUDA init probe + fail-loud (`CALYX_FORGE_DEVICE_UNAVAILABLE`)

| Field | Value |
|---|---|
| **Phase** | PH65 — calyxd daemon (loopback, healthcheck) |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `calyxd` |
| **Files** | `crates/calyxd/src/error.rs` (≤500), `crates/calyxd/src/main.rs` (≤500) |
| **Depends on** | T01 (CalyxConfig in scope) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/16 §4`, `16 §2` |

## Goal

In server mode, any CUDA initialization failure is immediately fatal with a
structured `CALYX_FORGE_DEVICE_UNAVAILABLE` error code — no silent fallback to
CPU. Implement the `DaemonError` catalog and the CUDA probe path that `calyxd`
calls at startup before accepting any connections. The test env var
`CALYX_FORCE_CUDA_FAIL=1` triggers the failure path deterministically for FSV.

## Build (checklist of concrete, code-level steps)

- [ ] `crates/calyxd/src/error.rs`: `DaemonError` enum with variants:
  - `DeviceUnavailable { code: &'static str, detail: String }` — maps to
    `CALYX_FORGE_DEVICE_UNAVAILABLE`
  - `VramBudgetExceeded { code: &'static str, budget_mib: u32, required_mib: u32 }` — maps to `CALYX_FORGE_VRAM_BUDGET`
  - `BindFailed { code: &'static str, addr: String, detail: String }` — maps to
    `CALYX_DAEMON_BIND_FAILED`
  - `ConfigError { code: &'static str, detail: String }` — maps to
    `CALYX_DAEMON_CONFIG_INVALID`
  - `HealthcheckFailed { code: &'static str, detail: String }` — maps to
    `CALYX_DAEMON_HEALTH_FAIL`
  - Each variant's `Display` impl includes the `CALYX_*` code string and a
    remediation hint (A16: structured errors always carry remediation)
- [ ] `fn probe_cuda_device() -> Result<CudaDeviceInfo, DaemonError>`: calls into
  `calyx-forge` CUDA init path; if `CALYX_FORCE_CUDA_FAIL=1` env var is set,
  returns `Err(DaemonError::DeviceUnavailable { code:
  "CALYX_FORGE_DEVICE_UNAVAILABLE", detail: "forced by CALYX_FORCE_CUDA_FAIL" })`
  unconditionally (test injection)
- [ ] `CudaDeviceInfo { device_name: String, vram_total_mib: u32,
  compute_cap: String }` — populated from the Forge init response on success
- [ ] `main.rs` startup sequence: call `probe_cuda_device()` immediately after
  loading config; on `Err` → `eprintln!` the structured error, exit with code 1,
  no further initialization attempted
- [ ] Log line on success: `INFO calyxd: CUDA device ready device="{name}"
  vram={mib}MiB compute={cap}` (structured, goes to `$log_dir/calyxd.log`)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: with `CALYX_FORCE_CUDA_FAIL=1` set in env, `probe_cuda_device()`
  returns `Err` whose `Display` contains `"CALYX_FORGE_DEVICE_UNAVAILABLE"`
- [ ] unit: `DaemonError::DeviceUnavailable` `Display` contains both the error
  code literal `"CALYX_FORGE_DEVICE_UNAVAILABLE"` and a non-empty remediation
  string
- [ ] unit: `DaemonError::BindFailed` `Display` contains
  `"CALYX_DAEMON_BIND_FAILED"` and the refused address
- [ ] edge: `CALYX_FORCE_CUDA_FAIL` absent → probe path runs normally (no panic
  on missing env var)
- [ ] edge: `CALYX_FORCE_CUDA_FAIL=0` → not treated as failure (only exact `"1"`
  triggers it)
- [ ] fail-closed: every `DaemonError` variant's `Display` includes a non-empty
  remediation string — assert `format!("{e}").contains("remediation:")` for each
  variant constructed with test data

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stderr output from `calyxd` when CUDA probe fails; exit code
- **Readback:**
  ```bash
  # On aiwonder — confirm fail-loud path:
  CALYX_FORCE_CUDA_FAIL=1 cargo run -p calyxd -- \
    --config infra/aiwonder/calyx.toml 2>&1; echo "exit=$?"
  # Must print CALYX_FORGE_DEVICE_UNAVAILABLE and exit=1
  ```
- **Prove:** output contains `CALYX_FORGE_DEVICE_UNAVAILABLE`; exit code is 1;
  no line in the output contains `"fallback"` or `"cpu"` (case-insensitive) —
  assert both present/absent in the captured output attached to the PH65 issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH65 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
