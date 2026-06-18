# 13 — Forge: the Baked-In Math Runtime

Implements A13/A25. The user's requirement: *"optimally built in Rust for all math computation; built-in full matrix multiplication etc.; everything baked into the database's capabilities."* Forge is Calyx's owned linear-algebra layer — no external BLAS service on the hot path; a CUDA(sm_120) path and a SIMD CPU path that are **bit-parity tested**.

> **The deep array-math, native-array-storage, and compression design lives in `23_ARRAY_MATH_STORAGE_COMPRESSION.md`** — the constellation as one co-located array bundle (invariant to N), all panel math as **grouped GEMM**, and **TurboQuant + MXFP4 microscaling** compression gated by *measured* intelligence (A25). This doc is the runtime/backend; `23` is the math/storage/compression model. Read them together.

## 1. Why the DB owns its math

Every Calyx operation is linear algebra over slot vectors: embedding projection, distance, RRF scoring, cross-term interaction, MI k-NN, kernel graph ops, quantization, guard cosine. Pushing these to an external library/service adds latency, a dependency, and a place for silent failure. Forge bakes them in so the database is self-contained (A13/A18) and Anneal can autotune them (`12`).

## 2. Backends

| Backend | Target | Built on |
|---|---|---|
| **CUDA / sm_120** | aiwonder RTX 5090 (Blackwell GB202, 32 GB), driver 595.71, CUDA 13.3 | `cudarc` + CubeCL-style autotuned kernels; cuBLAS/cuBLASLt for big matmul; shipped custom distance/normalization/top-k kernels; MI/NMI and broader PRD kernels are deferred |
| **CPU SIMD** | embedded vaults (laptops), aiwonder fallback | `wide`/`std::simd` AVX-512/AVX2/NEON; `faer`/`gemm` Rust matmul |
| **ONNX/candle** | running lens NNs locally | `candle` + ORT CUDA EP |

Backend selection is per-op, per-shape, autotuned by Anneal and cached (`12 §4`). **Bit-parity contract (A13):** CPU and GPU paths must agree within a declared numerical tolerance on a golden set (run on aiwonder; no CI pipeline) — embedded vault and server must compute the same constellation.

## 3. Operations Forge provides

