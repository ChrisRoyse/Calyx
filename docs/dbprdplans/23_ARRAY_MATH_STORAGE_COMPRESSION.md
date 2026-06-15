# 23 — Array Math, Native Array Storage & Compression

Implements A13/A22 + new **A25 (maximal measured compression)**. How Calyx does vector math, stores a constellation as one self-organizing array bundle regardless of lens count, and compresses as hard as possible **without losing intelligence — measured, not hoped.**

## 1. The constellation is a native array object (everything grouped, auto-organized)

A Teleological Constellation is physically **one co-located array bundle**: all lens outputs for a piece of data, stored *together* with every derived array and label, calculated and organized as a unit — structure invariant as lenses are added/removed (just one more block).

```
ConstellationBundle (one record group, co-located on disk + in a GPU batch)
 ├─ slots[]        : ragged array of N lens vectors  [v_1(d_1) … v_N(d_N)]   (dense f32/quant | sparse | multi)
 ├─ scalars[]      : fixed array of typed numeric features (depth, churn, coverage Δ, …)
 ├─ anchors[]      : grounded outcome labels (the only "reality" array)
 ├─ cross_terms[]  : derived associations-between-associations (agreement scalars eager; rest lazy)
 ├─ bits[]         : per-slot signal (Assay) — value of each lens for this data's outcomes
 ├─ guard[]        : per-slot Gτ cosine readings (Ward)
 ├─ provenance     : Ledger ref (one hash-chained entry per bundle)
 └─ header         : modality, flags, panel_version, cx_id
```

