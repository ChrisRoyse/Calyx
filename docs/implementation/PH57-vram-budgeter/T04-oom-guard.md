# PH57 · T04 — OOM guard — reduce-batch → retry → fail closed

| Field | Value |
|---|---|
| **Phase** | PH57 — VRAM budgeter + admission control |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/vram/oom_guard.rs` (≤500) |
| **Depends on** | T03 (admission control), T02 (LRU eviction) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §2` |

## Goal

Intercept every `cudaMalloc` return and implement the last-resort OOM guard: when
`cudaErrorMemoryAllocation` is returned, reduce the batch size by half and retry; if the
minimum batch size is still too large, fail closed with `CALYX_FORGE_VRAM_BUDGET`. No silent
driver-level abort; no `unwrap()` on CUDA alloc; no process crash. This guards the race
between the `free_device_vram()` query and the actual alloc (another process can claim VRAM
between them). Defends hazard 7 (VRAM OOM).

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct OomGuard { registry: Arc<Mutex<GpuBlockRegistry>>, min_batch: usize, max_retries: u8 }` in `oom_guard.rs`
- [ ] Implement `OomGuard::alloc_with_retry(&self, size: usize) -> Result<*mut u8, CalyxError>` — calls CUDA FFI `cudaMalloc`; on `cudaErrorMemoryAllocation`: call `registry.evict_lru()` to free space, retry; if eviction returns `None` (nothing to evict), return `CALYX_FORGE_VRAM_BUDGET`; limit retries to `max_retries` (default 3)
- [ ] Implement `OomGuard::dispatch_with_retry<F, R>(&self, batch_size: usize, f: F) -> Result<R, CalyxError>` where `F: Fn(usize) -> Result<R, CalyxError>` — calls `f(batch_size)`; if `f` returns `CALYX_FORGE_VRAM_BUDGET` and `batch_size / 2 >= min_batch`, retry with `batch_size / 2`; recurse up to `max_retries`; else fail closed
- [ ] Intercept `cudaErrorMemoryAllocation` specifically: map to `CALYX_FORGE_VRAM_BUDGET`; map all other CUDA errors to `CALYX_GPU_ERROR`; never `panic!` or `unwrap`
- [ ] Add `OomGuardStats { oom_intercepts: u64, batch_reductions: u64, final_failures: u64 }` to `VramStats`
- [ ] Emit structured log event on each OOM intercept with `{ attempt, batch_size_before, batch_size_after }` via `tracing::warn!`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: mock `cudaMalloc` returning `cudaErrorMemoryAllocation` on first 2 calls, success on 3rd → `alloc_with_retry` succeeds after 2 eviction+retry cycles; `oom_intercepts == 2`
- [ ] unit: mock `cudaMalloc` always returning `cudaErrorMemoryAllocation`, registry empty (nothing to evict) → returns `CALYX_FORGE_VRAM_BUDGET` after `max_retries`; no infinite loop
- [ ] unit: `dispatch_with_retry(batch=64, f)` where `f` fails for batch > 32 → retries with 32 → succeeds; `batch_reductions == 1`
- [ ] unit: `dispatch_with_retry(batch=1, f)` where `f` always fails → `final_failures == 1`, `CALYX_FORGE_VRAM_BUDGET` returned; no recursion past `min_batch`
- [ ] proptest: `forall max_retries, oom_pattern` — `dispatch_with_retry` terminates in ≤ `max_retries + 1` calls to `f`; never exceeds retry bound
- [ ] edge: `max_retries == 0` → single attempt; if it fails → `CALYX_FORGE_VRAM_BUDGET` immediately
- [ ] edge: mock CUDA error other than OOM (e.g., `cudaErrorIllegalAddress`) → `CALYX_GPU_ERROR` (not `CALYX_FORGE_VRAM_BUDGET`); no retry
- [ ] fail-closed: `alloc_with_retry` with `max_retries=3`, all fail → exactly 3 eviction attempts; return `CALYX_FORGE_VRAM_BUDGET`; `oom_intercepts == 3`, `final_failures == 1`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `OomGuardStats::oom_intercepts` and `final_failures` counters; `dmesg` on aiwonder (must show no OOM kill during test)
- **Readback:** `calyx readback --metric forge_oom_intercepts_total` and `forge_oom_final_failures_total`; `sudo dmesg | grep -i oom`
- **Prove:** inject a VRAM-exhaustion scenario on aiwonder (allocate all GPU memory via a test process, then dispatch to Forge); `forge_oom_intercepts_total > 0`; `dmesg` shows no OOM kill; the failing dispatch returns `CALYX_FORGE_VRAM_BUDGET` in the client log (not a panic).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH57 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
