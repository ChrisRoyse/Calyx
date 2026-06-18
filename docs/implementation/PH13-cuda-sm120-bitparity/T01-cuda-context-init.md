# PH13 · T01 — CUDA context init + device query + DEVICE_UNAVAILABLE

| Field | Value |
|---|---|
| **Phase** | PH13 — CUDA sm_120 Backend + Bit-Parity |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cuda/mod.rs` (≤500), `crates/calyx-forge/src/cuda/context.rs` (≤500) |
| **Depends on** | PH12 T01 (Backend trait, ForgeError) |
| **Axioms** | A13, A16 |
| **PRD** | `dbprdplans/13 §2/§4`, `dbprdplans/13 §6` |

## Goal

Initialize the CUDA context via `cudarc` targeting device 0 (the RTX 5090 on
aiwonder), query device properties (name, VRAM, compute capability), and enforce
the fail-closed contract: any init failure returns `ForgeError::DeviceUnavailable`
with `CALYX_FORGE_DEVICE_UNAVAILABLE` — no silent CPU fallback in server mode.
This is the foundation all other PH13 cards build on.

## Build (checklist of concrete, code-level steps)

- [x] `src/cuda/context.rs`: `pub struct CudaContext { inner: Arc<cudarc::driver::CudaDevice>, determinism: bool }`
- [x] `pub fn init_cuda(device_idx: u32, determinism: bool) -> Result<CudaContext, ForgeError>`
  — call `cudarc::driver::CudaDevice::new(device_idx)`; on `Err` → `ForgeError::DeviceUnavailable
  { device: format!("cuda:{device_idx}"), detail: format!("{err}"), remediation: "Check that CUDA 13.3 is installed at /usr/local/cuda-13.3 and nvidia-smi shows the RTX 5090 available".to_string() }`
- [x] `pub fn query_device_info(ctx: &CudaContext) -> DeviceInfo` — populates
  `DeviceInfo { kind: BackendKind::Cuda, name: <device name string>, avx512: false, vram_mib: Some(<total_mem / 1024 / 1024>) }`
- [x] VRAM soft-cap check: if `cuMemGetInfo` shows free VRAM < 4096 MiB at init time
  → `ForgeError::DeviceUnavailable { detail: "less than 4 GiB VRAM free; TEI containers may be using GPU memory" }` (server mode guard)
- [x] `src/cuda/mod.rs`: `pub struct CudaBackend { ctx: CudaContext }`;
  `impl CudaBackend { pub fn new() -> Result<Self, ForgeError> { init_cuda(0, false).map(|ctx| Self { ctx }) } }`
- [x] `impl Backend for CudaBackend`: stub all methods returning `ForgeError::Unimplemented`
  until T03–T05 fill them; `device_info()` delegates to `query_device_info`
- [x] Feature-gate entire `cuda` module: `#[cfg(feature = "cuda")]` in `lib.rs`
  so `cargo check` (without the feature) passes on Windows dev machine

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit `#[cfg(feature="cuda")]`: `CudaBackend::new()` succeeds on aiwonder;
  `device_info().name` contains `"5090"` or `"RTX"` (case-insensitive); `vram_mib >= 30000`
- [x] unit `#[cfg(feature="cuda")]`: `query_device_info` returns `kind == BackendKind::Cuda`
- [x] unit (mock path): `init_cuda` with a bad device index → `ForgeError::DeviceUnavailable`;
  `Display` starts with `"CALYX_FORGE_DEVICE_UNAVAILABLE"`
- [x] proptest: any `ForgeError::DeviceUnavailable` Display output contains both
  `"CALYX_FORGE_DEVICE_UNAVAILABLE"` and `"Remediation:"`
- [x] edge (≥3): (1) device_idx=99 (non-existent) → `DeviceUnavailable`; (2) `new()` called
  twice → second call succeeds (no double-init panic); (3) `device_info()` on a valid backend → all fields non-empty
- [x] fail-closed: on any `cudarc` error the returned `ForgeError::DeviceUnavailable`
  must NOT contain the raw CUDA error number alone — it must include a human remediation string

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cuda::context::tests` on aiwonder GPU
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda cuda::context -- --nocapture 2>&1 \
    | grep -E "RTX|5090|vram_mib|CALYX_FORGE_DEVICE_UNAVAILABLE|PASSED|FAILED"
  ```
- **Prove:** output contains `"RTX"` or `"5090"` from the device name assertion;
  `vram_mib >= 30000` assertion PASSED; bad-device-index test prints
  `CALYX_FORGE_DEVICE_UNAVAILABLE`; absent: any panic or raw CUDA error number alone

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] `cargo check` (no `--features cuda`) passes on any machine (feature gate works)
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (not yet — proven in T06)
- [x] FSV evidence (readback output / screenshot) attached to the PH13 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