**Auto-organization regardless of N (the founder's requirement):** the bundle is **structure-of-arrays keyed by `SlotId`**. Adding a lens appends one block (`slot_{N+1}`, its quant codebook-free params, bits, guard); removing one tombstones a block. Nothing else is rewritten (A5). The math (below) iterates present blocks, working identically for N=1 or N=200,000 — **embeddings, metadata, labels, associations, and calculations stored together and recomputed/organized automatically no matter how many embedders exist.**

On disk this bundle maps to the Aster per-slot column families + scalar/anchor/xterm/ledger CFs **sharing one `CxId` key** (`04 §4`): a constellation's whole array bundle is one keyspace range — one range scan to read, one group-commit to write, one GPU batch.

## 2. Memory layout: Struct-of-Arrays, blocked, SIMD/coalesced

| Choice | Layout | Why |
|---|---|---|
| Across constellations, per slot | **Struct-of-Arrays**: each slot is a contiguous column of `[cx0_v, cx1_v, …]` (Arrow chunk) | vectorized scan straight from mmap; GPU-coalesced loads; one slot's column is one ANN/quant unit |
| Within a constellation | blocked ragged array indexed by `SlotId` | O(1) block access; add/remove a lens = add/remove a block |
| Sparse slots | CSR (indices+values) blocks | SPLADE/keyword lenses |
| Multi (ColBERT) | token-major blocks | MaxSim |
| Quantized | block-scaled codes + per-block scale (MXFP / TurboQuant) | tensor-core ready |

SoA is the core decision: distance/normalize/MI over a slot are a **single strided pass over a contiguous column**; projection/scoring across the panel is a **grouped GEMM over the per-slot blocks** (§3). Row-major (AoS) would scatter these; Calyx keeps the row *logically* (the bundle) but stores it column-physically (HTAP-style, `04`).

## 3. How all the vector math is done (the array-math catalog)

Every Calyx operation reduces to array math, batched on Blackwell sm_120 tensor cores (Forge, `13`), with a bit-parity CPU SIMD path. Dominant patterns:

Implementation honesty (#338): Stage 2 Forge ships the `Backend` contract for
GEMM/grouped GEMM, cosine/dot/L2, normalize, top-k, and device info, plus
quantization modules. Catalog rows for KSG k-NN, histograms/NMI, sparse ops,
bilinear cross-terms, graph kernels, and ColBERT MaxSim are the target array
math design, not current `Backend` trait methods.

| Op | Math | Kernel strategy |
|---|---|---|
| **Lens projection** (embed) | `X · Wᵀ` per lens, different dims per lens | **Grouped GEMM** (cuBLAS 12.5 / CUTLASS): N differently-sized matmuls in **one launch** — the panel projects in one kernel regardless of N |
| **Microbatch ingest** | many constellations × N lenses | grouped GEMM over (microbatch × slot) problem list; one dispatch |
| **Agreement / Gτ / dot / cosine** | `cos(v_a, v_b)`, `q·D` | fused normalize+GEMM over candidate blocks; **TurboQuant unbiased inner product** on quantized codes (no dequant) |
| **RRF scoring** | rank fusion across slots | batched topk + reduction |
| **Cross-term interaction** | `v_aᵀ W v_b` (low-rank bilinear), `v_a⊙v_b` | small batched GEMM / elementwise |
| **MI (KSG)** | k-NN distances | reuse ANN graph; batched neighbor-distance GEMM |
| **Redundancy NMI** | histograms | block-partitioned GPU histogram |
| **ColBERT MaxSim** | `Σ_q max_d q·d` | token-block GEMM + segmented max |
| **Kernel graph** | SCC, betweenness, LP for MFVS | sparse graph kernels (CPU + GPU-assisted LP) |
| **Quantize/Dequant** | rotate, scalar-quant, QJL | fused; data-oblivious (§4) |
| **Reductions / normalize / softmax** | column reductions | warp/block reductions |

**Grouped GEMM is the linchpin.** A panel of N lenses with dims `d_1…d_N` is a *group of N matmuls of different sizes* — precisely what `cublasGemmGroupedBatchedEx` / CUTLASS grouped GEMM execute in one launch (a generalization of batched GEMM allowing variable shapes). So "project/score across however many embedders" is **one optimized kernel**; cost scales with total work, not launch overhead × N. cuBLAS's heuristics recommender (trained on real timing data) picks the kernel; Anneal caches the winner per `(shape-group, dtype, device)` (`12`).

## 4. Compression: maximal, measured, intelligence-preserving (A25)

The founder's requirement: **as much compression as possible while not losing intelligence.** Calyx can do this because it can *measure* whether intelligence survived (Assay). Three layers:

### 4.1 Storage compression — TurboQuant (default), data-oblivious, near-optimal
Calyx's default slot-vector quantizer is **TurboQuant** (Google, ICLR 2026):
- **Random rotation** → coordinates become a concentrated **Beta distribution**, near-independent in high-d → **optimal per-coordinate scalar quantizer**. Result: **near-optimal MSE distortion within ~2.7× of the information-theoretic floor**, at *every* bit-width.
- **Unbiased inner product:** MSE-optimal quant is biased for dot products, so TurboQuant adds a **1-bit QJL (Quantized Johnson–Lindenstrauss)** transform on the residual → an **unbiased, low-distortion inner-product estimator.** Exactly what Calyx needs: cosine/dot is the core of agreement, `Gτ`, RRF — so heavy compression does **not** bias the intelligence operations.
- **Data-oblivious & online → ~zero indexing time.** No codebook training, no dataset-specific tuning. Decisive for Calyx: a **hot-swapped lens is immediately quantized and searchable** (Doctrine §5), streaming ingest quantizes on the fly. Also out-recalls classic PQ in NN search per Google's evaluation.
- **Operating points:** quality-neutral ≈ 3.5 bits/channel; marginal degradation ≈ 2.5 bits/channel (Google's KV-cache result) — Calyx targets the most aggressive bit-width that still passes the intelligence contract (§4.4). Companions **QJL** (binary inner-product prefilter) and **PolarQuant** available where their structure fits.

Fallbacks/companions per slot: binary (1-bit) recall **prefilter** funnel, PQ where a trained codebook genuinely wins, raw-f32 cold sidecar for exact rescore (`04`). The slot's quantizer is itself Anneal-tuned.

### 4.2 Compute compression — Blackwell microscaling (MXFP4/MXFP8)
On the RTX 5090 (sm_120), Forge runs matmul/distance in **block-scaled microscaling formats**: **MXFP4/NVFP4** (32-element blocks, E8M0 power-of-2 scales, fused scaling in GEMM) for ~4× FP16 throughput and 2× memory, **MXFP8** where FP4 is too lossy, **bf16/fp32 accumulate**. Microscaling pairs naturally with TurboQuant's per-block scales. Quant level per op chosen by the same measured contract (§4.4).

### 4.3 Semantic compression — the kernel (the ultimate compressor)
The deepest compression is not numeric — it's the **grounding kernel** (`08`): store the ≈1% that grounds the corpus, regenerate/answer the ≈99% by association. Plus **meaning compression** (`06`): derive `N + C(N,2) + 1` grounded signals per input instead of storing redundant copies, keep cross-terms **lazy** (compute from two stored vectors on demand). Together: the paper's 99% meaning-compression claim as storage policy.

> **Compression is a facet of the intelligence objective (A32, `27 §9`), not a competing goal.** An intelligent representation is already compact; Calyx **never deletes data** to compress — it compresses the *representation* (TurboQuant + MXFP4 + the kernel) to the most aggressive level that *measurably preserves intelligence* (§4.4), and the freed footprint/compute is spent growing `J`. Compressing past where bits degrade would *lower* `J`, so the objective forbids lossy-of-intelligence compression by construction.

> **A25 forbids deleting-*to-compress*, never lawful/user deletion (A33, `30 §4`).** "Without losing anything" means: never drop data to save space — compress the *representation* instead. It does **not** forbid right-to-erasure, retention, or user-deletion, which are first-class crypto-shredding operations (`30`). No agent may refuse a lawful delete citing A25.

### 4.4 The intelligence-preservation contract (why Calyx can compress harder than anyone)
A quantization/precision level for a slot is **accepted only if measured intelligence survives** — Assay computes before/after:

```
accept_quant(slot, level) iff
  cosine_error(level)         ≤ ε_cos        // inner-product distortion bound (TurboQuant gives this provably)
  Δ bits_about(slot, anchor)  ≤ δ_bits        // lens still earns ≥0.05 bits about outcomes (07)
  Δ guard FAR/FRR             ≈ 0             // Gτ decisions unchanged (09)
  kernel-only recall          unregressed     // the kernel still explains the corpus (08)
```

Anneal **searches downward** for the most aggressive level that still passes, per slot, per workload, A/B'd on live traffic, reversible, and ultimately Ledger-logged (`12`). PH16's current promotion surface is an append-only local JSONL audit stub, not a real Ledger chain entry; real Ledger-backed compression promotion belongs to later cross-engine Anneal/provenance wiring. Calyx is not guessing a bit-width — it **measures the floor where intelligence starts to degrade and sits just above it.** The literal implementation of "max compression without losing intelligence," possible only because Calyx has Assay's bits and Ward's FAR as ground truth.

### 4.5 The compression report
`compression_report(vault)` exposes the honest numbers: bits/channel per slot, achieved distortion vs the TurboQuant lower bound, storage bytes saved, kernel compression ratio + kernel-only recall, meaning-compression yield, and the measured intelligence delta (bits/cosine/FAR) at the chosen level — compression auditable, never silent.

## 5. Why this is uniquely Calyx

- **Array-native + co-located:** one constellation = one self-organizing array bundle (embeddings + metadata + labels + associations + calculations together), batched as one GPU unit, structurally invariant to N.
- **Grouped GEMM:** the multi-lens panel is one optimized variable-shape kernel — math done right no matter how many embedders.
- **TurboQuant + microscaling:** near-information-theoretic-optimal storage compression with **unbiased inner products** and zero indexing time, on Blackwell FP4 tensor cores.
- **Measured intelligence preservation:** the only system that compresses to the *measured* floor of preserved bits, because it has the bits.

**One sentence:** Calyx stores each constellation as one co-located, self-organizing array bundle, does all vector math as variable-shape grouped GEMMs on Blackwell tensor cores, and compresses as hard as the information-theoretic floor allows — via TurboQuant's data-oblivious unbiased-inner-product quantization and MXFP4 microscaling — while Assay and Ward *measure* that no intelligence was lost, making "maximum compression without losing intelligence" a guarantee, not a hope.

Sources (engineering): [TurboQuant (Google Research)](https://research.google/blog/turboquant-redefining-ai-efficiency-with-extreme-compression/) · [TurboQuant paper (arXiv 2504.19874)](https://arxiv.org/html/2504.19874v1) · [cuBLAS Grouped GEMM](https://developer.nvidia.com/blog/introducing-grouped-gemm-apis-in-cublas-and-more-performance-updates/) · [CUTLASS grouped GEMM](https://github.com/NVIDIA/cutlass/blob/main/examples/24_gemm_grouped/gemm_grouped.cu) · Blackwell MXFP4/NVFP4 microscaling tensor cores.
