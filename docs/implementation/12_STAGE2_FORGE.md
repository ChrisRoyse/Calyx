# Stage 2 — Forge Math Runtime (PH12–PH16)

> **STATUS: ✅ DONE (FSV-signed-off; latest repo head tracked in #23).** All of PH12–PH16 are
> implemented and committed in `calyx-forge` (~9.1k LOC): CPU SIMD backend,
> CUDA sm_120 backend with a CPU↔GPU bit-parity suite, TurboQuant, MXFP4/MXFP8
> microscaling + grouped/ragged GEMM, and the per-shape autotune cache. Stage 2
> FSV evidence is recorded in the closed PH12-PH16 issues and context #23.
> Build/test natively on aiwonder (CUDA 13.3, RTX 5090 sm_120) — no cross-build.
> Downstream Stage 5 consumes Forge where applicable. Stage 4 currently uses
> Sextant-owned CPU/index paths for HNSW, quantization, and fan-out; #299 records
> that no Forge GPU HNSW or grouped fan-out path is wired yet. Next active stage
> is Lodestar (`16_STAGE6_LODESTAR.md`).
> Post-sweep hardening #306 routes `CudaBackend::normalize` through the
> `normalize_rows_f32` CUDA kernel instead of CPU delegation; CUDA claims remain
> tied to aiwonder `--features cuda` gates.
> Post-sweep hardening #307 makes GEMM parity read back both relative and
> absolute worst cases; near-zero cancellation cells may pass by a named
> `<=1e-6` absolute floor while ordinary values still use `<=1e-3` relative.
> Post-sweep hardening #316 adds a readback-visible grouped GEMM execution mode:
> ordinary execution reports `grouped_batched`, `sequential_fallback`, or
> `no_active_problems`; strict execution fails loud instead of accepting a
> fallback when the one-launch contract is required.
> Pre-Lodestar hardening #333 makes absent-slot sentinels a release-mode
> fail-closed invariant: if a skipped grouped-GEMM slot is written, the runtime
> returns `CALYX_FORGE_NUMERICAL_INVARIANT` instead of relying on debug-only
> assertions. Evidence root:
> `/home/croyse/calyx/data/fsv-issue333-stage1-5-hardening-20260608`.
> Contract-honesty hardening #338 makes the Stage 2 backend surface explicit:
> source constants list shipped `Backend` ops versus deferred PRD catalog ops,
> `CUDA_EXACT_TOPK_MAX_K = 1024` is the public exact CUDA top-k ceiling, and PH16
> promotion provenance is documented as a local JSONL audit stub rather than a
> real Ledger chain entry.

Calyx's owned linear-algebra layer: a CPU SIMD path and a CUDA sm_120 path that
are **bit-parity tested**, plus TurboQuant, MXFP4 microscaling, grouped GEMM,
and per-shape autotuning. No external BLAS service on the hot path (A13).
Builds **natively** on aiwonder against CUDA 13.3 for the RTX 5090 (sm_120) —
no cross-build needed (corrects the PRD `13 §4` note; see `01 §3`). Lands in
`calyx-forge`. Deep array/compression model: PRD `23`.

---

## PH12 — CPU SIMD backend
- **Status.** DONE via issues #71-#76; FSV roots are recorded in
  `PH12-cpu-simd-backend/README.md`.
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
- **Status.** ✅ FSV-signed-off (`cuda/` backend + `.cu` kernels + parity suite,
  commits `6b3c2d3`…`dd27885`; aggregate evidence in #23).
- **Post-sweep note.** #306 adds the real `normalize_rows_f32` device kernel to
  the distance PTX artifact and routes `CudaBackend::normalize` through it.
- **Objective.** GPU kernels (cudarc/CubeCL + cuBLAS for big matmul) targeting
  sm_120; **bit-parity** with the CPU path on a golden set.
- **Deps.** PH12.
- **Deliverables.** `cuda/` kernels (gemm via cudarc/cuBLAS, fused cosine/topk),
  ptx+cubin for sm_120 with JIT fallback, determinism mode (fixed reductions).
- **Key tasks.** build against `/usr/local/cuda-13.3`; sm_120 codegen; pin
  reductions; `CALYX_FORGE_DEVICE_UNAVAILABLE` on CUDA init fail (no silent CPU
  fallback in server mode).
- **FSV gate.** CPU↔GPU **≤1e-3 rel** on the golden set; matmul within **10% of
  cuBLAS** on sm_120 (read the timing + the parity diff on aiwonder's GPU).
- **Axioms/PRD.** A13, `13 §2/§4/§6`, `19 §4`.

## PH14 — TurboQuant (rotate + scalar + QJL)
- **Status.** ✅ FSV-signed-off (`quant/turboquant.rs`, `rotation.rs`, `qjl.rs`,
  `binary.rs`; seed-replay + operating-point FSV tests in-tree, commits
  `b9c7267`…`4db91c2`; aggregate evidence in #23).
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
- **Status.** ✅ FSV-signed-off (`quant/mxfp4_codec.rs`, `cuda/mxfp4`/`mxfp8`,
  `cuda/grouped_gemm.rs` + `ragged_gemm.rs`; N-invariance FSV tests + MXFP8
  fallback, commits `13423a9`…`8933925`; #316 execution-mode readback and
  strict grouped launch hardening; aggregate evidence in #23).
- **Objective.** Blackwell block-scaled compute (MXFP4/NVFP4, MXFP8 fallback,
  fp32 accumulate) and **grouped GEMM** so an N-lens panel projects/scores in
  one launch regardless of N.
- **Deps.** PH14.
- **Deliverables.** in-tree grouped/ragged GEMM wrapper, MXFP4 fp32-accumulate
  GEMM path, ragged-bundle handling (absent slots skipped, never zero-filled).
- **Key tasks.** variable-shape problem list per (microbatch×slot); FP4 only
  where Assay later proves quant-safe; mixed-completeness batches correct.
- **Post-sweep note.** #333 validates absent-slot sentinel ranges and values
  after device execution in release builds, returning a structured Forge error
  if a skipped slot is out of range or was overwritten; aiwonder release CUDA
  readback evidence lives under
  `/home/croyse/calyx/data/fsv-issue333-stage1-5-hardening-20260608/forge-absent`.
- **FSV gate.** grouped GEMM result == per-matmul loop (read), and is **invariant
  to N**; execution mode is read from `GroupedGemmPlan` and strict grouped launch
  fails loud if cuBLAS grouped mode is unsupported; skipped slots keep their
  sentinel bytes in release builds; FP4 within bound on safe
  slots; partial-bundle batch → correct per-constellation result.
- **Axioms/PRD.** `23 §3/§4.2`, A25, `17 §7.4`.

## PH16 — Autotune config cache
- **Status.** ✅ FSV-signed-off (`autotune/` cache + microbench + explorer +
  reversible promotion; two-shape convergence FSV test, commits
  `5029978`…`6eff08f`; aggregate evidence in #23).
- **Objective.** Per-shape best-config cache `(op,shape,dtype,device,recall_tgt)`
  → params, refreshed by a low-rate explorer; the seam Anneal later drives.
- **Deps.** PH15.
- **Deliverables.** `autotune.rs` (microbench, cache, ε-greedy/Thompson
  explorer, A/B-on-live hook), persisted cache.
- **Key tasks.** measure on real shapes; promote only on measured win; expose
  `autotune(op,shape,dtype,device)->BestConfig`.
- **FSV gate.** the same op on two shapes converges to two cached configs
  (read the cache); a promotion is local-JSONL logged + reversible.
- **Axioms/PRD.** A14, `12 §4`, `13 §7`.

---

## Stage 2 exit — ✅ achieved
Forge does matmul/distance/quant/topk on both CPU and the RTX 5090 with proven
bit-parity for the byte-readback golden set; CUDA top-k is exact for
`k <= CUDA_EXACT_TOPK_MAX_K` (`1024`) and fails loud above that until multi-pass
exact merge work lands (#303). The current `Backend` trait ships
`gemm`/`cosine`/`dot`/`l2`/`normalize`/`topk`/`device_info`; PRD catalog ops such
as KSG k-NN, histograms/NMI, sparse ops, bilinear cross-terms, graph kernels, and
ColBERT MaxSim remain explicit deferred work after #338. Their owners are the
later phase cards that consume those kernels: PH27/PH28 for Assay/Loom math,
PH31/PH32/PH52 for graph and spectral math, and PH68/PH70 for scale/index
validation. PH16 promotion provenance is an append-only local JSONL audit stub;
real Ledger integration is PH43 T05/T06 plus PH46 T05/T06, not a hidden Stage 2
claim.
TurboQuant gives unbiased inner products, grouped GEMM makes panel math
N-invariant with readback-visible launch mode, and configs autotune per shape — PRD `MATH`/`ARRAYMATH`/
`COMPRESS` foundations. Implemented and FSV-signed-off; downstream Stage 5
readbacks on aiwonder depend on these kernels and remain green. Stage 4 uses
Sextant-owned CPU/index paths for HNSW/quant/fan-out until a future Forge
integration is wired by PH46/PH68 and validated by PH70; current repo head is
recorded in context issue #23.