Implementation honesty (#338): the Stage 2 `Backend` trait currently ships
`gemm`, `cosine`, `dot`, `l2`, `normalize`, `topk`, and `device_info`; the source
contract is `FORGE_SHIPPED_BACKEND_OPS`. PRD catalog rows beyond that are design
requirements for later engine integration and are tracked as deferred by
`FORGE_DEFERRED_BACKEND_OPS` until their owning phases wire them into Forge.
CUDA exact `topk` is public-contract bounded by `CUDA_EXACT_TOPK_MAX_K = 1024`;
larger `k` fails loud rather than returning a non-exact merge.

| Op | Use | Kernel notes |
|---|---|---|
| `gemm` / **grouped GEMM** | lens projection across N variable-dim lenses, scoring | **one launch for the whole panel regardless of N** (cuBLAS 12.5 `GemmGroupedBatchedEx` / CUTLASS grouped); MXFP4/MXFP8 microscaling on Blackwell tensor cores; cuBLASLt for large (`23 §3`) |
| `normalize` (L2) (shipped) | every dense slot | fused with write |
| `cosine` / `dot` / `l2` distance (shipped) | ANN, agreement, guard | fused, batched over candidate blocks |
| `topk` (shipped; CUDA exact for `k <= 1024`) | ANN rerank, kernel funnel | GPU bitonic / CPU heap; CUDA fails closed above `CUDA_EXACT_TOPK_MAX_K` |
| `quantize` / `dequantize` (**TurboQuant** default, QJL, MXFP4, binary, PQ) | storage, prefilter, compute | **TurboQuant**: data-oblivious rotate→scalar-quant + 1-bit QJL residual = unbiased inner product, ~zero indexing (`23 §4`); MXFP4 block-scale for compute |
| `knn` (for KSG MI; deferred #338) | Assay bits | reuse ANN graph; batched neighbor distances |
| `histogram` / `nmi` (deferred #338) | streaming redundancy | partitioned, GPU |
| `spmm` / sparse ops (deferred #338) | SPLADE/keyword lenses, inverted scoring | CSR on GPU/CPU |
| `bilinear` `v_aᵀW v_b` | cross-term interaction | small `W`, batched |
| graph ops (SCC, betweenness, FVS LP) | Lodestar kernel | `calyx-mincut`/`-paths`/`-solver` (CPU, GPU-assisted LP) |
| `colbert_maxsim` | late-interaction rerank | token-block GPU |

## 4. Blackwell-specific notes (sm_120)

- Target `sm_120` (compute_cap 12.0) explicitly; ship PTX + cubin for sm_120 with a JIT fallback. Stable PyTorch wheels lag Blackwell (aiwonder gotcha) — Forge does **not** depend on host PyTorch; lens NNs run in pinned TEI Docker or candle/ORT.
- Use FP8 (E4M3) tensor-core matmul where Assay shows quant-safe slots; bf16 default; fp32 accumulate.
- Respect the 600 W power cap and the `leapable-gpu-max-power.service`; Forge yields VRAM/SM budget to resident TEI/marketplace (Anneal-capped, `12 §6`).
- 32 GB VRAM is a working set, never the source of truth (`04 §3`): batches stream from mmap'd Aster columns.

Rust GPU is now credible: Burn's CubeCL matmul kernels match/beat cuBLAS in published benchmarks, candle ships CUDA kernels, and NVIDIA's `cuda-oxide` compiles Rust SIMT kernels to PTX — Forge stays Rust-native end-to-end while keeping cuBLASLt as the proven big-matmul path.

## 5. Memory & batching

- **Microbatching:** ingest batches all lenses of a constellation, plus a window of constellations, into single GPU dispatches (`04 §5`) — embedding dominates cost, so batch width is the main throughput lever.
- **Pinned-host + async copy** double-buffering to overlap mmap reads with compute.
- **Arena allocator** for transient working buffers; no per-op cudaMalloc on the hot path.
- **VRAM budgeter:** a soft cap (config) so Forge coexists with the 3 resident TEI containers (general/legal/reranker) on the single GPU.

## 6. Numerical correctness & determinism

- Determinism mode for FSV/repro (`11`): fixed reduction order, no atomics-nondeterminism, so a replayed answer matches bit-for-bit within tolerance.
- Shipped distance, normalization, top-k, and matmul paths are validated against
  CPU references on a golden corpus (run on aiwonder) (A13). MI/NMI, sparse,
  graph, and late-interaction kernels remain deferred until their owning phases
  wire them into Forge with their own parity evidence.
- NaN/Inf guards on every kernel boundary → `CALYX_FORGE_NUMERICAL_INVARIANT` fail-closed (A16).

## 7. Forge API (internal; summary)

```
gemm(a, b, m, k, n, out)
cosine(a, b, dim, out)
dot(a, b, dim, out)
l2(a, b, dim, out)
normalize(vecs, dim)
topk(scores, k) -> (idx, val)        // CUDA exact only for k <= 1024
device_info() -> DeviceInfo
```

Deferred API surface: `knn`, histogram/NMI, sparse ops, bilinear cross-terms,
graph kernels, and ColBERT MaxSim remain PRD requirements, not Stage 2
`Backend` methods.

**One sentence:** Forge is the database's own GPU/SIMD math engine for shipped Stage 2 matmul, distance, normalization, top-k, and grouped-GEMM foundations, with quantization storage/compression work and broader MI, sparse, graph, and late-interaction kernels tracked as explicit deferred PRD work.

Sources: [candle (Rust ML, CUDA kernels)](https://github.com/huggingface/candle) · [Burn/CubeCL matmul vs cuBLAS](https://www.phoronix.com/news/Burn-MATMUL-Kernels-CUDA) · [cuda-oxide Rust→PTX](https://www.marktechpost.com/2026/05/09/nvidia-ai-just-released-cuda-oxide-an-experimental-rust-to-cuda-compiler-backend-that-compiles-simt-gpu-kernels-directly-to-ptx/).
