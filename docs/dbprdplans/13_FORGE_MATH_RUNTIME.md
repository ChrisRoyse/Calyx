# 13 — Forge: the Baked-In Math Runtime

Implements A13/A25. The user's requirement: *"optimally built in Rust for all math computation; built-in full matrix multiplication etc.; everything baked into the database's capabilities."* Forge is Calyx's owned linear-algebra layer — no external BLAS service on the hot path; a CUDA(sm_120) path and a SIMD CPU path that are **bit-parity tested**.

> **The deep array-math, native-array-storage, and compression design lives in `23_ARRAY_MATH_STORAGE_COMPRESSION.md`** — the constellation as one co-located array bundle (invariant to N), all panel math as **grouped GEMM**, and **TurboQuant + MXFP4 microscaling** compression gated by *measured* intelligence (A25). This doc is the runtime/backend; `23` is the math/storage/compression model. Read them together.

## 1. Why the DB owns its math

Every Calyx operation is linear algebra over slot vectors: embedding projection, distance, RRF scoring, cross-term interaction, MI k-NN, kernel graph ops, quantization, guard cosine. Pushing these to an external library/service adds latency, a dependency, and a place for silent failure. Forge bakes them in so the database is self-contained (A13/A18) and Anneal can autotune them (`12`).

## 2. Backends

| Backend | Target | Built on |
|---|---|---|
| **CUDA / sm_120** | aiwonder RTX 5090 (Blackwell GB202, 32 GB), driver 595.71, CUDA 13.2 | `cudarc` + CubeCL-style autotuned kernels; cuBLAS/cuBLASLt for big matmul; custom fused kernels for distance/MI/quantize |
| **CPU SIMD** | embedded vaults (laptops), aiwonder fallback | `wide`/`std::simd` AVX-512/AVX2/NEON; `faer`/`gemm` Rust matmul |
| **ONNX/candle** | running lens NNs locally | `candle` + ORT CUDA EP |

Backend selection is per-op, per-shape, autotuned by Anneal and cached (`12 §4`). **Bit-parity contract (A13):** CPU and GPU paths must agree within a declared numerical tolerance on a golden set (run on aiwonder; no CI pipeline) — embedded vault and server must compute the same constellation.

## 3. Operations Forge provides

| Op | Use | Kernel notes |
|---|---|---|
| `gemm` / **grouped GEMM** | lens projection across N variable-dim lenses, scoring | **one launch for the whole panel regardless of N** (cuBLAS 12.5 `GemmGroupedBatchedEx` / CUTLASS grouped); MXFP4/MXFP8 microscaling on Blackwell tensor cores; cuBLASLt for large (`23 §3`) |
| `normalize` (L2) | every dense slot | fused with write |
| `cosine` / `dot` / `l2` distance | ANN, agreement, guard | fused, batched over candidate blocks |
| `topk` | ANN rerank, kernel funnel | GPU bitonic / CPU heap |
| `quantize` / `dequantize` (**TurboQuant** default, QJL, MXFP4, binary, PQ) | storage, prefilter, compute | **TurboQuant**: data-oblivious rotate→scalar-quant + 1-bit QJL residual = unbiased inner product, ~zero indexing (`23 §4`); MXFP4 block-scale for compute |
| `knn` (for KSG MI) | Assay bits | reuse ANN graph; batched neighbor distances |
| `histogram` / `nmi` | streaming redundancy | partitioned, GPU |
| `spmm` / sparse ops | SPLADE/keyword lenses, inverted scoring | CSR on GPU/CPU |
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
- All distance/MI/quant kernels validated against a CPU reference on a golden corpus (run on aiwonder) (A13).
- NaN/Inf guards on every kernel boundary → `CALYX_FORGE_NUMERICAL_INVARIANT` fail-closed (A16).

## 7. Forge API (internal; summary)

```
gemm(a, b, opts) -> c
batched_cosine(query, candidates_block) -> scores
topk(scores, k) -> (idx, val)
quantize(slot_sample) -> Codebook ; encode/decode(vec, codebook)
knn(graph, points, k) -> neighbor dists           // for Assay
maxsim(query_tokens, doc_tokens) -> score
autotune(op, shape, dtype, device) -> BestConfig  // cached; driven by Anneal
device_budget() -> {vram_free, sm_free}
```

**One sentence:** Forge is the database's own GPU/SIMD math engine — matmul, distance, quantization, MI, and graph kernels baked in, autotuned per shape, bit-parity across CPU and the RTX 5090, so Calyx needs no external math service and Anneal makes every kernel faster for the job it's actually doing.

Sources: [candle (Rust ML, CUDA kernels)](https://github.com/huggingface/candle) · [Burn/CubeCL matmul vs cuBLAS](https://www.phoronix.com/news/Burn-MATMUL-Kernels-CUDA) · [cuda-oxide Rust→PTX](https://www.marktechpost.com/2026/05/09/nvidia-ai-just-released-cuda-oxide-an-experimental-rust-to-cuda-compiler-backend-that-compiles-simt-gpu-kernels-directly-to-ptx/).
