# PH13 · T02 — build.rs: nvcc sm_120 compilation + PTX embed

| Field | Value |
|---|---|
| **Phase** | PH13 — CUDA sm_120 Backend + Bit-Parity |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/build.rs` (≤500), `crates/calyx-forge/src/cuda/kernels/distance.cu` (≤500), `crates/calyx-forge/src/cuda/kernels/topk.cu` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A13 |
| **PRD** | `dbprdplans/13 §4` |

## Goal

Write the `build.rs` that compiles the `.cu` SIMT kernel sources with
`nvcc -arch=sm_120` against CUDA 13.3 at `/usr/local/cuda-13.3`, embeds PTX
bytes in the binary for JIT fallback, and places cubin as the fast path. The
build succeeds only on aiwonder (feature-gated); on non-CUDA builds `build.rs`
is a no-op. This is the kernel compilation infrastructure all other PH13 cards
depend on.

## Build (checklist of concrete, code-level steps)

- [x] `build.rs`: check `cfg!(feature="cuda")`; if not set, print `cargo:warning=cuda feature not enabled, skipping kernel compilation` and exit `Ok(())`
- [x] Locate `nvcc` at `$CUDA_PATH/bin/nvcc` (default `CUDA_PATH=/usr/local/cuda-13.3`);
  if not found → `panic!("nvcc not found at {path}; set CUDA_PATH to CUDA 13.3 root")` (loud failure, not silent skip)
- [x] Compile each `.cu` with:
  `nvcc -arch=sm_120 -O3 --use_fast_math=false -Xcompiler -fPIC --ptx -o <out>.ptx <src>.cu`
  and separately:
  `nvcc -arch=sm_120 -O3 --use_fast_math=false -Xcompiler -fPIC -cubin -o <out>.cubin <src>.cu`
  (`--use_fast_math=false` is mandatory for the determinism contract)
- [x] Embed PTX bytes via `include_bytes!` macro paths written by `build.rs` into
  `OUT_DIR`; emit `cargo:rustc-env=FORGE_DISTANCE_PTX_PATH=...` and
  `cargo:rustc-env=FORGE_TOPK_PTX_PATH=...`
- [x] `src/cuda/kernels/distance.cu`: skeleton fused cosine kernel `__global__ void
  cosine_batch_f32(...)` — computes dot + norm in one pass, stores result in
  `out[]`; block size 256; determinism comment: `// DETERMINISM: warp reduce with
  fixed shuffle mask, no atomics`
- [x] `src/cuda/kernels/topk.cu`: skeleton bitonic sort kernel `__global__ void
  bitonic_topk_f32(...)` — in-place top-k over a score array; deterministic
  tie-break by index (lower wins)
- [x] `cargo:rerun-if-changed=src/cuda/kernels/distance.cu`
  `cargo:rerun-if-changed=src/cuda/kernels/topk.cu`
  so incremental builds recompile on `.cu` changes

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit `#[cfg(feature="cuda")]`: `include_bytes!(env!("FORGE_DISTANCE_PTX_PATH"))`
  is non-empty and starts with the PTX magic bytes `"//\n.version"` or `".version"` (PTX header)
- [x] unit `#[cfg(feature="cuda")]`: PTX bytes for distance kernel contain the
  string `"cosine_batch_f32"` (kernel entry point name present in PTX)
- [x] integration: `nvcc --version` on aiwonder prints `V13.3` (build.rs prints
  the detected nvcc version to stderr via `cargo:warning`)
- [x] edge (≥3): (1) `CUDA_PATH` unset → `build.rs` checks default path; (2) `.cu`
  file missing → `build.rs` panics with file path in message; (3) `--use_fast_math`
  is NOT in the nvcc command (grep the `build.rs` source in CI/lint pass)
- [x] fail-closed: `nvcc` returns non-zero exit code → `build.rs` panics with the
  full stderr of nvcc in the panic message so the developer sees the compiler error

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo build -p calyx-forge --features cuda` on aiwonder producing `.ptx` in `OUT_DIR`
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo build -p calyx-forge --features cuda 2>&1 | grep -E "warning=|Compiling|error"
  # Find and inspect the PTX:
  find $CALYX_HOME/target -name "distance.ptx" 2>/dev/null | head -1 | xargs head -5
  ```
- **Prove:** build succeeds with no errors; `distance.ptx` found in target; first
  5 lines of PTX contain `.version` and `.target sm_120`; absent: any `error[E]`
  or `--use_fast_math` in the nvcc flags

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] `cargo check` (no `--features cuda`) passes (no-op build.rs)
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (PTX header + build log screenshot) attached to the PH13 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
