# PH12 · T01 — Backend trait + error types

| Field | Value |
|---|---|
| **Phase** | PH12 — CPU SIMD Backend |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/backend.rs` (≤500), `crates/calyx-forge/src/error.rs` (≤500) |
| **Depends on** | PH03, PH04 (error catalog, core types) |
| **Axioms** | A13, A16 |
| **PRD** | `dbprdplans/13 §2`, `dbprdplans/13 §7`, `dbprdplans/23 §3` |

## Goal

Define the `Backend` trait that all compute backends (CPU, CUDA) implement, the
`BackendKind` / `BestConfig` types the autotune cache (PH16) will key on, and
the `ForgeError` structured error type that maps to the `CALYX_*` error catalog
from `calyx-core`. This is the shared contract every subsequent PH12–PH16 card
builds against.

## Build (checklist of concrete, code-level steps)

- [x] `src/backend.rs`: define `trait Backend: Send + Sync` with methods:
  `fn gemm(&self, a: &[f32], b: &[f32], m: usize, k: usize, n: usize, out: &mut [f32]) -> Result<(), ForgeError>`;
  `fn cosine(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<(), ForgeError>`;
  `fn dot(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<(), ForgeError>`;
  `fn l2(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<(), ForgeError>`;
  `fn normalize(&self, vecs: &mut [f32], dim: usize) -> Result<(), ForgeError>`;
  `fn topk(&self, scores: &[f32], k: usize) -> Result<Vec<(usize, f32)>, ForgeError>`;
  `fn device_info(&self) -> DeviceInfo`
- [x] `enum BackendKind { Cpu, Cuda }` with `Display`; `struct BestConfig { backend: BackendKind, tile_m: usize, tile_n: usize, tile_k: usize, extra: HashMap<String,String> }` — both `serde::{Serialize, Deserialize}`, `Clone`, `Debug`
- [x] `struct DeviceInfo { kind: BackendKind, name: String, avx512: bool, vram_mib: Option<u64> }` — used by autotune and FSV readback
- [x] `src/error.rs`: `enum ForgeError` variants:
  `NumericalInvariant { op: String, detail: String }` → maps to `CALYX_FORGE_NUMERICAL_INVARIANT`;
  `DeviceUnavailable { device: String, detail: String }` → maps to `CALYX_FORGE_DEVICE_UNAVAILABLE`;
  `ShapeMismatch { expected: Vec<usize>, got: Vec<usize> }`;
  `Unimplemented { op: String }` — each variant carries a `remediation: String` field
- [x] `impl std::fmt::Display for ForgeError` — format includes the `CALYX_*` code name as the first token so grep in logs is unambiguous
- [x] Re-export `BackendKind`, `BestConfig`, `Backend`, `ForgeError`, `DeviceInfo` from `lib.rs`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `DeviceInfo::default()` round-trips through `serde_json`; `BackendKind::Cpu` → `Display` = `"cpu"`
- [x] unit: `BestConfig { backend: Cpu, tile_m: 64, .. }` serializes to JSON containing `"backend":"cpu"` and deserializes back equal
- [x] proptest: any `ForgeError` variant's `Display` output starts with `"CALYX_FORGE_"` (the error-code prefix invariant)
- [x] edge (≥3): (1) `ShapeMismatch` with empty vecs; (2) `NumericalInvariant` with a 512-char detail string; (3) `DeviceUnavailable` with `remediation` containing newlines — all `Display` without panic
- [x] fail-closed: `ForgeError::NumericalInvariant` → `Display` contains literal string `CALYX_FORGE_NUMERICAL_INVARIANT`; `ForgeError::DeviceUnavailable` → contains `CALYX_FORGE_DEVICE_UNAVAILABLE`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-forge backend::tests` output on aiwonder
- **Readback:** `cargo test -p calyx-forge -- backend --nocapture 2>&1 | grep -E "PASSED|FAILED|CALYX_FORGE"`
- **Prove:** all tests PASSED; output contains `CALYX_FORGE_NUMERICAL_INVARIANT` and `CALYX_FORGE_DEVICE_UNAVAILABLE` from the Display assertions — absent: any `panic` or `unwrap` trace

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH12 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
