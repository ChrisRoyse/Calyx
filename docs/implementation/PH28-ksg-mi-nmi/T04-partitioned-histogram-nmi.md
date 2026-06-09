# PH28 · T04 — Partitioned histogram NMI (streaming)

| Field | Value |
|---|---|
| **Phase** | PH28 — KSG MI + partitioned NMI |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/nmi.rs` (≤500) |
| **Depends on** | T01 (MiEstimate type) · PH27 T04 (agreement_graph, pairwise scalar inputs) |
| **Axioms** | A2, A16 |
| **PRD** | `dbprdplans/07 §2` |

## Goal

Implement `partitioned_histogram_nmi_v1` — the streaming partitioned-histogram
NMI estimator used for pairwise redundancy scoring on the agreement graph
(absorbed from ContextGraph `pairwise_mi` `partitioned_histogram_nmi_v1`). Fast,
streaming (no full dataset in memory), used for large-n agreement-graph
redundancy queries. Returns `NmiEstimate { nmi: f32, n_samples: usize, trust:
Trusted | Provisional }`. Fails closed below quorum n≥50.

Post-sweep #317: the implemented `partitioned_histogram_nmi` guard rejects
mismatched, empty, n<50, and NaN/Inf scalar streams before binning, and accepts
n=50 exactly. Stage 5 readback records the NMI edge error codes.

## Build (checklist of concrete, code-level steps)

- [x] Define `NmiEstimate`: `{ nmi: f32, n_samples: usize, estimator: EstimatorKind::PartitionedHistogramNmi, trust: Trusted | Provisional }`
- [x] Implement `partitioned_histogram_nmi_v1(x_scalars: impl Iterator<Item=f32>, y_scalars: impl Iterator<Item=f32>, n_bins: usize, n_samples_hint: usize) -> Result<NmiEstimate, CalyxError>`:
  - stream both iterators simultaneously; accumulate a `n_bins × n_bins` joint histogram `H[i][j]` (increment the bin for each `(x, y)` pair); track marginal counts `H_x[i]`, `H_y[j]`
  - once streaming is complete: compute `MI = Σ H[i][j]/N · log(H[i][j]·N / (H_x[i]·H_y[j]))` with Laplace smoothing (add 0.5 to each bin to avoid log(0))
  - compute `NMI = MI / sqrt(H(X) · H(Y))` (symmetric NMI, geometric mean normalization)
  - if `n_samples < 50` → `Err(CALYX_ASSAY_INSUFFICIENT_SAMPLES)`
  - `n_bins` default = `max(5, floor(sqrt(n/5)))` (Scott's rule adaptation for NMI); configurable
- [x] Implement `auto_n_bins(n: usize) -> usize`: `max(5, (n as f64 / 5.0).sqrt().floor() as usize)`
- [x] Pair-redundancy fast path `pair_redundancy_nmi(slot_a: SlotId, slot_b: SlotId, vault, clock) -> Result<NmiEstimate, CalyxError>`: reads the Agreement scalar stream from the xterm CF (fast: scalars not full vectors), calls `partitioned_histogram_nmi_v1` on the scalar streams

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: stream two identical `[0.0, 0.1, 0.2, …, 1.0]` sequences (n=100) → NMI = 1.0 ± 0.05
- [x] unit: two independent uniform random streams (n=500, seed=42) → NMI ≤ 0.1 (near-zero for independent variables)
- [x] unit: two streams with known NMI ≈ 0.7 (constructed from a known 2-cluster distribution, seed=42) → within 0.1 of 0.7
- [x] edge: n=30 → `CALYX_ASSAY_INSUFFICIENT_SAMPLES`; streaming iterator yields no items → `CALYX_ASSAY_INSUFFICIENT_SAMPLES`; n=50 exactly is accepted
- [x] fail-closed: unequal-length inputs and NaN/Inf scalar streams → `CALYX_ASSAY_INSUFFICIENT_SAMPLES` before bin accumulation

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** two agreement-scalar streams for a planted high-redundancy pair (both streams = cosines of near-identical vectors, NMI expected ≥ 0.8) and a planted independent pair (NMI expected ≤ 0.1)
- **Readback:**
  ```
  cargo test nmi_planted_redundant_and_independent -- --nocapture
  ```
  Prints two NMI values; high-redundancy ≥ 0.8, independent ≤ 0.1.
- **Prove:** run the test on aiwonder; capture output. Confirm both thresholds hold. This directly validates the redundancy gate in the differentiation contract (PH29 ≤0.6 NMI → admit).

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH28 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
