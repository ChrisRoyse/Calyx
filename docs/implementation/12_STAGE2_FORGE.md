# Stage 2 — Forge Math Runtime (PH12–PH16)

Calyx's owned linear-algebra layer: a CPU SIMD path and a CUDA sm_120 path that
are **bit-parity tested**, plus TurboQuant, MXFP4 microscaling, grouped GEMM,
and per-shape autotuning. No external BLAS service on the hot path (A13).
Builds **natively** on aiwonder against CUDA 13.2 for the RTX 5090 (sm_120) —
no cross-build needed (corrects the PRD `13 §4` note; see `01 §3`). Lands in
`calyx-forge`. Deep array/compression model: PRD `23`.

---

## PH12 — CPU SIMD backend
- **Objective.** Reference + production CPU path: `gemm`, `cosine`/`dot`/`l2`,
  `normalize`, `topk` using `wide`/`std::simd` (AVX-512 on the Ryzen).
- **Deps.** PH04.
- **Deliverables.** `cpu/` kernels; a trait `Backend` so GPU plugs in later;
  golden-vector fixtures (seeded) with numpy/BLAS reference outputs.
- **Key tasks.** correct, vectorized kernels; NaN/Inf guards at boundaries
  (`CALYX_FORGE_NUMERICAL_INVARIANT`); deterministic reduction order.
- **FSV gate.** outputs match the golden reference within tolerance (read
  computed-vs-golden bytes); NaN input → fails closed.
- **Axioms/PRD.** A13, A16, `13 §3`.

## PH13 — CUDA sm_120 backend + bit-parity
- **Objective.** GPU kernels (cudarc/CubeCL + cuBLASLt for big matmul) targeting
  sm_120; **bit-parity** with the CPU path on a golden set.
- **Deps.** PH12.
- **Deliverables.** `cuda/` kernels (gemm via cuBLASLt, fused cosine/topk),
  ptx+cubin for sm_120 with JIT fallback, determinism mode (fixed reductions).
- **Key tasks.** build against `/usr/local/cuda-13.2`; sm_120 codegen; pin
  reductions; `CALYX_FORGE_DEVICE_UNAVAILABLE` on CUDA init fail (no silent CPU
  fallback in server mode).
- **FSV gate.** CPU↔GPU **≤1e-3 rel** on the golden set; matmul within **10% of
  cuBLAS** on sm_120 (read the timing + the parity diff on aiwonder's GPU).
- **Axioms/PRD.** A13, `13 §2/§4/§6`, `19 §4`.

## PH14 — TurboQuant (rotate + scalar + QJL)
- **Objective.** Default slot quantizer: random rotation → per-coord scalar
  quant + 1-bit QJL residual = **unbiased inner product**, data-oblivious,
  ~zero indexing.
- **Deps.** PH13.
- **Deliverables.** `quant/turboquant.rs` (rotate, scalar-quant, QJL),
  versioned/content-addressed rotation seed (recorded for replay), encode/
  decode, unbiased dot estimator.
- **Key tasks.** seed versioning (replay-safe, `24 §7 row 11`); operating points
  (~3.5 bits quality-neutral, ~2.5 marginal); binary prefilter companion.
- **FSV gate.** unbiased inner-product within the distortion bound on random
  vectors; **re-quant with the recorded seed is bit-identical** (read bytes);
  cosine error ≤ ε.
- **Axioms/PRD.** A25, `23 §4.1`, `13 §3`.

## PH15 — MXFP4/microscaling + grouped GEMM
- **Objective.** Blackwell block-scaled compute (MXFP4/NVFP4, MXFP8 fallback,
  fp32 accumulate) and **grouped GEMM** so an N-lens panel projects/scores in
  one launch regardless of N.
- **Deps.** PH14.
- **Deliverables.** grouped-GEMM wrapper (cuBLAS `GemmGroupedBatchedEx` /
  CUTLASS grouped), MXFP4 GEMM path, ragged-bundle handling (absent slots
  skipped, never zero-filled).
- **Key tasks.** variable-shape problem list per (microbatch×slot); FP4 only
  where Assay later proves quant-safe; mixed-completeness batches correct.
- **FSV gate.** grouped GEMM result == per-matmul loop (read), and is **invariant
  to N** (one launch); FP4 within bound on safe slots; partial-bundle batch →
  correct per-constellation result.
- **Axioms/PRD.** `23 §3/§4.2`, A25, `17 §7.4`.

## PH16 — Autotune config cache
- **Objective.** Per-shape best-config cache `(op,shape,dtype,device,recall_tgt)`
  → params, refreshed by a low-rate explorer; the seam Anneal later drives.
- **Deps.** PH15.
- **Deliverables.** `autotune.rs` (microbench, cache, ε-greedy/Thompson
  explorer, A/B-on-live hook), persisted cache.
- **Key tasks.** measure on real shapes; promote only on measured win; expose
  `autotune(op,shape,dtype,device)->BestConfig`.
- **FSV gate.** the same op on two shapes converges to two cached configs
  (read the cache); a promotion is logged + reversible.
- **Axioms/PRD.** A14, `12 §4`, `13 §7`.

---

## Stage 2 exit
Forge does matmul/distance/quant/topk on both CPU and the RTX 5090 with proven
bit-parity, TurboQuant gives unbiased inner products, grouped GEMM makes panel
math N-invariant, and configs autotune per shape — PRD `MATH`/`ARRAYMATH`/
`COMPRESS` foundations.
