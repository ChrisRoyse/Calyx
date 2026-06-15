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

## Implementation record

- Code surface: `crates/calyx-forge/src/vram/oom_guard.rs` defines `OomGuard`, injectable `CudaMalloc`, `CudaAllocError`, `OomGuardStats`, and the CUDA-feature `RawCudaMalloc` adapter.
- Error mapping: CUDA allocation OOM maps to `CALYX_FORGE_VRAM_BUDGET`; non-OOM CUDA allocation errors map to `CALYX_GPU_ERROR`.
- Counters: `VramBudgeter::stats()` now exposes `OomGuardStats { oom_intercepts, batch_reductions, final_failures }`.
- Metrics: `VramStats::admission_metrics_text()` emits `forge_oom_intercepts_total`, `forge_oom_batch_reductions_total`, and `forge_oom_final_failures_total`.
- Coverage: unit/proptest coverage lives in `crates/calyx-forge/src/vram/oom_guard_tests.rs`; readback artifact coverage lives in `crates/calyx-forge/tests/ph57_oom_guard_fsv.rs`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `OomGuardStats::oom_intercepts` and `final_failures` counters; `dmesg` on aiwonder (must show no OOM kill during test)
- **Readback:** `calyx readback --metric forge_oom_intercepts_total` and `forge_oom_final_failures_total`; `sudo dmesg | grep -i oom`
- **Prove:** inject a VRAM-exhaustion scenario on aiwonder (allocate all GPU memory via a test process, then dispatch to Forge); `forge_oom_intercepts_total > 0`; `dmesg` shows no OOM kill; the failing dispatch returns `CALYX_FORGE_VRAM_BUDGET` in the client log (not a panic).

## FSV evidence

- Root: `/home/croyse/calyx/data/fsv-issue478-oom-guard-20260614T201628Z`
- Synthetic OOM guard readback: `ph57-oom-guard-readback.json`, 1401 bytes, sha256 `5c970858a58db6f15641bb3707ed79c863e04d16ba0765cdef912494fb6d1a57`
- Real CUDA OOM readback: `ph57-oom-guard-cuda-readback.json`, 719 bytes, sha256 `13f6ac41d0922cc3dd7c4602201b4945fe63ef54f5ab23cdae1330a913ad0c97`
- Metrics readback: `ph57-oom-guard.prom`, 1123 bytes, sha256 `bc5067b20f26b415a1cfdd6fd7feeebacca07161a579a09e6010e7a3f607f6b8`
- Synthetic counters: before `oom_intercepts=0`, after alloc retry `oom_intercepts=2`, after dispatch `batch_reductions=1`, after final failure `oom_intercepts=5`, `final_failures=1`; dispatch output `32`; final error `CALYX_FORGE_VRAM_BUDGET`; non-OOM CUDA error `CALYX_GPU_ERROR`.
- Real CUDA OOM: requested `34743517184` bytes against device total `33669775360`; before `oom_intercepts=0`; after `oom_intercepts=1`, `final_failures=1`; error `CALYX_FORGE_VRAM_BUDGET`.
- Prometheus metrics: `forge_oom_intercepts_total 5`, `forge_oom_batch_reductions_total 1`, `forge_oom_final_failures_total 1`.
- Kernel readback: sudo `dmesg --ctime` last 300 lines checked after the CUDA OOM FSV; `oom|out of memory|xid|nvrm` matches = 0.
- CUDA parity root: `/home/croyse/calyx/data/fsv-issue478-cuda-parity-20260614T201810Z`; `cuda-gemm-parity.json` sha256 `950601de2fee27f9649fb6ca247913854ee3eeb5de0772a8409e65beddc07fdd`, max relative error `0.00031746612512506545`; `cuda-normalize-parity.json` sha256 `c761b5ee188d90780ed21bf4444bfd53eb372ee6414882152ffc09c0ed10a038`, relative error `0.0000002533753900024749`.
- Gates passed on aiwonder: `cargo fmt --all -- --check`; `cargo check -p calyx-forge`; `cargo clippy -p calyx-forge --all-targets -- -D warnings`; `cargo test -p calyx-forge -- --nocapture`; `cargo test -p calyx-forge --features cuda --test ph57_oom_guard_fsv -- --nocapture`; `cargo test -p calyx-forge --features cuda --test cuda_parity -- --nocapture`; `.rs` line-count gate (no files >500; touched max `budget.rs` 483).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH57 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
