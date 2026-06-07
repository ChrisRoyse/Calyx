# PH28 — KSG MI + partitioned NMI

**Stage:** S5 — Loom + Assay (DDA & Bits)  ·  **Crate:** `calyx-assay`  ·
**PRD roadmap:** P4  ·  **Axioms:** A2, A16

## Objective

Implement the two production MI estimators — the KSG (Kraskov–Stögbauer–
Grassberger) k-NN mutual information estimator for continuous↔continuous and
continuous↔discrete pairs, and the partitioned histogram NMI (`partitioned_
histogram_nmi_v1`, streaming) for high-d/large-n redundancy on the agreement
graph. Both estimators carry bootstrap confidence intervals and sample count on
every output; both fail closed below quorum n≥50 (`CALYX_ASSAY_INSUFFICIENT_
SAMPLES`). A random-projection pre-step controls k-NN bias on high-dimensional
slots. This is the first real signal measurement; it wires into Loom's
`AssayGate` trait (T03, PH27) so materialization decisions become live.

> **Honesty is load-bearing:** bits are labeled `trusted` only when computed
> against a grounded anchor (A2); bits about an ungrounded/auto-labeled target
> are tagged `provisional`. Every estimate carries sample count + CI; no
> estimate is returned without them. Fail-closed below quorum — never a noisy
> point estimate when n<50.

## Dependencies

- **Phases:** PH27 (agreement graph, active pair info, xterm CF, LRU cache),
  PH13 (Forge ANN graph via k-NN indices, GPU batched distance), PH09
  (Aster reads for slot/anchor pairs)
- **Provides for:** PH29 (differentiation contract, n_eff), PH30 (panel
  sufficiency, bits_report), PH27 T03 (live AssayGate wire-up)

## Current state (build off what exists)

`calyx-assay` is a 9-line stub; greenfield. Forge ANN k-NN indices are complete
from PH13; Aster slot/anchor CF reads are complete from PH09; the `AssayGate`
trait stub from PH27 T03 provides the hook point.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-assay/src/lib.rs` | Crate root; re-exports public API |
| `crates/calyx-assay/src/ksg.rs` | KSG estimator: k-NN MI via ANN graph, continuous↔continuous + continuous↔discrete, bias-corrected, bootstrap CI |
| `crates/calyx-assay/src/nmi.rs` | Partitioned histogram NMI (`partitioned_histogram_nmi_v1`), streaming, redundancy-graph use case |
| `crates/calyx-assay/src/projection.rs` | Random-projection pre-step for high-d: JL lemma projection to `2·ceil(log2(n))` dims; seeded deterministically |
| `crates/calyx-assay/src/bootstrap.rs` | Bootstrap CI engine: resampled MI mean ± 1.96σ; configurable n_bootstrap (default 200); seeded |
| `crates/calyx-assay/src/gate.rs` | `AssayGate` impl that wires `pair_gain` into PH27 `MaterializationPlan`; `lens_signal` entry point |
| `crates/calyx-assay/src/tests.rs` | Planted-synthetic FSV tests: known MI, known NMI, CI correctness, quorum enforcement |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | KSG estimator: k-NN MI, bias correction, continuous↔discrete | — |
| T02 | Random-projection pre-step (high-d) | T01 |
| T03 | Bootstrap CI engine | T01 |
| T04 | Partitioned histogram NMI (streaming) | — |
| T05 | Quorum guard + `CALYX_ASSAY_INSUFFICIENT_SAMPLES` | T01, T04 |
| T06 | `AssayGate` impl + `lens_signal` wire-up + planted-signal FSV | T01, T03, T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. **Planted-signal MI within CI:** create a synthetic dataset on aiwonder with
   known MI ≈ 0.5 nats (generated from a known joint Gaussian); call
   `ksg_estimate(X, Y, k=5)`; the returned `{bits, ci_low, ci_high}` must
   contain the known value:
   ```
   cargo test ksg_planted_signal_in_ci -- --nocapture
   ```
   Test prints the CI; known value must be inside it.

2. **Fails closed below quorum (n<50):** call `ksg_estimate` on a sample of
   n=30 paired vectors; must return `Err(CALYX_ASSAY_INSUFFICIENT_SAMPLES)`,
   not a noisy point estimate. Verify via:
   ```
   cargo test ksg_quorum_fail_closed -- --nocapture
   ```

3. **NMI redundancy detection:** generate two near-identical high-d vectors;
   `partitioned_histogram_nmi_v1` must return NMI ≥ 0.8; two independent random
   vectors must return NMI ≤ 0.1.

Evidence (all three terminal outputs) attached to PH28 GitHub issue.

## Risks / landmines

- **KSG k-NN bias at high-d:** without the random-projection pre-step, the
  k-NN graph distances degenerate in high-d (curse of dimensionality). Always
  project to `min(d, 2·ceil(log2(n)))` dims before KSG. Seed the projector
  deterministically from `(slot_a_id, slot_b_id, n_samples)`.
- **Discrete outcomes:** for binary anchors (Pass/Fail), use the discrete KSG
  variant with a correction term for tied k-NN distances. Do not use the
  continuous formula on discrete data.
- **Bootstrap seed:** all bootstrap resamples must be seeded from a
  `ChaCha8Rng` with a deterministic seed so tests are reproducible. Never
  `thread_rng()` in logic paths.
- **DPI honesty:** `lens_signal` returns bits tagged `trusted` only when the
  anchor is grounded (A2). If the anchor is not grounded, tag as `provisional`.
  This tagging is a correctness requirement, not cosmetic.
- **Sample count in CI output:** every `MiEstimate` struct must carry
  `n_samples: usize`. Downstream consumers (PH29, PH30) reject estimates
  without sample count.
