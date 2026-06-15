# PH65 · T03 — VRAM budget enforcer (honor resident TEI)

| Field | Value |
|---|---|
| **Phase** | PH65 — calyxd daemon (loopback, healthcheck) |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `calyxd` |
| **Files** | `crates/calyxd/src/vram.rs` (≤500) |
| **Depends on** | T02 (CudaDeviceInfo, DaemonError catalog) |
| **Axioms** | A16, A26 |
| **PRD** | `dbprdplans/16 §1`, `16 §4`, `16 §6` |

## Goal

Implement the VRAM budget enforcer so `calyxd` never over-allocates GPU memory
against the 3 resident TEI containers (`:8088` general, `:8089` reranker,
`:8090` legal ModernBERT) plus dcgm-exporter. The configured `vram_budget_mib`
cap from `calyx.toml` is the hard ceiling for Forge dispatch; any request that
would breach it gets `CALYX_FORGE_VRAM_BUDGET`. The enforcer is queried before
every Forge kernel dispatch and after startup to confirm the budget is not
already exhausted by TEI.

## Build (checklist of concrete, code-level steps)

- [ ] `VramBudget` struct: `budget_mib: u32`, `device_total_mib: u32`;
  `fn allocated_mib(&self) -> u32` — queries NVML/sysfs for current GPU memory
  used; `fn available_mib(&self) -> u32 = budget_mib.saturating_sub(allocated_mib())`
- [ ] `VramBudget::check_can_allocate(&self, required_mib: u32) ->
  Result<(), DaemonError>`: if `allocated_mib() + required_mib > budget_mib` →
  `Err(DaemonError::VramBudgetExceeded { budget_mib: self.budget_mib,
  required_mib })` with code `CALYX_FORGE_VRAM_BUDGET`
- [ ] `VramBudget::from_config(cfg: &CalyxConfig, device: &CudaDeviceInfo) ->
  Result<Self, DaemonError>`: validates `budget_mib ≤ device.vram_total_mib`;
  if the already-resident TEI footprint (measured at startup) would already
  exhaust the budget → `Err(DaemonError::VramBudgetExceeded)` with a message
  naming the TEI endpoints
- [ ] Read VRAM use via `/proc/driver/nvidia/gpus/*/information` or `nvml-sys`;
  if NVML is unavailable, read `/sys/bus/pci/drivers/nvidia/*/resource` file
  size as fallback; document the fallback path explicitly (it is not a silent
  data-path fallback — it is an observability fallback for a sysfs read)
- [ ] `fn startup_vram_audit(budget: &VramBudget) -> Result<VramAuditReport,
  DaemonError>`: runs at daemon start, logs TEI footprint and remaining budget;
  `VramAuditReport { tei_used_mib: u32, calyx_budget_mib: u32,
  device_total_mib: u32 }` — written to startup log line
- [ ] Honor `leapable-gpu-max-power.service` (600 W cap, `16 §1`) by never
  scheduling Forge work that would push power above the cap — document as a note
  in the source (enforcement is the OS-level service; Calyx just stays within
  the VRAM budget)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `VramBudget { budget_mib: 8192, device_total_mib: 32607 }` with
  mock allocated=4096 → `check_can_allocate(4000)` returns Ok;
  `check_can_allocate(4097)` returns `Err(CALYX_FORGE_VRAM_BUDGET)`
- [ ] unit: `from_config` with `budget_mib > device_total_mib` → error at
  construction time (not later at dispatch)
- [ ] unit: mock allocated already at `budget_mib` → `check_can_allocate(1)`
  returns `Err` immediately
- [ ] unit: `available_mib()` saturates at 0 when allocated > budget (no
  underflow / negative values)
- [ ] edge: `budget_mib = 0` → `check_can_allocate(0)` is Ok; `(1)` is Err
- [ ] edge: NVML unavailable (mock the sysfs path as absent) → falls back to
  sysfs; test passes with a known mock value
- [ ] fail-closed: startup audit with TEI footprint exceeding budget → returns
  `CALYX_FORGE_VRAM_BUDGET` with TEI endpoint names in the error detail

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** startup log line on aiwonder containing `VramAuditReport` values;
  `CALYX_FORGE_VRAM_BUDGET` error in unit test output
- **Readback:**
  ```bash
  # On aiwonder — see real TEI footprint vs budget:
  cargo test -p calyxd vram -- --nocapture 2>&1 | grep -E "budget|VRAM|MiB"
  # Also: startup run with real NVML:
  cargo run -p calyxd -- --config infra/aiwonder/calyx.toml --audit-vram 2>&1
  ```
- **Prove:** `VramAuditReport` printed at startup shows `tei_used_mib` > 0
  (TEI is actually present), `calyx_budget_mib` matches config value, and
  `device_total_mib == 32607` (the RTX 5090 confirmed value from `01 §2`)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH65 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
