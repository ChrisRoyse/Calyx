# 06. Forge Math Runtime (calyx-forge)

The `calyx-forge` crate is Calyx's low-level numeric runtime. It provides a single
`Backend` abstraction implemented twice — a CPU SIMD backend (`wide` f32x8/f32x16)
and a CUDA `sm_120` backend — engineered for **bit-near parity** between the two.
On top of that it layers quantization codecs (TurboQuant, binary, MXFP4/MXFP8),
grouped/ragged GEMM, and a persisted autotune cache. Every public op is
**fail-closed**: any NaN/Inf, zero-norm, shape mismatch, seed mismatch, or
unprovable VRAM allocation raises a typed `ForgeError` rather than returning a
silently wrong value.

This document is derived strictly from source. Where a fact could not be
established from the code it is marked "Not determined from source".

See also: [05_query_engine.md](05_query_engine.md), [07_registry_lenses.md](07_registry_lenses.md).

## Source files covered

Core / abstraction
- `src/lib.rs` — crate root, public re-exports
- `src/backend.rs` — `Backend` trait, `BackendKind`, `DeviceInfo`, `BestConfig`, op catalog
- `src/error.rs` — `ForgeError` enum, error codes, `Display`

CPU SIMD backend
- `src/cpu/mod.rs` — `CpuBackend`, AVX-512 detection, trait impl
- `src/cpu/distance.rs` — cosine / dot / l2 batch kernels
- `src/cpu/gemm.rs` — tiled f32x16 / f32x8 GEMM
- `src/cpu/normalize.rs` — row normalization
- `src/cpu/topk.rs` — heap-based exact top-k
- `src/cpu/guard.rs` — finiteness / norm / shape guards

CUDA backend (`cuda` feature)
- `src/cuda/mod.rs`, `src/cuda/context.rs`, `src/cuda/distance.rs`, `src/cuda/topk.rs`,
  `src/cuda/gemm.rs`, `src/cuda/kernels.rs`
- `src/cuda/kernels/distance.cu`, `src/cuda/kernels/topk.cu`, `src/cuda/kernels/mxfp4_gemm.cu`
- `src/cuda/grouped_gemm.rs`, `src/cuda/ragged_gemm.rs`, `src/cuda/gemm/mxfp4_path.rs`, `src/cuda/gemm/mxfp8_path.rs`
- `build.rs` — nvcc PTX/cubin compilation pipeline

Quantization
- `src/quant/mod.rs` — `QuantLevel`, `Quantizer`, `QuantizedVec`
- `src/quant/rotation.rs` — Hadamard + Rademacher rotation seed
- `src/quant/turboquant.rs` — TurboQuant Bits3p5 / Bits2p5 codec
- `src/quant/qjl.rs` — QJL unbiased dot residual
- `src/quant/binary.rs` — 1-bit (Bits1) codec, Hamming prefilter
- `src/cuda/mxfp4.rs`, `src/cuda/mxfp8.rs`, `src/quant/mxfp4_codec.rs` — MX formats

Autotune
- `src/autotune/mod.rs`, `src/autotune/microbench.rs`, `src/autotune/explorer.rs`, `src/autotune/promotion.rs`

Parity / golden tests
- `tests/cuda_parity.rs`, `tests/cuda_parity_support.rs`, `tests/golden/`

Crate manifest: `Cargo.toml`.

---

## 1. Backend abstraction

### 1.1 The `Backend` trait

Defined in `src/backend.rs:34`. It is `Send + Sync` and exposes seven operations:

| Method | Signature | Semantics |
|---|---|---|
| `gemm` | `(&self, a: &[f32], b: &[f32], m, k, n, out: &mut [f32]) -> Result<()>` | C(m×n) = A(m×k)·B(k×n), **column-major** |
| `cosine` | `(&self, a: &[f32], b: &[f32], dim, out: &mut [f32]) -> Result<()>` | per-candidate cosine of one query vs N candidates |
| `dot` | `(&self, a, b, dim, out) -> Result<()>` | per-candidate dot |
| `l2` | `(&self, a, b, dim, out) -> Result<()>` | per-candidate **squared** L2 (not sqrt'd) |
| `normalize` | `(&self, vecs: &mut [f32], dim) -> Result<()>` | in-place L2 row normalization |
| `topk` | `(&self, scores: &[f32], k) -> Result<Vec<(usize, f32)>>` | exact descending top-k with index tie-break |
| `device_info` | `(&self) -> DeviceInfo` | backend identity / capabilities |

The shipped op catalog is encoded as constants for cross-engine contract tests:

| Constant | Value (`src/backend.rs`) |
|---|---|
| `FORGE_SHIPPED_BACKEND_OPS` | `["gemm","cosine","dot","l2","normalize","topk","device_info"]` (line 11) |
| `FORGE_DEFERRED_BACKEND_OPS` | `["knn","histogram_nmi","spmm_sparse_ops","bilinear_cross_term","graph_ops","colbert_maxsim"]` (line 22) |
| `CUDA_EXACT_TOPK_MAX_K` | `1024` (line 32) |

Supporting types:
- `BackendKind` (`src/backend.rs:54`) — `Cpu` | `Cuda`, serializes lowercase.
- `DeviceInfo` (line 78) — `{ kind, name: String, avx512: bool, vram_mib: Option<u64> }`.
- `BestConfig` (line 69) — `{ backend: BackendKind, tile_m, tile_n, tile_k: usize, extra: HashMap<String,String> }`; the cached autotune result.

### 1.2 Determinism and parity philosophy

The two backends are written to agree numerically. Both use **column-major**
layout for GEMM, identical reduction orders where it matters, and the CPU GEMM
deliberately splits an f32x16 product into two f32x8 subtotals (`src/cpu/gemm.rs:94-104`)
with a source comment stating a full f32x16 tree reduction "drifts from cuBLAS in
near-zero cancellation cells." The CUDA build flags (Section 2.4) disable fast-math
and FMA contraction for the same reason. Parity is verified by golden tests
(Section 3).

### 1.3 Error model

`ForgeError` (`src/error.rs:8`) is a 12-variant enum. Each variant has a stable
machine code via `code()` and a human remediation via `remediation()`; `Display`
emits `<CODE> <fields>\nRemediation: <text>`.

| Variant | Code |
|---|---|
| `NumericalInvariant` | `CALYX_FORGE_NUMERICAL_INVARIANT` |
| `DeviceUnavailable` | `CALYX_FORGE_DEVICE_UNAVAILABLE` |
| `GpuError` | `CALYX_GPU_ERROR` |
| `ShapeMismatch` | `CALYX_FORGE_SHAPE_MISMATCH` |
| `Unimplemented` | `CALYX_FORGE_UNIMPLEMENTED` |
| `QuantError` | `CALYX_FORGE_QUANT_ERROR` |
| `QuantIntelligenceLoss` | `CALYX_QUANT_INTELLIGENCE_LOSS` |
| `CacheError` | `CALYX_FORGE_CACHE_ERROR` |
| `VramBudget` | `CALYX_FORGE_VRAM_BUDGET` |
| `LensVramBudget` | `CALYX_VRAM_BUDGET_EXCEEDED` |
| `SeedVersionMismatch` | `CALYX_FORGE_QUANT_SEED_VERSION` |

The shared guard helpers (`src/cpu/guard.rs`) enforce the invariants before any
compute: `check_finite` (rejects NaN/Inf, reports the offending index),
`check_norm_positive` (rejects zero or non-finite norm, reports the row), and
`check_shape_2d` (validates `len == rows*cols`, overflow-checked).

---

## 2. Backend implementations

### 2.1 CPU SIMD backend

`CpuBackend` (`src/cpu/mod.rs:10`) holds a single bool `avx512`, detected at
construction via `std::arch::is_x86_feature_detected!("avx512f")` (line 91; always
`false` off x86_64). `simd_path()` returns `"f32x16"` when AVX-512 is present, else
`"f32x8"`. `device_info()` reports `name = "calyx-cpu"`, `vram_mib = None`.

The `wide` crate supplies the SIMD types. The distance/normalize kernels always use
`f32x16` lanes (16-wide load, `reduce_add()` per chunk, scalar tail); GEMM picks
f32x16 vs f32x8 at runtime.

| CPU kernel | File / fn | Algorithm |
|---|---|---|
| `gemm_f32` | `cpu/gemm.rs:9` | tiled, column-major, `TILE_M=64`, `TILE_K=64`; AVX-512 path splits f32x16 product into two f32x8 subtotals (determinism); fills output with 0 then writes each cell |
| `cosine_batch` | `cpu/distance.rs:6` | query norm once; per row `dot/(‖q‖·‖c‖)`, fail-closed on zero norm |
| `dot_batch` | `cpu/distance.rs:25` | per row dot via 16-wide `reduce_add` + scalar tail |
| `l2_batch` | `cpu/distance.rs:33` | per row Σ(q−c)² (squared L2) |
| `normalize_f32` | `cpu/normalize.rs:6` | per row `1/‖row‖` scale; `f32x16::splat` multiply |
| `topk_f32` | `cpu/topk.rs:7` | min-heap of size k; ties broken by **lower index** (`RankedScore::cmp` uses `total_cmp` then reverse index) |

CPU GEMM tiling constants: `TILE_M = 64`, `TILE_K = 64` (`cpu/gemm.rs:6-7`). The
tiling loop is `col → row_tile(step TILE_M) → row`, and the per-cell dot tiles the
K dimension in `TILE_K` blocks.

CPU top-k tie-break: the heap comparator (`cpu/topk.rs:52`) is
`score.total_cmp(other.score).then_with(|| other.index.cmp(&self.index))`, then the
final vector is sorted descending — yielding "higher score wins, ties to lower
index," matching the CUDA kernel.

### 2.2 CUDA backend (feature `cuda`)

`CudaBackend` (`src/cuda/mod.rs:35`) wraps a single `CudaContext`. `new()` calls
`init_cuda(0, false)`. Each `Backend` method copies host→device, launches the
kernel (or cuBLAS), copies device→host, and re-checks finiteness — fail-loud, **no
CPU fallback inside the CUDA backend.**

| Trait method | Dispatches to | File |
|---|---|---|
| `gemm` | `gemm::gemm_host` → cuBLAS `cublasSgemm_v2` | `cuda/gemm.rs:34` |
| `cosine` | `distance::cosine_host` → `cosine_batch_f32` | `cuda/distance.rs:80` |
| `dot` | `distance::dot_host` → `dot_batch_f32` | `cuda/distance.rs:98` |
| `l2` | `distance::l2_host` → `l2_batch_f32` | `cuda/distance.rs:116` |
| `normalize` | `distance::normalize_host` → `normalize_rows_f32` | `cuda/distance.rs:148` |
| `topk` | `topk::topk_host` → `bitonic_topk_f32` | `cuda/topk.rs:72` |
| `device_info` | `query_device_info` | `cuda/context.rs:118` |

GEMM is **not** a custom kernel for f32 — `gemm_host` calls cuBLAS SGEMM with
`alpha=1.0, beta=0.0, OP_N/OP_N, lda=m, ldb=k, ldc=m` (column-major).

#### CudaContext (`src/cuda/context.rs:11`)

Holds `Arc<CudarcContext>`, `determinism`, `device_idx`, `name`,
`compute_capability: (i32,i32)`, `total_mem_mib`, `free_mem_mib_at_init`, plus lazy
`OnceLock` PTX-module caches for distance and topk.

`init_cuda(device_idx, determinism)` (line 84): creates the cudarc context, reads
device name, compute capability, and `cudaMemGetInfo`; enforces a minimum free-VRAM
floor. `query_device_info` (line 118) builds `DeviceInfo { kind: Cuda, name,
avx512: false, vram_mib: Some(total_mem_mib) }`. `free_device_vram_bytes` (line 63)
does a live `cudaMemGetInfo` and fails loud (no zero fallback) — used by the VRAM
budgeter.

| CUDA context constant | Value | File |
|---|---|---|
| `BYTES_PER_MIB` | `1048576` | `cuda/context.rs:7` |
| `MIN_FREE_VRAM_MIB` | `4096` (4 GiB floor) | `cuda/context.rs:8` |
| CUDA remediation target | "CUDA 13.3 at /usr/local/cuda-13.3, RTX 5090" | `cuda/context.rs` |

Tests assert the deployment device reports compute capability `(12, 0)` and
≥30,000 MiB VRAM (RTX 5090).

### 2.3 CUDA kernels (`.cu`)

| Kernel | File | Block dim | Grid dim | Computes |
|---|---|---|---|---|
| `cosine_batch_f32` | `distance.cu:22` | 256 (`__launch_bounds__(256)`) | one block per candidate | `dot/(‖q‖·‖c‖)`; zero-norm → sentinel `-2.0f` |
| `dot_batch_f32` | `distance.cu:78` | 256 | one block per candidate | dot; NaN → `NAN` |
| `l2_batch_f32` | `distance.cu:116` | 256 | one block per candidate | Σ(q−c)² |
| `normalize_rows_f32` | `distance.cu:155` | 256 | one block per row | in-place `row·(1/‖row‖)` |
| `bitonic_topk_f32` | `topk.cu:19` | 1024 (`TOPK_BLOCK`) | `n.div_ceil(1024)` | per-chunk bitonic sort + top-k |
| `mxfp4_gemm` (Section 5) | `mxfp4_gemm.cu` | 128 (`MXFP4_THREADS`) | `ceil(m*n/128)` | dequant-in-register MXFP4 GEMM |

`BLOCK_THREADS = 256` (`cuda/distance.rs:11`) drives the distance/normalize launch;
shared-memory reductions use static `__shared__ float[256]` arrays and a tree
reduction starting `stride = 128`. The cosine zero-norm sentinel `-2.0f` is detected
host-side by `check_device_output` treating any value `<= -1.5` as a zero-norm error.

### 2.4 Build pipeline and `sm_120` (`build.rs`)

Kernels are compiled by `nvcc` at build time into **PTX** (loaded at runtime via
NVRTC `Ptx::from_src`) and embedded via `include_bytes!(env!(...))`. Parallel cubin
artifacts are also produced and embedded but the runtime distance/topk paths load
the PTX. The architecture constant is:

```rust
const CUDA_ARCH: &str = "sm_120";   // build.rs:6
```

It is injected as `-arch=sm_120` (a single `-arch`, not multi-`-gencode`) into both
the `--ptx` and `-cubin` nvcc invocations. The full deterministic flag list
(`deterministic_args`, build.rs:126) is:

```
-arch=sm_120 -O3 --ftz=false --prec-div=true --prec-sqrt=true
--fmad=false -Xcompiler -fPIC <--ptx|-cubin> -o <out> <src>
```

No `--use_fast_math`; FMA contraction is **off** (`--fmad=false`) — all for
bit-parity with the CPU path. nvcc is located via `CUDA_PATH` env (default
`/usr/local/cuda-13.3`); the build prints a warning unless it sees a CUDA 13.3
release. The kernel table (`KERNELS`) registers `distance`, `topk`, `mxfp4_gemm`,
each emitting `FORGE_<NAME>_PTX_PATH` / `FORGE_<NAME>_CUBIN_PATH` env vars. If the
`cuda` feature is off, nvcc is skipped entirely.

### 2.5 Cargo features

```toml
[features]
default = []
cuda = ["dep:cudarc"]    # Cargo.toml:9-11
```

The entire `cuda` module is gated behind the `cuda` feature, which enables the
optional `cudarc` dependency with features
`["std","driver","cublas","nvrtc","dynamic-loading","cuda-13020"]` (CUDA 13.3 runtime,
dynamic driver loading, NVRTC for PTX, cuBLAS for GEMM).

---

## 3. CPU/GPU bit-parity verification

Parity is enforced by `tests/cuda_parity.rs` against golden binaries in
`tests/golden/` (generated by `generate_golden.py`; manifest
`golden_manifest.json`). Tolerances (`tests/cuda_parity_support.rs`):

| Tolerance | Value |
|---|---|
| `PARITY_TOL` (relative) | `1e-3` |
| `PARITY_ABS_TOL` (absolute, near-zero) | `1e-6` |

**Parity check algorithm** (`parity_report` / `assert_parity`):
1. Run the op on both `CpuBackend` and `CudaBackend` over identical golden inputs.
2. For each element compute `abs_err = |cpu − gpu|` and `rel_err = abs_err/(|gpu| + 1e-8)`.
3. Track the worst relative and worst absolute element.
4. **Pass condition:** `max_rel_err <= PARITY_TOL` **OR** `max_abs_err <= PARITY_ABS_TOL`
   (the absolute floor lets near-zero cells pass even when relative error is large).
5. Otherwise `panic!("PARITY FAIL ...")` with the offending indices and values.

Golden parity tests cover gemm, cosine, dot, l2, normalize, and topk (the topk test
asserts CPU indices == GPU indices == the golden `topk_ref` indices exactly). A
separate `perf_vs_cublas` test asserts Forge GEMM throughput is **≥ 0.90×** raw
cuBLAS on `sm_120`. FSV readbacks are written to `CALYX_FSV_ROOT` when set. All
CUDA-gated tests are `#[ignore]` without the `cuda` feature.

---

## 4. Quantization

### 4.1 Levels and the `Quantizer` trait

`QuantLevel` (`src/quant/mod.rs:27`) enumerates the formats with their bit budgets:

| `QuantLevel` | bits/channel | lossy? | Codec |
|---|---|---|---|
| `F32` | 32.0 | no | (raw) |
| `Bits8` | 8.0 | no | Not determined from source (no dedicated codec read) |
| `Bits8Fp` | 8.0 | yes | MXFP8 fallback (`MxFp4Codec`) |
| `Bits4Fp` | 4.0 | yes | MXFP4 (`MxFp4Codec`) |
| `Bits3p5` | 3.5 | yes | TurboQuant (7-bit code) |
| `Bits2p5` | 2.5 | yes | TurboQuant (base-5 code) |
| `Bits1` | 1.0 | yes | `BinaryCodec` |

`is_lossy()` returns true for everything except `F32` and `Bits8`.

`Quantizer` trait (`src/quant/mod.rs:70`): `encode(&[f32]) -> QuantizedVec`,
`decode(&QuantizedVec) -> Vec<f32>`, `dot_estimate(a,b) -> f32`, `level()`, `dim()`.

`QuantizedVec` (`src/quant/mod.rs:78`):
`{ level: QuantLevel, dim: usize, bytes: Vec<u8>, scale: f32, seed_id: [u8;32] }`.

### 4.2 Rotation seed (Hadamard + Rademacher)

`RotationSeed` (`src/quant/rotation.rs:12`): `{ id: [u8;32], version: u8, dim,
diagonal: Vec<f32> }`, where `diagonal` is a ±1 (Rademacher) sign vector.
`CURRENT_SEED_VERSION = 1`.

- `new_seed(dim, entropy)` (line 32): seeds ChaCha8 from `SHA256(entropy ‖ dim_le)`,
  draws the ±1 diagonal, and sets `id = SHA256(diagonal_le ‖ version ‖ dim_le)`.
- `apply_rotation` (line 47): **block Hadamard transform** then per-element sign
  multiply. `apply_block_hadamard` decomposes `dim` into power-of-two blocks
  (largest-first) and runs an in-place Walsh–Hadamard butterfly, scaling each block
  by `1/√block_len` so the transform is **orthonormal** (L2-preserving).
- `apply_inverse_rotation` (line 61): signs first, then Hadamard (the transform is
  its own inverse up to the scale). Round-trips to within `1e-6`.

This rotation is the shared front-end of TurboQuant and the binary codec — it
spreads vector energy across coordinates so per-coordinate scalar quantization
behaves uniformly.

### 4.3 TurboQuant codec (Bits3p5 / Bits2p5)

`TurboQuantCodec` (`src/quant/turboquant.rs:16`) holds the rotation `seed`, a derived
`rademacher` seed (for QJL, Section 4.4), and a `level` (only `Bits3p5` or `Bits2p5`
accepted). Quantization constants:

| Constant | Value | Meaning |
|---|---|---|
| `BITS3P5_CODE_BITS` | `7` | bits per scalar code at Bits3p5 |
| `BITS3P5_LEVELS` | `128` (`1<<7`) | code levels at Bits3p5 |
| `BITS2P5_LEVELS` | `5` | base-5 levels at Bits2p5 |

**Encode steps** (`rotate_quantize_scalar_parts` + `encode`, lines 58-85):
1. Reject non-finite input (fail-closed).
2. `rotated = apply_rotation(seed, vec)`.
3. `scale = max(|rotated|)` (per-vector amplitude).
4. `code = clamp(round((value/scale + 1)·(max_code/2)), 0, max_code)` where
   `max_code = level_steps(level) − 1` (`quantize_codes`, line 199). This maps the
   symmetric range `[−scale, scale]` onto `0..=max_code`.
5. Pack codes (Section 4.3.1).
6. Compute the scalar `decoded` and append a QJL residual section (Section 4.4).
7. Emit `QuantizedVec { level, dim, bytes, scale, seed_id }`.

**Decode** (`dequantize_scalar`, line 187): for `scale != 0`,
`value = code·(2·scale)/max_code − scale`, then `apply_inverse_rotation`.

#### 4.3.1 Bit packing

- **Bits3p5** (`pack_bits3p5`, line 228): one 7-bit code per coordinate written
  little-endian into a bitstream, so 8 values occupy 56 bits = 7 bytes. Packed
  length = `(dim·7).div_ceil(8)` bytes. `write_bits`/`read_bits` operate bit-by-bit.
- **Bits2p5** (`pack_bits2p5`, line 244): four base-5 codes packed into one 10-bit
  lane as `c0 + 5·c1 + 25·c2 + 125·c3`, stored in 2 bytes (upper 6 bits padding).
  Packed length = `dim.div_ceil(4)·2` bytes → exactly 4 values per 2 bytes (2.5
  bits/value nominal once the QJL residual amortizes).

### 4.4 QJL unbiased dot residual

The scalar codes alone are biased; TurboQuant appends a 1-bit-per-coordinate QJL
(Quantized Johnson–Lindenstrauss) residual section so `dot_estimate` is unbiased.

`QjlResidual` (`src/quant/qjl.rs:7`): `{ bits: Vec<u8>, rademacher_seed: [u8;32] }`.

**Encode** (`encode_qjl_residual`, line 12): for each coordinate compute
`residual = (rotated − scalar_decoded)·sign` (sign from the Rademacher diagonal); set
bit `idx` iff `residual > 0`. This stores the **sign of the quantization residual** in
the Rademacher-rotated basis.

**Section layout** (`append_qjl_section`, line 116): `[tag 0x01][32-byte rademacher
seed][ceil(dim/8) residual bytes]`, appended after the scalar bytes. `read_qjl_section`
(line 122) validates total length and the tag.

**Unbiased dot** (`dot_estimate_unbiased`, line 84):
1. Verify both vectors share `seed_id`; decode both to scalar fp32.
2. `scalar_dot = Σ a·b` over the decoded scalars.
3. Read both QJL residual sections; verify rademacher seed matches the codec.
4. `correction = scale_a·scale_b·(Σ bipolar(a_i)·bipolar(b_i))/dim`, where
   `bipolar(bit)` is `+1`/`−1` (`dot_qjl_correction`, line 49).
5. Return `scalar_dot + correction`.

Tests bound mean absolute dot error to ≤0.05 (Bits3p5) and ≤0.10 (Bits2p5) over 1000
random unit pairs at dim 128.

### 4.5 Binary codec (Bits1)

`BinaryCodec` (`src/quant/binary.rs:12`) wraps a rotation seed. **Encode**: rotate,
then store one **sign bit** per coordinate (`pack_sign_bits`, bit set iff value > 0).
`scale = binary_amplitude(dim) = 1/√dim`. **Decode**: each bit → `±amplitude`, then
inverse rotation. Validation rejects nonzero padding bits in the final byte.

`hamming_dot_estimate` (line 88): `1 − 2·mismatches/dim` — the cosine-like estimate
from the Hamming distance of the two sign-bit strings. `binary_prefilter` (line 111)
scores candidates by Hamming dot, sorts descending (ties to lower index), and returns
the top `keep` indices — a cheap candidate prefilter feeding a finer rerank.

### 4.6 MXFP4 / MXFP8 element formats

Block-scaled microscaling formats with a shared per-block E8M0 power-of-two scale.

| Constant | Value | File |
|---|---|---|
| `MXFP4_BLOCK_SIZE` | `32` | `cuda/mxfp4.rs` |
| `MXFP4_PACKED_BYTES` | `16` (2 codes/byte) | `cuda/mxfp4.rs` |
| `MXFP4_EXP_BIAS` | `127` | `cuda/mxfp4.rs` |
| `MXFP4_MAX_SIGNED_CODE` | `7` | `cuda/mxfp4.rs` |
| `MXFP4_ZERO_CODE` | `7` | `cuda/mxfp4.rs` |
| `MXFP4_NAN_CODE` | `15` (→ 0.0) | `cuda/mxfp4.rs` |
| `MXFP8_BLOCK_SIZE` | `32` | `cuda/mxfp8.rs` |
| `MXFP8_BLOCK_BYTES` | `33` (32 codes + 1 scale) | `cuda/mxfp8.rs` |
| `E4M3_EXP_BIAS` | `7` | `cuda/mxfp8.rs` |

**MXFP4** packs 32 values into 16 bytes (two 4-bit codes per byte: even index → low
nibble, odd index → high nibble) plus one E8M0 scale byte. The 4-bit code is a
**symmetric signed integer** `code − 7 ∈ [−7, 7]` (it is *not* a true E2M1 float —
there is no E2M1 mantissa table in the source; code 15 is reserved → decodes to 0.0).

```
e8m0_scale(b)      = 2^(b − 127)
scale_byte(amax)   = floor(log2(amax)) + 127        (clamped to [-127,127]+127; 0 if amax==0)
encode code        = round(clamp(value/scale, -1, 1) * 7) + 7   (nonzero never collapses to 7)
decode value       = (code − 7) * scale / 7
```

**MXFP8** is **E4M3** (sign bit 7, exponent bits 6..3 bias 7, mantissa bits 2..0),
one byte per element + one E8M0 scale byte (33 bytes/block). The same
`scale_byte`/`e8m0_scale` machinery applies. `encode_e4m3` performs an exhaustive
nearest-code search over all 256 byte values (correctness-driven), and the codec is
fail-closed — it never emits NaN/Inf codes.

Both encoders run `check_finite` first and reject NaN/Inf with `NumericalInvariant`.

### 4.7 MxFp4Codec and the Assay intelligence gate

`MxFp4Codec` (`src/quant/mxfp4_codec.rs`) decides per slot whether FP4 is safe.
`AssayQuantSafety { baseline_bits, quantized_bits, cosine, far_delta }` passes iff:

| Gate | Threshold |
|---|---|
| retained bits fraction | `quantized_bits/baseline_bits >= 0.95` (`MIN_RETAINED_FRACTION`) |
| cosine vs baseline | `>= 0.99` (`MIN_COSINE`) |
| far-neighbor delta | `<= 0.01` (`MAX_FAR_DELTA`) |
| all four fields finite | required |

`record_assay_safety(slot, safety)` admits a slot **only if `passes()`**. At encode
time: assay-safe slots use **MXFP4** (`Bits4Fp`, 17 bytes/block serialized: 16 codes +
1 scale); all other slots fall back to **MXFP8** (`Bits8Fp`, 33 bytes/block). In both
cases `scale = 0.0` and `seed_id = ZERO_SEED` (the real scale lives inside each
block). This is the "intelligence-loss" guard that ties into the `QuantIntelligenceLoss`
error — see [07_registry_lenses.md](07_registry_lenses.md) for how lenses select slots.

---

## 5. MXFP4 grouped / ragged GEMM

### 5.1 MXFP4 GEMM kernel

`src/cuda/kernels/mxfp4_gemm.cu`, launched by `src/cuda/gemm/mxfp4_path.rs`.

- `MXFP4_THREADS = 128`; **one thread per output cell**, `grid = ceil(m*n/128)`, no
  shared-memory tiling (naive). Requires compute capability ≥ (12,0).
- Column-major, **fp32 accumulation**: each thread loops `depth` in `0..k`,
  dequantizing `A[row,depth]` and `B[depth,col]` in-register and accumulating
  `sum += decode·decode`. Output is `isfinite(sum) ? sum : NAN` (fail-closed).
- In-kernel dequant matches the Rust formula exactly:
  `2^(scale−127)` for the block scale, `(code−7)·scale/7` per element, code 15 → 0.
- Host layout: blocks are split into a contiguous 16-byte `codes` buffer and a
  separate 1-byte-per-block `scales` buffer (distinct from the codec's interleaved
  17-byte serialization).

The **MXFP8 GEMM path** (`gemm/mxfp8_path.rs`) has no custom kernel: it decodes
blocks to fp32 on the host, copies to device, and calls cuBLAS SGEMM.

### 5.2 Grouped GEMM

`src/cuda/grouped_gemm.rs`. Solves many small GEMMs that share a flat A/B/C device
slab, batching same-shape problems through `cublasSgemmGroupedBatched`.

Types:
- `GemmProblem` (Copy): `{ m, k, n, a_offset, b_offset, c_offset }` — element offsets
  into the slabs; matrices column-major (`lda=m, ldb=k, ldc=m`, `OP_N/OP_N`).
- `GroupedGemmPlan`: `problems`, `slot_ids`, `absent_sentinel_ranges`, `active`,
  `execution_mode`, and the three device slabs.
- `GroupedGemmExecutionMode`: `NotRun` | `NoActiveProblems` | `GroupedBatched` |
  `SequentialFallback`.
- `AbsentSlotSentinel`: `{ flat_idx, c_offset, len }` marking where an absent slot's
  NaN guard lives. `ABSENT_SENTINEL = f32::NAN`, compared bit-exact via `to_bits()`.

**Plan build** (`build_grouped_gemm_plan`): validate each present problem (bounds, i32
limits), then **sort active problems by `(k, n, m, slot_idx)`** so identical shapes
are adjacent. `check_finite` runs on A/B always, on C only when there are no absent
sentinels (sentinels are intentionally NaN). Host slabs are copied to device;
`execution_mode = NotRun`.

**Execute** (`execute_grouped_gemm` / `execute_grouped_gemm_strict`):
1. Re-validate against live device slab lengths.
2. Walk the sorted active list; each time `(m,k,n)` changes, open a new **group**
   (`alpha=1.0, beta=0.0, OP_N/OP_N, lda=m, ldb=k, ldc=m`, `group_size=0`); every
   problem increments its group's size and pushes per-matrix A/B/C base pointers.
   Result: one cuBLAS group per distinct shape.
3. Launch `cublasSgemmGroupedBatched`.
4. **Fallback:** a non-`CUBLAS_STATUS_NOT_SUPPORTED` error is fatal. On
   `NOT_SUPPORTED`: strict mode errors; `AllowSequential` loops each problem through
   `cublasSgemm_v2` (`SequentialFallback`). Success → `GroupedBatched`.
5. `check_device_output`: every active C region must be finite; every absent sentinel
   region must still bit-equal `f32::NAN` (proving absent slots were untouched).

### 5.3 Ragged GEMM

`src/cuda/ragged_gemm.rs` layers a rectangular `[n_constellations][n_slots]` view on
top of the grouped plan. `build_ragged_batch(ctx, Vec<Vec<Option<GemmProblem>>>)`
flattens row-major (every row must have `n_slots`), computes required slab lengths
from the max `offset + rows·cols`, zero-fills host slabs, and for each absent (None)
slot appends a 1-element `f32::NAN` sentinel to C with a recorded `AbsentSlotSentinel`.
`extract_ragged_results` reads C back and maps present slots to `Vec<f32>`, absent to
`None`, returning `Vec<Vec<Option<Vec<f32>>>>`. `build_ragged_batch_from_slabs` is the
caller-supplied-slab variant.

---

## 6. Autotune cache

`src/autotune/`. Picks and persists the best `BestConfig` per workload, with an
explore/exploit policy and an A/B promotion gate.

### 6.1 Cache key (`AutotuneKey`)

`src/autotune/mod.rs:27`. Fields: `op: String`, `shape: Vec<usize>`, `dtype: String`,
`device: String`, `recall_tgt: f32`. Because `f32` is not hashable, `Eq`/`Hash` use
`recall_quantum()` = `round(recall_tgt·100)` as an i32 (1%-buckets; NaN →
`i32::MIN`, saturating at the i32 bounds). So the key captures **5 dimensions**:
op, shape, dtype, device, and recall-target quantized to 1% buckets.
`default_for` sets `recall_tgt = 0.95`.

### 6.2 Storage and selection

`AutotuneCache` (`mod.rs:73`) maps `AutotuneKey → BestConfig` in a `HashMap` plus a
`PathBuf`. On disk it is **pretty JSON** as a sorted `Vec<{key, config}>`
(`PersistedCache`/`PersistedEntry`), so output is deterministic. No env var supplies
the path — it is passed explicitly to `load(path)`.

- `load` reads the file; `NotFound` → empty cache (not an error); malformed JSON →
  `CacheError`.
- `persist` is an **atomic write**: serialize → write temp (`<name>.tmp`) →
  `sync_all` (fsync) → `fs::rename` into place; temp removed on any failure.
  Entries sorted by `(op, shape, dtype, device, recall_quantum)`.
- `rollback(key, previous)` re-inserts a prior config.

The top-level `autotune(cache, key)` (`promotion.rs:90`) is a **pure read**: return
the cached `BestConfig` clone, or `BestConfig::default_for(key)` on miss. The default
uses `BackendKind::Cuda` when the `cuda` feature is compiled (else `Cpu`), with tiles
`tile_m=64, tile_n=64, tile_k=32` and `extra = { op, source: "autotune-default" }`.
The actual selection is performed by the explorer + promotion machinery, which writes
winners into the cache.

### 6.3 Microbenchmark

`src/autotune/microbench.rs`. `microbench(op, config, shape, ctx, iters)` dispatches
by op string (`gemm`, `cosine`, `grouped_gemm`, `turboquant_encode`; unknown →
`Unimplemented`). Each path computes a FLOP model (gemm `2·m·k·n`, cosine `4·rows·dim`,
grouped `2·groups·m·k·n`, turboquant `2·dim`) and times it via `time_op`:

1. Validate `iters != 0` and `flops_per_iter > 0 && finite`.
2. One **untimed warmup** call.
3. `iters` timed iterations using `Instant::now()`; reject non-positive/non-finite times.
4. `summarize` into `BenchResult { gflops, elapsed_ms, cv_pct }` where `elapsed_ms` is
   the **summed** time, gflops is aggregate throughput, and `cv_pct` is the population
   coefficient of variation. A `cargo:warning` fires if `cv_pct > CV_WARN_PCT = 20.0`.

Inputs use fixed ChaCha8 seeds (deterministic). CUDA bench paths are `cfg(cuda)`-gated
and call `cuda_required` otherwise. `BenchCudaContext` aliases `CudaContext` with cuda,
else is an uninhabited enum.

### 6.4 Explorer (explore/exploit)

`src/autotune/explorer.rs`. `ExplorerPolicy` is `EpsilonGreedy` | `Thompson`. The
`Explorer` keeps index-aligned `candidate_stats` (BenchResults) and
`candidate_configs` per key, plus `last_promotion_ts`.

- **Epsilon-greedy** (`next_epsilon_greedy`): with probability `EPSILON = 0.1`
  explore (uniform random candidate), else exploit the incumbent → **10% explore /
  90% exploit**.
- **Thompson** (`next_thompson`): per candidate compute `(wins, losses)` where a
  recorded result is a win iff its gflops ≥ the key-wide mean gflops; draw
  `Beta(wins+1, losses+1)` via a Gamma-ratio sampler; pick the argmax sample.
- `record_trial(key, config, result)` appends to both parallel arrays.

### 6.5 Promotion / A/B

`src/autotune/promotion.rs` + parts of `explorer.rs`.

| Constant | Value | File |
|---|---|---|
| `EPSILON` | `0.1` | `explorer.rs` |
| `MIN_PROMOTE_MARGIN` | `0.02` (challenger must beat incumbent by >2% mean gflops) | `explorer.rs` |
| `MIN_PROMOTE_TRIALS` | `3` (min trials per config before promotion) | `explorer.rs` |
| `CV_WARN_PCT` | `20.0` | `microbench.rs` |
| `CLOCK_MS_TO_NS` | `1_000_000` | `promotion.rs` |

- `should_promote` (`explorer.rs`): both challenger and incumbent need ≥
  `MIN_PROMOTE_TRIALS` results, and `challenger_mean > incumbent_mean·(1 +
  MIN_PROMOTE_MARGIN)`.
- `promote_if_winner`: if it should promote, `cache.insert(key, challenger)`, record
  `last_promotion_ts`, return the demoted incumbent (in-memory only; caller must
  `persist()`).
- `should_use_challenger(AbHook { rate }, rng)`: serves the challenger with
  probability `rate` (≥1.0 always, ≤0.0/non-finite never) — the A/B traffic split.
- `log_promotion`: appends a one-line **JSONL** `PromotionEvent { key, old_config,
  new_config, timestamp_ns, action: Promoted|RolledBack }` with fsync (append-only
  audit; separate file from the cache).
- `rollback_promotion`: finds the last `Promoted` event for the key, restores
  `old_config` via `cache.rollback`, and logs a `RolledBack` event (timestamp in ns
  via `CLOCK_MS_TO_NS`).

The cache (pretty JSON, atomic) and the promotion log (append-only JSONL) are two
separate on-disk artifacts.

---

## 7. Constants reference

| Constant | Value | File | Role |
|---|---|---|---|
| `CUDA_EXACT_TOPK_MAX_K` | 1024 | backend.rs | exact CUDA topk cap on k |
| `TILE_M` (CPU GEMM) | 64 | cpu/gemm.rs | row tile |
| `TILE_K` (CPU GEMM) | 64 | cpu/gemm.rs | depth tile |
| `BLOCK_THREADS` (distance) | 256 | cuda/distance.rs | distance/normalize block dim |
| `TOPK_BLOCK` | 1024 | cuda/topk.rs + topk.cu | topk block dim = exactness bound |
| `MXFP4_THREADS` | 128 | cuda/gemm/mxfp4_path.rs | mxfp4 gemm block dim |
| `CUDA_ARCH` | `sm_120` | build.rs | nvcc target arch |
| `MIN_FREE_VRAM_MIB` | 4096 | cuda/context.rs | min free VRAM floor |
| `PARITY_TOL` / `PARITY_ABS_TOL` | 1e-3 / 1e-6 | tests/cuda_parity_support.rs | CPU/GPU parity tolerances |
| `CURRENT_SEED_VERSION` | 1 | quant/rotation.rs | rotation seed version |
| `BITS3P5_CODE_BITS` / `BITS3P5_LEVELS` | 7 / 128 | quant/turboquant.rs | Bits3p5 packing |
| `BITS2P5_LEVELS` | 5 | quant/turboquant.rs | Bits2p5 base-5 packing |
| `QJL_SECTION_TAG` | 0x01 | quant/qjl.rs | residual section marker |
| `MXFP4_BLOCK_SIZE` / `MXFP4_PACKED_BYTES` | 32 / 16 | cuda/mxfp4.rs | MXFP4 block |
| `MXFP4_EXP_BIAS` / `MXFP4_ZERO_CODE` / `MXFP4_NAN_CODE` | 127 / 7 / 15 | cuda/mxfp4.rs | MXFP4 codes |
| `MXFP8_BLOCK_SIZE` / `MXFP8_BLOCK_BYTES` | 32 / 33 | cuda/mxfp8.rs | MXFP8 block; E4M3, bias 7 |
| `MIN_RETAINED_FRACTION` / `MIN_COSINE` / `MAX_FAR_DELTA` | 0.95 / 0.99 / 0.01 | quant/mxfp4_codec.rs | Assay FP4 gate |
| `ABSENT_SENTINEL` | `f32::NAN` | cuda/grouped_gemm.rs | absent-slot guard |
| `EPSILON` / `MIN_PROMOTE_MARGIN` / `MIN_PROMOTE_TRIALS` | 0.1 / 0.02 / 3 | autotune/explorer.rs | explore + promotion gates |
| `CV_WARN_PCT` | 20.0 | autotune/microbench.rs | noisy-bench warning |

---

## 8. Public API surface (selected re-exports from `src/lib.rs`)

- Backends: `Backend`, `BackendKind`, `CpuBackend`, `CudaBackend` (cuda),
  `DeviceInfo`, `BestConfig`, `init_cuda`, `query_device_info`.
- Quantizers: `Quantizer`, `QuantLevel`, `QuantizedVec`, `TurboQuantCodec`,
  `BinaryCodec`, `MxFp4Codec`, `RotationSeed`, `new_seed`, `apply_rotation`,
  `dot_estimate_unbiased`, `hamming_dot_estimate`, `binary_prefilter`.
- MX formats: `encode_mxfp4_block`/`decode_mxfp4_block`, `e8m0_scale`,
  `encode_mxfp8_block`/`decode_mxfp8_block`, and block/byte-size constants.
- Grouped/ragged GEMM (cuda): `GemmProblem`, `GroupedGemmPlan`,
  `GroupedGemmExecutionMode`, `RaggedBatch`, `build_grouped_gemm_plan`,
  `execute_grouped_gemm`(`_strict`), `build_ragged_batch`(`_from_slabs`),
  `extract_ragged_results`.
- Autotune: `AutotuneCache`, `AutotuneKey`, `autotune`, `microbench`,
  `Explorer`, `ExplorerPolicy`, `next_candidate`, `record_trial`, `should_promote`,
  `promote_if_winner`, `rollback_promotion`, `log_promotion`, `AbHook`,
  `EPSILON`, `MIN_PROMOTE_MARGIN`, `MIN_PROMOTE_TRIALS`.
- VRAM management (`src/vram/`): `AdmissionController`, `VramBudgeter`, `OomGuard`,
  `GpuBlockRegistry`, etc. (VRAM budgeting/admission is a sibling subsystem; not
  expanded here — "Not determined from source" beyond the re-export list).
- Errors: `ForgeError`, `Result`.
