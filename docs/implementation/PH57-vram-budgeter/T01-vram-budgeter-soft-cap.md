# PH57 · T01 — VRAM budgeter — soft cap config, free-VRAM query, usage accounting

| Field | Value |
|---|---|
| **Phase** | PH57 — VRAM budgeter + admission control |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/vram/budget.rs` (≤500), `crates/calyx-forge/src/vram/mod.rs` (≤500) |
| **Depends on** | PH13 (CUDA backend exists) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §2`, `13 §5` |

## Goal

Implement the VRAM budgeter that enforces a soft configurable cap on Forge's GPU allocation,
queries device free VRAM before any large dispatch (using `cudaMemGetInfo` inside the process,
not `nvidia-smi`), and maintains an atomic usage counter so all Forge subsystems see the same
budget. `CALYX_FORGE_VRAM_BUDGET` is the error code for over-budget dispatches. Never assumes
32 GB is available; always queries current free VRAM accounting for TEI and dcgm-exporter
residents.

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct VramBudgeter { soft_cap_bytes: usize, allocated_bytes: AtomicUsize }` in `vram/budget.rs`
- [ ] Implement `VramBudgeter::from_env() -> Self` — reads `CALYX_FORGE_VRAM_BUDGET` env var (bytes, e.g. `12884901888` for 12 GiB); falls back to 12 GiB if unset; logs the configured cap at startup
- [ ] Implement `VramBudgeter::free_device_vram() -> Result<usize, CalyxError>` — calls `cudaMemGetInfo(&free, &total)` via the existing `calyx-forge` CUDA FFI; returns `free` bytes; on CUDA error returns `CALYX_GPU_ERROR`
- [ ] Implement `VramBudgeter::can_allocate(&self, bytes: usize) -> Result<(), CalyxError>` — checks `allocated_bytes.load() + bytes <= soft_cap_bytes`; also calls `free_device_vram()` and checks `bytes <= free - reserved_headroom` (reserved_headroom = 512 MiB for driver overhead); returns `CALYX_FORGE_VRAM_BUDGET` if either check fails
- [ ] Implement `VramBudgeter::reserve(&self, bytes: usize) -> Result<VramGuard, CalyxError>` — calls `can_allocate`, then atomic-adds `bytes` to `allocated_bytes`; returns `VramGuard` (RAII release on drop)
- [ ] Define `struct VramGuard<'b> { budgeter: &'b VramBudgeter, bytes: usize }` with `Drop` impl that subtracts `bytes` from `allocated_bytes`
- [ ] Define `struct VramStats { soft_cap_bytes: usize, allocated_bytes: usize, device_free_bytes: usize }` in `vram/mod.rs`; implement `VramBudgeter::stats() -> VramStats`
- [ ] Add `CALYX_FORGE_VRAM_BUDGET` to `calyx-core` error catalog; remediation: "Forge VRAM budget exceeded; reduce batch size or wait for eviction; set CALYX_FORGE_VRAM_BUDGET env var"

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: budgeter with `soft_cap = 1 GiB`; reserve 512 MiB → succeeds; `allocated_bytes == 512 MiB`; reserve another 512 MiB → succeeds; `allocated_bytes == 1 GiB`; reserve 1 byte more → `CALYX_FORGE_VRAM_BUDGET`
- [ ] unit: `VramGuard` drop — reserve 256 MiB, drop guard; `allocated_bytes == 0`; reserve 256 MiB again → succeeds
- [ ] unit: `from_env()` with `CALYX_FORGE_VRAM_BUDGET=1073741824` (1 GiB) → `soft_cap == 1073741824`; unset env → `soft_cap == 12884901888` (12 GiB default)
- [ ] unit: `free_device_vram()` on aiwonder — returns a value > 0 and ≤ 34_359_738_368 (32 GiB + tolerance); must not return 0 on an idle GPU
- [ ] proptest: `forall soft_cap, allocs: Vec<usize>` — sum of concurrent reservations never exceeds `soft_cap`; guards always release on drop
- [ ] edge: `soft_cap == 0` → every `can_allocate` returns `CALYX_FORGE_VRAM_BUDGET`
- [ ] edge: `bytes == 0` alloc → `can_allocate` succeeds (zero-size reservation valid), `allocated_bytes` unchanged
- [ ] fail-closed: `free_device_vram()` returns `CALYX_GPU_ERROR` (mock CUDA failure) → `can_allocate` returns `CALYX_FORGE_VRAM_BUDGET` (treat unknown device state as over-budget)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `VramStats::allocated_bytes` and `VramStats::device_free_bytes` from `VramBudgeter::stats()`, plus `nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits`
- **Readback:** `calyx readback --metric forge_vram_allocated_bytes` and compare with `nvidia-smi` output; `allocated_bytes + device_free_bytes` should approximate total VRAM (accounting for TEI residents)
- **Prove:** reserve 1 GiB via `VramBudgeter::reserve`, observe `forge_vram_allocated_bytes` increases by 1 GiB in the metric; drop the guard, observe it returns to baseline. Never exceeds `soft_cap_bytes`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (VRAM accounting does not affect compute parity, but CUDA path must still pass)
- [ ] FSV evidence (readback output / screenshot) attached to the PH57 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
