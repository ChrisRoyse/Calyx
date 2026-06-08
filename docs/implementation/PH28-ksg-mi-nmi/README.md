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

## Current state

✅ **DONE / FSV-signed-off in Stage 5** (`0ada102`). `calyx-assay` now provides
the KSG-style estimator, deterministic projection, bootstrap CI, partitioned
histogram NMI, logistic probe, AssayGate lens/pair signal, quorum guards, and
Stage 5 FSV readbacks. Final FSV root:
`/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`.

Post-sweep #291 adds a shared sample-matrix guard: KSG and logistic-probe inputs
must be non-empty-dimensional, rectangular, and finite. Short sample count,
ragged rows, and NaN/Inf values all fail closed with
`CALYX_ASSAY_INSUFFICIENT_SAMPLES`; the Stage 5 FSV readback records those edge
codes.

Post-sweep #317 extends the same fail-closed contract to partitioned NMI:
mismatched, empty, n<50, and NaN/Inf scalar streams fail before binning, while
n=50 exactly is accepted. FSV root:
`/home/croyse/calyx/data/fsv-issue317-nmi-fail-closed-20260608`.

Post-sweep #318 wires the seeded bootstrap engine into the public KSG estimator,
logistic-probe estimator, AssayGate lens signal, PairGain estimate, and Aster
Assay CF persistence/readback path. FSV root:
`/home/croyse/calyx/data/fsv-issue318-bootstrap-ci-20260608`.

Post-sweep #319 adds `AsterAssayMaterializationGate`, which reads AsterVault
slot vectors and grounded binary anchors, computes PairGain, and feeds Loom
materialization planning before eager xterm materialization. Post-sweep #340
makes this adapter fail closed by default through `materialization_plan`; missing
anchors/slots return `CALYX_STALE_DERIVED`, and the zero-gain lazy fallback is an
explicit `materialization_plan_fail_safe_lazy` opt-in rather than a hidden
default.
FSV root:
`/home/croyse/calyx/data/fsv-issue319-aster-materialization-gate-20260608`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-assay/src/lib.rs` | Crate root; re-exports public API |
| `crates/calyx-assay/src/ksg.rs` | KSG estimator: k-NN MI via ANN graph, continuous↔continuous + continuous↔discrete, bias-corrected, bootstrap CI |
| `crates/calyx-assay/src/nmi.rs` | Partitioned histogram NMI (`partitioned_histogram_nmi_v1`), streaming, redundancy-graph use case |
| `crates/calyx-assay/src/projection.rs` | Random-projection pre-step for high-d: JL lemma projection to `2·ceil(log2(n))` dims; seeded deterministically |
| `crates/calyx-assay/src/bootstrap.rs` | Bootstrap CI engine: seeded 95% percentile-span envelope; configurable resamples (default 200) and seed (default 0) |
| `crates/calyx-assay/src/gate.rs` | `AssayGate` impl that wires `pair_gain` into PH27 `MaterializationPlan`; `lens_signal` entry point |
| `crates/calyx-assay/src/loom_adapter.rs` | AsterVault-backed adapter from grounded slot/anchor rows to fail-closed Loom materialization planning |
| `crates/calyx-assay/src/logistic.rs` | Binary-outcome logistic-probe MI estimator |
| `crates/calyx-assay/tests/stage5_fsv.rs` | Planted-synthetic FSV tests: known MI, known NMI, CI correctness, quorum enforcement, CPU projection determinism, GPU projection fail-loud honesty |

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

2. **Fails closed below quorum and malformed samples:** call `ksg_estimate` on
   n=30 paired vectors, a ragged matrix, and a NaN/Inf-containing matrix; call
   `partitioned_histogram_nmi` on empty, n=30, and NaN/Inf scalar streams. Each
   must return `Err(CALYX_ASSAY_INSUFFICIENT_SAMPLES)`, not a noisy point
   estimate. Verify via:
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
