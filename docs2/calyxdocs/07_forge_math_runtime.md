# 07 — Forge Math Runtime (calyx-forge)

`calyx-forge` is Calyx's owned numeric runtime: one math backend implemented
twice — a CPU SIMD path (via the `wide` crate) and a CUDA path (via `cudarc`,
gated behind the `cuda` feature) — engineered for bit-near parity, plus a
quantization stack from full precision (fp32) down to 1-bit. It also contains a
VRAM budgeter/admission subsystem, an autotune cache, and a compression report
generator.

**Source files covered:**

- `crates/calyx-forge/src/lib.rs`
- `crates/calyx-forge/Cargo.toml`
- `crates/calyx-forge/build.rs`
- `crates/calyx-forge/src/backend.rs`
- `crates/calyx-forge/src/error.rs`
- `crates/calyx-forge/src/cpu/mod.rs`
- `crates/calyx-forge/src/cpu/distance.rs`
- `crates/calyx-forge/src/cpu/gemm.rs`
- `crates/calyx-forge/src/cpu/normalize.rs`
- `crates/calyx-forge/src/cpu/topk.rs`
- `crates/calyx-forge/src/cpu/guard.rs`
- `crates/calyx-forge/src/cuda/mod.rs`
- `crates/calyx-forge/src/cuda/context.rs`
- `crates/calyx-forge/src/cuda/kernels.rs`
- `crates/calyx-forge/src/cuda/distance.rs`
- `crates/calyx-forge/src/cuda/gemm.rs`
- `crates/calyx-forge/src/cuda/gemm/mxfp4_path.rs`
- `crates/calyx-forge/src/cuda/gemm/mxfp8_path.rs`
- `crates/calyx-forge/src/cuda/topk.rs`
- `crates/calyx-forge/src/cuda/grouped_gemm.rs`
- `crates/calyx-forge/src/cuda/ragged_gemm.rs`
- `crates/calyx-forge/src/cuda/kernels/distance.cu`
- `crates/calyx-forge/src/cuda/kernels/topk.cu`
- `crates/calyx-forge/src/cuda/kernels/mxfp4_gemm.cu`
- `crates/calyx-forge/src/cuda/mxfp4.rs`
- `crates/calyx-forge/src/cuda/mxfp8.rs`
- `crates/calyx-forge/src/quant/mod.rs`
- `crates/calyx-forge/src/quant/binary.rs`
- `crates/calyx-forge/src/quant/turboquant.rs`
- `crates/calyx-forge/src/quant/qjl.rs`
- `crates/calyx-forge/src/quant/rotation.rs`
- `crates/calyx-forge/src/quant/mxfp4_codec.rs`
- `crates/calyx-forge/src/autotune/mod.rs`
- `crates/calyx-forge/src/vram/mod.rs`
- `crates/calyx-forge/src/vram/budget.rs`
- `crates/calyx-forge/src/vram/admission.rs`
- `crates/calyx-forge/src/vram/oom_guard.rs`
- `crates/calyx-forge/src/vram/lru_evict.rs`
- `crates/calyx-forge/src/vram/yield_policy.rs`
- `crates/calyx-forge/src/compression_report/mod.rs`
- `crates/calyx-forge/src/compression_report/types.rs`

Planning docs cross-checked: `docs/dbprdplans/13_FORGE_MATH_RUNTIME.md` and
`docs/dbprdplans/23_ARRAY_MATH_STORAGE_COMPRESSION.md`.

See also [08_registry_lenses.md](08_registry_lenses.md), [09_sextant_search.md](09_sextant_search.md),
[05_core.md](05_core.md). Cross-references in the planning text to issue #338
("implementation honesty") concern the deferred-ops contract documented in §2.

---

## 1. The `Backend` trait and the math operations

The central abstraction is the `Backend` trait (`src/backend.rs`). It is the
"Stage 2" contract: one trait, two implementations (`CpuBackend`,
`CudaBackend`). `Backend: Send + Sync`.

### 1.1 `Backend` trait signatures

| Method | Signature | Computes |
|---|---|---|
| `gemm` | `fn gemm(&self, a: &[f32], b: &[f32], m: usize, k: usize, n: usize, out: &mut [f32]) -> Result<()>` | column-major `C = A·B`, A is `m×k`, B is `k×n`, out is `m×n` |
| `cosine` | `fn cosine(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()>` | cosine of one query row `a` against each candidate row in `b`; one score per row of `out` |
| `dot` | `fn dot(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()>` | dot product of query vs each candidate row |
| `l2` | `fn l2(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()>` | **squared** L2 distance (the code computes `Σ(q-c)²`, not its sqrt) |
| `normalize` | `fn normalize(&self, vecs: &mut [f32], dim: usize) -> Result<()>` | in-place per-row L2 normalization (each row scaled by `1/‖row‖`) |
| `topk` | `fn topk(&self, scores: &[f32], k: usize) -> Result<Vec<(usize, f32)>>` | top-k `(index, score)` pairs, descending by score, ties broken by lower index |
| `device_info` | `fn device_info(&self) -> DeviceInfo` | static device descriptor (no `Result`) |

Note shape conventions: `cosine`/`dot`/`l2` treat `a` as a single row of length
`dim`, `b` as `out.len()` candidate rows of length `dim` each (validated by
`check_shape_2d`). GEMM is column-major (`col_major(row, col, rows) = col*rows + row`).

