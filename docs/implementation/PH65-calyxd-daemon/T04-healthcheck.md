# PH65 · T04 — `healthcheck` command: CUDA probe + real read + JSON write

| Field | Value |
|---|---|
| **Phase** | PH65 — calyxd daemon (loopback, healthcheck) |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `calyxd` |
| **Files** | `crates/calyxd/src/health.rs` (≤500), `crates/calyx-cli/src/main.rs` (modify ≤500) |
| **Depends on** | T02 (CUDA probe), T03 (VRAM budget) |
| **Axioms** | A16, A18 |
| **PRD** | `dbprdplans/16 §4`, `16 §6` |

## Goal

Implement `calyx healthcheck --wait <secs>` — a real two-part probe: (1) CUDA
init via Forge with the configured VRAM budget, (2) a live read from the
configured vault — that writes a structured JSON result to
`/zfs/hot/logs/calyx-health/latest.json`. The `.status` field is the literal
string `"pass"` on success and `"fail"` on any error. Failure writes the
`CALYX_*` error code to the JSON under `.error_code`. The `--wait` flag retries
for up to N seconds (1-second intervals) to tolerate a slow CUDA init. Used as
`ExecStartPost` in the PH66 systemd unit.

> **Path note:** until PH66 operator steps provision `/zfs/hot/logs/`, the
> `health_log_path` in `calyx.toml` is set to
> `$CALYX_HOME/logs/calyx-health/latest.json`; after provisioning it is updated
> to `/zfs/hot/logs/calyx-health/latest.json`. The healthcheck code follows
> whatever path the config supplies — no hard-coded path in the binary.

## Build (checklist of concrete, code-level steps)

- [ ] `CalyxHealthResult` struct (serde-serializable):
  ```rust
  pub struct CalyxHealthResult {
      pub status: &'static str,     // "pass" or "fail"
      pub timestamp_utc: String,    // ISO-8601
      pub cuda_device: Option<String>,
      pub vram_budget_mib: u32,
      pub vault_read_ok: bool,
      pub error_code: Option<String>,   // CALYX_* code if fail
      pub error_detail: Option<String>,
  }
  ```
- [ ] `fn run_healthcheck(cfg: &CalyxConfig) -> CalyxHealthResult`: probe CUDA
  (calls `probe_cuda_device()`), probe VRAM budget (calls `startup_vram_audit()`),
  issue a real vault read (open the configured vault, read one constellation by
  a known test CxId, verify it round-trips — not a ping, a real Aster read)
- [ ] `fn write_health_result(result: &CalyxHealthResult, path: &Path) ->
  Result<(), DaemonError>`: serialize to JSON, write to a temp file **inside
  the same directory** (avoid `EXDEV` cross-dataset rename per `01 §4`), then
  `rename` to `path`
- [ ] `--wait <secs>` retry loop: attempt `run_healthcheck` every 1 second up to
  the configured timeout; on first `"pass"` result, write and exit 0; if all
  attempts fail, write the last failure result and exit 1
- [ ] `calyx healthcheck` CLI subcommand wired into `calyx-cli/src/main.rs` (adds
  to existing CLI, does not replace it)
- [ ] Log each retry attempt: `INFO healthcheck: attempt {n}/{max} status={s}`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `CalyxHealthResult { status: "pass", … }` serializes to JSON with
  `"status":"pass"` — assert exact JSON key presence via `serde_json::Value`
- [ ] unit: `write_health_result` with a temp dir → file exists at path, is valid
  JSON, `.status` field present
- [ ] unit: with `CALYX_FORCE_CUDA_FAIL=1` → `run_healthcheck` returns a result
  with `status == "fail"` and `error_code == Some("CALYX_FORGE_DEVICE_UNAVAILABLE")`
- [ ] unit: `--wait 2` with a probe that fails twice then succeeds → final result
  is `"pass"`, exactly 3 attempts logged
- [ ] edge: parent directory of health path does not exist → `write_health_result`
  creates it (via `fs::create_dir_all`) before writing
- [ ] edge: vault path does not exist → `status == "fail"` with
  `error_code == Some("CALYX_DAEMON_HEALTH_FAIL")`, not a panic
- [ ] edge: `--wait 0` → single attempt, no retry loop
- [ ] fail-closed: CUDA probe fails + vault read fails → both recorded in the JSON
  result; exit code 1; no silent success reported

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/hot/logs/calyx-health/latest.json` (or
  `$CALYX_HOME/logs/calyx-health/latest.json` pre-provisioning)
- **Readback:**
  ```bash
  # On aiwonder — run healthcheck against a real vault:
  source /home/croyse/calyx/repo/env.sh
  calyx healthcheck --wait 30
  cat /home/croyse/calyx/logs/calyx-health/latest.json | python3 -m json.tool
  # Must show: "status": "pass", "vault_read_ok": true, "cuda_device": "<name>"

  # Fail path:
  CALYX_FORCE_CUDA_FAIL=1 calyx healthcheck --wait 5; echo "exit=$?"
  cat .../latest.json | python3 -m json.tool
  # Must show: "status": "fail", "error_code": "CALYX_FORGE_DEVICE_UNAVAILABLE"
  ```
- **Prove:** pass path: `.status == "pass"` AND `.vault_read_ok == true` AND
  `.cuda_device` is non-null with a device name string. Fail path: `.status ==
  "fail"` AND `.error_code == "CALYX_FORGE_DEVICE_UNAVAILABLE"` AND exit code 1.
  Both JSON files attached as evidence to the PH65 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH65 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