### 1.2 Shipped vs deferred operations (#338 honesty contract)

`src/backend.rs` exposes two string-slice constants enumerating the contract:

```rust
pub const FORGE_SHIPPED_BACKEND_OPS: &[&str] =
    &["gemm", "cosine", "dot", "l2", "normalize", "topk", "device_info"];
pub const FORGE_DEFERRED_BACKEND_OPS: &[&str] =
    &["knn", "histogram_nmi", "spmm_sparse_ops", "bilinear_cross_term",
      "graph_ops", "colbert_maxsim"];
```

The deferred list is documented as PRD-listed Forge operations intentionally
**not** implemented in the Stage 2 trait (tracked at issue #338). Planning docs
13 and 23 list a much larger catalog (knn/KSG MI, histogram/NMI, sparse/spmm,
bilinear cross-terms, graph SCC/betweenness/LP, ColBERT MaxSim); only the seven
shipped ops exist as `Backend` methods. This is a real gap, intentionally
declared in source (see §10).

Additional public constant: `CUDA_EXACT_TOPK_MAX_K: usize = 1024` — CUDA `topk`
is exact only for global `k <= 1024`; larger `k` fails loud (see §3.4).

### 1.3 Public structs / enums in `backend.rs`

| Item | Definition |
|---|---|
| `BackendKind` (enum) | `Cpu`, `Cuda`. Serde `rename_all = "lowercase"`; `Display` → `"cpu"`/`"cuda"`. |
| `DeviceInfo` (struct) | `kind: BackendKind`, `name: String`, `avx512: bool`, `vram_mib: Option<u64>`. `Default` = `{Cpu, "cpu", false, None}`. |
| `BestConfig` (struct) | `backend: BackendKind`, `tile_m: usize`, `tile_n: usize`, `tile_k: usize`, `extra: HashMap<String,String>`. Autotune winner record. |
| `Result<T>` (alias) | `std::result::Result<T, ForgeError>`. |

There is **no dynamic backend-dispatch enum** that selects CPU-vs-CUDA at
runtime in this crate. Dispatch is by constructing either `CpuBackend` or
`CudaBackend` (both implement `Backend`); a caller holds `Box<dyn Backend>` /
`&dyn Backend`. See §6.

---

## 2. CPU SIMD backend

`CpuBackend` (`src/cpu/mod.rs`) is a unit-ish struct holding a single `avx512:
bool` detected at construction via `std::arch::is_x86_feature_detected!("avx512f")`
(false on non-x86_64). It exposes `avx512_available() -> bool` and
`simd_path() -> &'static str` (`"f32x16"` if AVX-512, else `"f32x8"`). If
AVX-512 is absent, `new()` logs `CALYX_FORGE_CPU_AVX512_UNAVAILABLE` and falls
back to an f32x8-compatible path.

SIMD is provided by the `wide` crate: `f32x16` (16-lane) and `f32x8` (8-lane).

### 2.1 Distance kernels (`src/cpu/distance.rs`)

All three batch ops validate inputs (`validate_batch`): query is shape `1×dim`,
candidates `out.len()×dim`, both checked finite. Per row they call a `wide`
reduction:

- **`dot_batch`** — `dot()` loops over 16-lane chunks: `sum += (load16(q) *
  load16(c)).reduce_add()`, then a scalar tail for the remaining `< 16` elements.
- **`cosine_batch`** — computes query norm via `sum_squares` (16-lane), then per
  candidate `dot_and_norm` (one pass accumulating both `q·c` and `c·c` in
  f32x16), `score = dot / (‖q‖·‖c‖)`. Zero/non-finite norm fails closed via
  `check_norm_positive`.
- **`l2_batch`** — `l2_squared()`: `diff = load16(q) - load16(c); sum += (diff*diff).reduce_add()`.

The vectorization width is fixed at **16** for distance/normalize (`f32x16`
unconditionally, regardless of detected AVX-512); the 8-lane width only appears
in GEMM. `load16` copies a 16-element slice into a `[f32;16]` and constructs
`f32x16::from(lanes)`. Determinism comments state chunks are reduced in
ascending input-offset order, one `reduce_add()` subtotal per chunk.

### 2.2 GEMM (`src/cpu/gemm.rs`)

Constants: `TILE_M = 64`, `TILE_K = 64`. `gemm_f32` validates shapes/finiteness,
zero-fills `out`, returns early for `m==0 || n==0`, then dispatches:

- If `avx512f` detected → `gemm_tiled_f32x16`.
- Else → `gemm_tiled_f32x8`.

Both tile rows by `TILE_M` and depth by `TILE_K`, computing each output cell as a
dot product. The 8-lane path (`dot_f32x8`) uses `f32x8 * f32x8 .reduce_add()`.
The 16-lane path (`dot_f32x16`) deliberately does **not** do a full f32x16 tree
reduction: it multiplies in `f32x16`, then reduces the 16 products as **two
explicit 8-element scalar subtotals** (the in-code `DETERMINISM` comment: a full
f32x16 tree reduction "drifts from cuBLAS in near-zero cancellation cells"). This
is a deliberate bit-parity measure (see §4).

### 2.3 Normalize (`src/cpu/normalize.rs`)

`normalize_f32(vecs, dim)`: validates `dim`/length (length must be a multiple of
`dim`; `dim==0` valid only for empty), checks finiteness, then for each row
computes `norm = sqrt(sum_squares(row))` (f32x16), fails closed on zero/non-finite
norm, and scales the row by `1/norm` using `f32x16::splat(scale)`.

### 2.4 Top-k (`src/cpu/topk.rs`)

`topk_f32(scores, k)`: returns `Vec::new()` for `k==0` or empty input; checks
finiteness (NaN fails closed). Uses a min-heap (`BinaryHeap<Reverse<RankedScore>>`)
of capacity `k.min(len)`, then sorts descending. `RankedScore` ordering uses
`f32::total_cmp` on score, then **lower index wins ties** (`other.index.cmp(&self.index)`
inside `Reverse`). Complexity `O(n log k)`.

### 2.5 Numeric guards (`src/cpu/guard.rs`)

Shared fail-closed helpers, re-exported as `cpu::{check_finite, check_norm_positive, check_shape_2d}`:

| Function | Behavior |
|---|---|
| `check_finite(slice, op)` | Errors `NumericalInvariant` on first non-finite (reports index). |
| `check_norm_positive(norm, op, row)` | OK iff `norm > 0 && finite`; else `NumericalInvariant`. |
| `check_shape_2d(slice, rows, cols, name)` | Errors `ShapeMismatch` unless `slice.len() == rows*cols` (checked-mul guards overflow). |

---

## 3. CUDA backend

The entire `cuda` module is `#[cfg(feature = "cuda")]` (`lib.rs` line 7–8). It
is built on `cudarc` with features `std, driver, cublas, nvrtc,
dynamic-loading, cuda-13020` (`Cargo.toml`). When the feature is off, none of
`CudaBackend`, `CudaContext`, grouped/ragged GEMM, or the CUDA VRAM probe types
are compiled or exported.

### 3.1 The `cuda` feature flag and build (`build.rs`)

`build.rs` compiles the `.cu` kernels **only if the `cuda` feature is enabled**
(`cuda_feature_enabled()` checks `cfg!(feature="cuda")` or `CARGO_FEATURE_CUDA`);
otherwise it emits a warning and skips. Constants: `CUDA_PATH_DEFAULT =
"/usr/local/cuda-13.2"`, `CUDA_ARCH = "sm_120"`. `nvcc` is located at
`$CUDA_PATH/bin/nvcc` (panics if missing). For each kernel it compiles **both a
PTX and a cubin** with deterministic math flags:

```
-arch=sm_120 -O3 --ftz=false --prec-div=true --prec-sqrt=true --fmad=false -Xcompiler -fPIC
```

`--fmad=false` (no fused multiply-add) and `--prec-*=true` are the
GPU-side bit-parity controls (see §4). It explicitly does **not** use
`--use_fast_math` (asserted by a kernels.rs test). Output paths are exported as
`cargo:rustc-env` vars (`FORGE_{DISTANCE,TOPK,MXFP4_GEMM}_{PTX,CUBIN}_PATH`),
which `src/cuda/kernels.rs` embeds with `include_bytes!(env!(...))`.

The three `.cu` kernel sources: `distance`, `topk`, `mxfp4_gemm`.

### 3.2 CUDA context (`src/cuda/context.rs`)

`CudaContext` wraps `Arc<cudarc CudaContext>` plus `determinism: bool`,
`device_idx`, `name`, `compute_capability: (i32,i32)`, `total_mem_mib`,
`free_mem_mib_at_init`, and two lazily-loaded module caches
(`OnceLock<Arc<CudaModule>>` for the distance and topk PTX modules).

`init_cuda(device_idx, determinism) -> Result<CudaContext>` initializes the
device, queries name/compute-capability/`mem_get_info`, and **fails closed** if
free VRAM `< MIN_FREE_VRAM_MIB = 4096` (4 GiB) — the remediation string names
CUDA 13.2 at `/usr/local/cuda-13.2` and the RTX 5090. `query_device_info` returns
a `DeviceInfo { kind: Cuda, name, avx512: false, vram_mib: Some(total_mem_mib) }`.
`free_device_vram_bytes()` calls `cudaMemGetInfo` live on every call (never
`nvidia-smi`, never a fixed 32 GiB); a driver error surfaces as
`DeviceUnavailable` (fail-closed = treat as over-budget).

A test asserts the device is an RTX/5090 with compute capability `(12, 0)` and
`>= 30000` MiB VRAM — i.e., the deployment target is a single Blackwell RTX 5090
(sm_120). MXFP4/MXFP8 paths gate on `compute >= (12,0)` (§3.5).

### 3.3 The `.cu` kernels

| `.cu` file | `extern "C"` kernel(s) | What it computes |
|---|---|---|
| `kernels/distance.cu` | `cosine_batch_f32`, `dot_batch_f32`, `l2_batch_f32`, `normalize_rows_f32` | one block per candidate/row, 256 threads, `__launch_bounds__(256)`. Shared-memory block reduction. NaN/Inf set a `bad` flag → output `NAN`. Cosine writes sentinel `-2.0` when the denominator is zero. Normalize scales each element by `1/‖row‖` or writes `NAN`. |
| `kernels/topk.cu` | `bitonic_topk_f32` | `TOPK_BLOCK = 1024` threads/block; loads a 1024-element chunk into shared mem, runs a full **bitonic sort** with deterministic tie-break (lower index wins), writes the top `k`. NaN in a chunk emits sentinel index `-1`, score `-2.0`. |
| `kernels/mxfp4_gemm.cu` | `gemm_mxfp4_fp32_accum_kernel` | one thread per output cell (`__launch_bounds__(128)`); decodes MXFP4 nibbles (block size 32, E8M0 scale `ldexpf(1, scale-127)`, signed code `clamp(code,14)-7`, ×`scale/7`, code 15 → 0) and accumulates the dot in **fp32**; non-finite sum → `NAN`. |

`distance.cu` reductions iterate `stride = 128 → 1` halving with `__syncthreads()`
(determinism comment: ties broken by index, no warp-divergent index compares).

### 3.4 CUDA op host wrappers

`CudaBackend` (`src/cuda/mod.rs`) holds a `CudaContext` and implements
`Backend` by delegating to host functions that copy host→device, launch, sync,
copy back, and re-check finiteness:

- `distance::{cosine_host, dot_host, l2_host, normalize_host}` →
  `cosine_batch_gpu`/`dot_batch_gpu`/`l2_batch_gpu`/`normalize_rows_gpu`. Launch
  config: grid = `n_cands` (or `rows`) blocks, block = `BLOCK_THREADS = 256`.
  `check_device_output` re-reads results; cosine treats `<= -1.5` as the zero-norm
  sentinel → `NumericalInvariant`.
- `gemm::gemm_host` → `gemm_cublas` (cuBLAS `cublasSgemm_v2` via
  `cudarc::cublas`, `CUBLAS_OP_N`/`N`, α=1, β=0, column-major leading dims
  `lda=m, ldb=k, ldc=m`). Zero-work shapes memset the output.
- `topk::topk_host` → `topk_gpu`. Chunks the input into 1024-element blocks,
  launches `bitonic_topk_f32` (one block per chunk, 1024 threads), reads back
  per-chunk top-`k`, and **merges chunks on the CPU** (`merge_chunks` sorts by
  score desc, index asc, truncates to `k`). Guards: `k_eff > CUDA_EXACT_TOPK_MAX_K
  (1024)` returns `ShapeMismatch` with remediation "exact only for global k <=
  1024; use CPU topk or add a multi-pass exact CUDA merge". Negative sentinel
  index / non-finite score → `NumericalInvariant`; out-of-range index →
  `DeviceUnavailable`.

`gemm.rs` also ships `bench_gemm_cublas` / `bench_gemm_reference_cublas`
(GFLOP/s benchmarks, 5 warmup iters) and `probe_allocation(ctx, bytes)` which
fails closed if `requested_bytes > free_bytes` from `mem_get_info`.

### 3.5 Quantized CUDA GEMM paths

| Function | File | Path |
|---|---|---|
| `gemm_mxfp4_fp32_accum` | `cuda/gemm/mxfp4_path.rs` | Flattens `MxFp4Block` codes+scales, copies to device, launches the custom `gemm_mxfp4_fp32_accum_kernel` (128 threads), fp32 accumulate, finiteness check. Gated `ensure_mxfp4_sm120`: `compute >= (12,0)` else `DeviceUnavailable("MXFP4 requires sm_120 (Blackwell)")`. |
| `gemm_mxfp8_fp32_accum` | `cuda/gemm/mxfp8_path.rs` | **Decodes** MXFP8 blocks to fp32 on the host, then runs standard `gemm_cublas`. Same sm_120 gate. (No dedicated MXFP8 device kernel; cuBLAS does the matmul.) |

A source comment in `mxfp4_path.rs` notes the current cuBLAS C surface via
`cudarc` has no native FP4 GEMM entry point, so the custom kernel is a fallback;
an optimized CUTLASS 3.x grouped MXFP4 path is referenced as future work.

### 3.6 Grouped & ragged GEMM (`grouped_gemm.rs`, `ragged_gemm.rs`)

These implement the planning docs' "grouped GEMM = one launch for the whole
panel regardless of N":

- `GemmProblem { m, k, n, a_offset, b_offset, c_offset }` — one matmul within a
  shared slab.
- `GroupedGemmPlan` — holds `problems: Vec<Option<GemmProblem>>`, `slot_ids`,
  `absent_sentinel_ranges: Vec<AbsentSlotSentinel>`, `active:
  Vec<ActiveGemmProblem>`, `execution_mode: GroupedGemmExecutionMode`, and three
  device slabs (`a_slab`, `b_slab`, `c_slab`).
- `GroupedGemmExecutionMode` (enum): `NotRun`, `NoActiveProblems`,
  `GroupedBatched`, `SequentialFallback` (each has an `as_str()`).
- `AbsentSlotSentinel { flat_idx, c_offset, len }` marks absent slots; the
  absent-output sentinel is `f32::NAN`.
- Builders: `build_grouped_gemm_plan`; executors `execute_grouped_gemm`
  (`AllowSequential` fallback) and `execute_grouped_gemm_strict`
  (`FailIfGroupedUnsupported`); `read_grouped_gemm_output`.
- `RaggedBatch { n_constellations, n_slots, plan, ctx }` wraps a 2-D
  (constellation × slot) ragged problem list: `build_ragged_batch`,
  `build_ragged_batch_from_slabs`, `extract_ragged_results` /
  `try_extract_ragged_results`.

---

## 4. Bit-near parity (CPU ↔ GPU)

Parity is the A13 contract ("CPU and GPU paths must agree within a declared
numerical tolerance on a golden set"). The code enforces it by controlling
reduction order and float contraction on both sides:

**GPU side (`build.rs`):** `--fmad=false` (disables fused multiply-add),
`--prec-div=true`, `--prec-sqrt=true`, `--ftz=false`, and no `--use_fast_math`.
A test in `kernels.rs` (`build_script_uses_explicit_deterministic_math_flags`)
asserts these flags are present and `--use_fast_math` is absent.

**CPU side:** the AVX-512 GEMM path (`dot_f32x16`) deliberately reduces 16
products as two 8-wide scalar subtotals rather than an f32x16 tree, with an
in-code comment that a full tree reduction "drifts from cuBLAS in near-zero
cancellation cells." Distance/normalize use fixed ascending-offset chunk order.

**Tolerance constants (from tests — there is no single exported parity epsilon):**

| Context | Tolerance | Source |
|---|---|---|
| CUDA GEMM vs CPU GEMM | relative `<= 1e-3 * max(|expected|, 1)` (`close_enough`) | `cuda/gemm.rs` tests |
| CUDA GEMM perf vs raw cuBLAS | ratio `>= 0.90` | `cuda/gemm.rs` `perf_vs_cublas` |
| MXFP4 GEMM vs CPU GEMM | max relative `<= 0.05` (5%) | `mxfp4_path.rs` |
| MXFP8 GEMM vs CPU GEMM | max relative `<= 0.02` (2%) | `mxfp8_path.rs` |
| CPU dot vs scalar reference | abs `<= 1e-5` | `cpu/distance.rs` proptest |
| TurboQuant unbiased dot mean error | `<= 0.05` (Bits3p5), `<= 0.10` (Bits2p5) | `quant/qjl.rs` |

The CPU↔GPU contract uses these in tests guarded by `crate::cuda::test_lock()`
(a process-global mutex serializing GPU tests). There is no separate "golden
corpus" file checked into this crate; parity is asserted op-by-op in tests run on
the aiwonder GPU box.

---

## 5. Quantization

`QuantLevel` (`src/quant/mod.rs`) enumerates **seven** levels. The shared
container is `QuantizedVec { level, dim, bytes: Vec<u8>, scale: f32, seed_id:
[u8;32] }`. The `Quantizer` trait: `encode(&[f32]) -> QuantizedVec`,
`decode(&QuantizedVec) -> Vec<f32>`, `dot_estimate(a, b) -> f32`, `level()`,
`dim()`.

### 5.1 The quantization levels

| `QuantLevel` | `bits_per_channel()` | `is_lossy()` | Codec | Scheme |
|---|---|---|---|---|
| `F32` | 32.0 | false | (none / passthrough) | full precision |
| `Bits8` | 8.0 | false | — (declared non-lossy; no scalar-int8 codec ships) | 8-bit |
| `Bits8Fp` | 8.0 | true | `MxFp4Codec` (MXFP8 fallback) | MXFP8 microscaling (E4M3 + E8M0) |
| `Bits4Fp` | 4.0 | true | `MxFp4Codec` | MXFP4 microscaling (E8M0 block scale) |
| `Bits3p5` | 3.5 | true | `TurboQuantCodec` | rotate → 7-bit scalar quant + 1-bit QJL residual |
| `Bits2p5` | 2.5 | true | `TurboQuantCodec` | rotate → base-5 scalar quant + 1-bit QJL residual |
| `Bits1` | 1.0 | true | `BinaryCodec` | rotate → sign bits (1-bit) |

`is_lossy()` is `false` only for `F32` and `Bits8`. (`Bits8` has no encode/decode
implementation in this crate — it is declared in the enum but no codec produces
it; the only 8-bit codec output is `Bits8Fp` via MXFP8.)

### 5.2 Rotation (`src/quant/rotation.rs`) — shared front-end

TurboQuant and Binary share a data-oblivious random rotation.
`CURRENT_SEED_VERSION = 1`. `RotationSeed { id: [u8;32], version: u8, dim:
usize, diagonal: Vec<f32> }` where `diagonal` is a ±1 Rademacher sign vector.

- `new_seed(dim, entropy)`: `rng_seed = SHA-256(entropy ‖ dim_le_u64)`; a
  `ChaCha8Rng` draws `dim` random ±1 signs; `id = SHA-256(diagonal_bytes ‖
  version ‖ dim)`. Deterministic for fixed entropy.
- `apply_rotation`: a **block Hadamard** transform (largest power-of-two blocks,
  in-place Walsh–Hadamard, each block scaled by `1/sqrt(block_len)`) followed by
  the ±1 diagonal sign flip. `apply_inverse_rotation` reverses (sign then
  Hadamard). The transform is an isometry (norm-preserving, verified by tests).
- `seed_id` serializes as a 64-char hex string. `verify_current_version()` fails
  closed with `SeedVersionMismatch` if `version != 1`.

### 5.3 TurboQuant (`src/quant/turboquant.rs`) — Bits3p5 / Bits2p5

`TurboQuantCodec { seed, rademacher, level }`. Only `Bits3p5` and `Bits2p5` are
valid (else `QuantError`). A second seed (`rademacher`) is derived
deterministically: `new_seed(dim, "calyx-qjl-rademacher-v1" ‖ seed.id ‖ version)`.

**Encode steps:**
1. Verify seed version; check dim and finiteness (non-finite → `NumericalInvariant`).
2. `apply_rotation(seed, vec)` → `rotated`.
3. `scale = max(|rotated_i|)` (per-vector amplitude).
4. Scalar quantize to integer codes:
   `code = clamp(round((value/scale + 1) * max_code / 2), 0, max_code)`,
   where `max_code = level_steps(level) - 1`.
5. Pack codes; dequantize back (`decoded`) for the residual.
6. Encode the 1-bit QJL residual on `(rotated - decoded)`.
7. Append the QJL section to `bytes`.

**Level constants & layout:**

| Level | code steps | code width | packing |
|---|---|---|---|
| `Bits3p5` | `BITS3P5_LEVELS = 128` (`1 << 7`) | 7 bits/coord (`BITS3P5_CODE_BITS = 7`) | bitstream, 8 coords = 56 bits = 7 bytes; `packed_len = ceil(dim*7 / 8)` |
| `Bits2p5` | `BITS2P5_LEVELS = 5` | base-5, 4 coords packed into a 10-bit lane (`c0 + 5·c1 + 25·c2 + 125·c3`), upper 6 bits padding | 4 coords / 2 bytes; `packed_len = ceil(dim/4) * 2` |

**Dequantize:** `value = code * (2·scale)/max_code - scale` (and `scale==0` →
all zeros).

### 5.4 QJL residual (`src/quant/qjl.rs`) — the unbiased-inner-product fixup

`QJL_SECTION_TAG = 0x01`. `QjlResidual { bits: Vec<u8>, rademacher_seed:
[u8;32] }`.

- `encode_qjl_residual(rotated, scalar_decoded, rademacher)`: for each coord,
  `residual = (rotated - decoded) * rademacher_sign`; the **sign** of the residual
  is stored as one bit (`bits[i] = 1` iff residual > 0). `qjl_bits_len = ceil(dim/8)`.
- On-disk QJL section: `[tag:1][rademacher_seed:32][bits:ceil(dim/8)]`; total
  `qjl_section_len = 1 + 32 + ceil(dim/8)`.
- `dot_qjl_correction(qa, qb, rademacher, scale_a, scale_b) = scale_a·scale_b ·
  (Σ bipolar(qa_i)·bipolar(qb_i)) / dim`, where `bipolar(bit) = ±1`.
- `dot_estimate_unbiased(codec, a, b)`: decodes both scalar parts, sums their
  fp32 dot, then **adds** the QJL correction → unbiased inner-product estimate.
  Seed/rademacher mismatches fail closed.

This is `TurboQuantCodec::dot_estimate`: scalar-quant dot is biased; the 1-bit
QJL term de-biases it.

### 5.5 Binary (`src/quant/binary.rs`) — Bits1

`BinaryCodec { seed }`. Encode: verify version, check dim/finiteness,
`apply_rotation`, then **pack sign bits** (`bit i = 1` iff rotated_i > 0). Scale
stored is the binary amplitude `1/sqrt(dim)`. Packed length `ceil(dim/8)` bytes;
non-zero padding bits are rejected on validation.

Decode: reconstruct each coord as `±amplitude` by bit, then `apply_inverse_rotation`.
`dot_estimate` = `hamming_dot_estimate(a, b) = 1 - 2·mismatches/dim` (the
Hamming-derived cosine estimate). `binary_prefilter(query, candidates, keep)`
ranks candidates by Hamming dot estimate and returns the top `keep` indices — the
"binary recall prefilter funnel."

### 5.6 MXFP4 / MXFP8 microscaling (`cuda/mxfp4.rs`, `cuda/mxfp8.rs`, `quant/mxfp4_codec.rs`)

Both are MX (microscaling) block formats with a shared **E8M0** power-of-two
block scale.

- **MXFP4** (`MXFP4_BLOCK_SIZE = 32`, `MXFP4_PACKED_BYTES = 16`): per 32-element
  block, `scale_e8m0 = clamp(floor(log2(abs_max)), -127, 127) + 127`;
  `e8m0_scale = 2^(byte-127)`. Each value → a 4-bit signed code:
  `code = round(clamp(value/scale, -1, 1) * 7) + 7` (zero → code 7, the zero
  code; never produces the reserved NaN code 15; non-zero values forced to ≥1
  magnitude to preserve sign). Decode: `(code-7) * scale / 7`, code 15 → 0.
  `MxFp4Block { codes: [u8;16], scale_e8m0: u8 }`. Serialized block = 17 bytes.
- **MXFP8** (`MXFP8_BLOCK_SIZE = 32`, `MXFP8_BLOCK_BYTES = 33`): per-element
  **E4M3** byte (bit 7 sign, bits 6..3 exponent bias 7, bits 2..0 mantissa)
  chosen by exhaustive nearest-code search; fail-closed (never emits NaN/Inf
  codes). `MxFp8Block { codes: [u8;32], scale_e8m0: u8 }`.

`MxFp4Codec { dim, assay_safe_slots: BTreeMap<String, AssayQuantSafety> }` is the
intelligence-gated dispatcher between the two:

- `encode_for_slot(slot, vec)` → MXFP4 (`Bits4Fp`) **only if** the slot is in the
  Assay-safe set; otherwise it **falls back to MXFP8** (`Bits8Fp`).
- `encode_assay_checked(slot, vec, assay_safe: bool)` is the explicit form.
- `QuantizedVec` from this codec uses `scale = 0.0` and a zero seed_id (scale
  lives inside the blocks, not the envelope).

`AssayQuantSafety { baseline_bits, quantized_bits, cosine, far_delta }` with
admission thresholds `MIN_RETAINED_FRACTION = 0.95`, `MIN_COSINE = 0.99`,
`MAX_FAR_DELTA = 0.01`; `passes()` requires retained-bits fraction ≥ 0.95, cosine
≥ 0.99, FAR delta ≤ 0.01, all finite. This is the source-side realization of the
A25 "measured intelligence preservation" contract from planning doc 23.

### 5.7 `dot_estimate` summary by codec

| Codec | `dot_estimate` |
|---|---|
| `BinaryCodec` | Hamming: `1 - 2·mismatches/dim` |
| `TurboQuantCodec` | scalar dot + QJL correction (unbiased) |
| `MxFp4Codec` | decode both to fp32, raw fp32 dot (no QJL — Assay already gates FP4 admission) |

---

## 6. Backend selection / dispatch / feature gating / fallback

- **Backend dispatch type:** there is no runtime enum that auto-selects a
  backend. `CpuBackend` and `CudaBackend` both implement the `Backend` trait; a
  consumer chooses one and holds it as `dyn Backend`. `BackendKind` (`Cpu`/`Cuda`)
  is a descriptor used in `DeviceInfo`/`BestConfig`/autotune keys, not a dispatcher.
- **Feature gating:** the `cuda` feature (`default = []`, so **off by default**)
  enables `dep:cudarc` and compiles the whole `cuda` module plus the CUDA-only
  exports (`CudaBackend`, `CudaContext`, grouped/ragged GEMM, `CudaVramProbe`,
  `CudaStream`, `RawCudaMalloc`). With the feature off, only the CPU backend,
  quantization, the (CPU-testable) VRAM accounting, autotune, and the MX codecs
  are available.
- **No silent fallback to CPU:** the design is **fail-closed**. CUDA host
  functions surface `DeviceUnavailable`/`NumericalInvariant` rather than falling
  back to CPU. The GEMM remediation string literally says "fail closed instead of
  CPU fallback." CUDA topk above `k=1024` errors rather than returning an
  approximate result. The only intra-crate "fallback" is `MxFp4Codec`'s
  Bits4Fp→Bits8Fp downgrade when a slot is not Assay-proven safe (§5.6), and
  grouped GEMM's `SequentialFallback` mode when batched grouped GEMM is
  unsupported.
- **Autotune cache** (`src/autotune/mod.rs`): `AutotuneCache` maps `AutotuneKey {
  op, shape, dtype, device, recall_tgt }` → `BestConfig` (the chosen backend +
  tile sizes), persisted as atomic-rename JSON. `recall_tgt` is quantized to
  centi-units for key equality. Promotion/exploration helpers (`Explorer`,
  `should_promote`, `promote_if_winner`, `rollback_promotion`, A/B `autotune`)
  live alongside; constants `EPSILON`, `MIN_PROMOTE_MARGIN`, `MIN_PROMOTE_TRIALS`.
  This is the mechanism by which a backend/config is *selected per (op, shape,
  device)* — but selection is recorded, not auto-applied within `Backend` calls.

---

## 7. VRAM budgeting & admission (supporting subsystem)

`src/vram/` enforces that Forge coexists with resident TEI containers on the
single GPU. Key public items:

| Item | Notes |
|---|---|
| `VramProbe` (trait) | `free_device_vram() -> Result<usize>`; production impl `CudaVramProbe` calls `cudaMemGetInfo`; tests inject a deterministic probe. Probe error = over-budget (fail-closed). |
| `VramBudgeter<P: VramProbe>` | atomic soft-cap + live-headroom accounting; `from_env`, `with_soft_cap`, `can_allocate`, `stats() -> VramStats`. |
| `DEFAULT_SOFT_CAP_BYTES` | `12 GiB`. `RESERVED_HEADROOM_BYTES` = `512 MiB`. Env `CALYX_FORGE_VRAM_BUDGET` (`VRAM_BUDGET_ENV`). |
| `Category` (enum) | `Serving`, `Anneal`. |
| `AdmissionController` / `AdmitDecision` | `AdmitDecision::{ Split { sub_batch_size }, Queue { deadline: Instant }, Fail }`. Lens admission: `admit_lens`, `LensAdmission`, `LensAdmissionRequest`, `LensAdmissionPlacement`. |
| `OomGuard` / `OomGuardStats` | last-resort CUDA OOM retry-with-batch-reduction; `DEFAULT_OOM_MAX_RETRIES = 3`; `CudaMalloc`/`RawCudaMalloc`/`CudaAllocError`. |
| `GpuBlockRegistry` / `BlockKind` | LRU eviction; `BlockKind::{General, Frontier}`; `BlockId`, `DevicePtr`, `BlockDeallocator`, `GpuBlockStats`. |
| `YieldPolicy` / `YieldStats` | Anneal background lane: `DEFAULT_ANNEAL_VRAM_CAP_BYTES = 2 GiB`, `DEFAULT_POWER_BACKOFF_THRESHOLD_W = 560`, `DEFAULT_ANNEAL_THROTTLE_SLEEP = 50 ms`; NVML power probe (`NvmlPowerProbe`, `PowerProbe`). Env `CALYX_ANNEAL_VRAM_BUDGET` (`ANNEAL_VRAM_BUDGET_ENV`). |
| `VramStats` | snapshot with `admission_metrics_text()` emitting Prometheus counters (`calyx_forge_vram_admission_*`, `forge_oom_*`, `forge_anneal_*`). |

The errors `VramBudget` (`CALYX_FORGE_VRAM_BUDGET`) and `LensVramBudget`
(`CALYX_VRAM_BUDGET_EXCEEDED`) belong to this subsystem.

---

## 8. Compression report (`src/compression_report/`)

`compression_report(input: CompressionReportInput) -> Result<CompressionReport>`
produces the auditable per-slot compression numbers described in planning doc 23
§4.5. `COMPRESSION_REPORT_SCHEMA_VERSION = 1`.

- `CompressionReportInput { vault_id, slots: Vec<CompressionSlotMeasurement>,
  kernel: KernelCompressionMeasurement }`.
- `CompressionReport { schema_version, vault_id, slots: Vec<CompressionSlotReport>,
  totals: CompressionTotals, kernel: KernelCompressionReport, intelligence_delta:
  IntelligenceDeltaReport, meaning_compression_yield }`.
- Per-slot report fields include `bits_per_channel`, `turboquant_floor_cosine_error`,
  `achieved_cosine_error`, `distortion_vs_floor`, `storage_compression_ratio`,
  bits/guard-FAR/guard-FRR/kernel-recall before/after deltas, and `passed_contract`.
- `IntelligenceDeltaReport` aggregates `min_bits_delta`, `max_cosine_error`,
  `max_guard_far_delta`, `max_guard_frr_delta`, `min_kernel_only_recall_delta`.

---

## 9. Error taxonomy (`src/error.rs`)

`ForgeError` is the crate-wide error enum (`Result<T> = Result<T, ForgeError>`).
`Display` renders `<CODE> <fields>\nRemediation: <text>`; `code()` returns a
literal first token. Every variant carries a `remediation` string (fail-closed
ethos).

| Variant | `code()` | Fields |
|---|---|---|
| `NumericalInvariant` | `CALYX_FORGE_NUMERICAL_INVARIANT` | `op, detail, remediation` |
| `DeviceUnavailable` | `CALYX_FORGE_DEVICE_UNAVAILABLE` | `device, detail, remediation` |
| `GpuError` | `CALYX_GPU_ERROR` | `detail, remediation` |
| `ShapeMismatch` | `CALYX_FORGE_SHAPE_MISMATCH` | `expected: Vec<usize>, got: Vec<usize>, remediation` |
| `Unimplemented` | `CALYX_FORGE_UNIMPLEMENTED` | `op, remediation` |
| `QuantError` | `CALYX_FORGE_QUANT_ERROR` | `op, level, detail, remediation` |
| `QuantIntelligenceLoss` | `CALYX_QUANT_INTELLIGENCE_LOSS` | `slot, detail, remediation` |
| `CacheError` | `CALYX_FORGE_CACHE_ERROR` | `op, path, detail, remediation` |
| `VramBudget` | `CALYX_FORGE_VRAM_BUDGET` | `detail, remediation` |
| `LensVramBudget` | `CALYX_VRAM_BUDGET_EXCEEDED` | `detail, remediation` |
| `SeedVersionMismatch` | `CALYX_FORGE_QUANT_SEED_VERSION` | `expected: u8, got: u8` (fixed remediation) |

---

## 10. Gaps / not covered

- **Deferred backend ops:** `knn`, `histogram_nmi`, `spmm_sparse_ops`,
  `bilinear_cross_term`, `graph_ops`, `colbert_maxsim` are declared in
  `FORGE_DEFERRED_BACKEND_OPS` but **not implemented** as `Backend` methods. The
  planning-doc catalogs (13/23) list them as design targets only (issue #338).
- **`Bits8`:** present in `QuantLevel`, declared non-lossy, but no codec in this
  crate produces or consumes it (the only 8-bit output is `Bits8Fp` via MXFP8).
- **No native FP4 cuBLAS GEMM:** the MXFP4 device path is a custom one-cell-per-
  thread kernel (fp32 accumulate); an optimized CUTLASS grouped MXFP4 tensor-core
  path is referenced in comments as future work. MXFP8 GEMM decodes to fp32 and
  uses cuBLAS rather than an FP8 tensor-core path.
- **No CPU↔GPU golden-corpus file in-crate:** parity is asserted per-op in tests
  (run on the aiwonder RTX 5090); tolerances are test-local constants (§4), not
  one exported epsilon. CUDA tests require the physical GPU and are serialized by
  a process-global `test_lock`.
- **No runtime backend auto-dispatch:** selection between CPU/CUDA is the
  caller's choice plus the recorded `AutotuneCache` winner; nothing in `Backend`
  switches backends automatically or falls back CPU↔GPU at call time.
- The autotune, VRAM, and compression-report subsystems are documented at the
  public-API level (§6–§8); their internal exploration/eviction algorithms are
  summarized, not traced step-by-step (they are adjacent to, not part of, the core
  math/quantization mandate of this doc).
