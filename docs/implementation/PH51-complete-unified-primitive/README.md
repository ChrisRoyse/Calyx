# PH51 — `complete()` Unified Primitive (predict = abduce = impute)

**Stage:** S11 — Oracle & AGI Layer  ·  **Crate:** `calyx-oracle`  ·
**PRD roadmap:** `26 §11.1`  ·  **Axioms:** A2, A16, A20

## Objective

Implement `complete(cx, clamp, free)` — the single energy-descent primitive that unifies
forward prediction, abduction (reverse_query), and lateral imputation:

```
complete(cx, clamp: SlotSet, free: SlotSet) -> filled cx + confidence
  clamp present, free future    → PREDICTION   (the Oracle / consequence)
  clamp outcome, free cause     → ABDUCTION    (reverse_query / root cause)
  clamp some lenses, free rest  → IMPUTATION   (slot completion / repair)
```

The energy function is `E(x) = −log Σ_i exp(β · sim(x, cx_i))` over the region members
(softmax-weighted similarity; β is Anneal-tuned). Completion = a few gradient-free descent
steps updating absent slots. Free (filled) slots are tagged `inferred` or `provisional`,
never confused with measured ones. Confidence capped at `oracle_self_consistency`. Refused
when the panel is insufficient (A20). This is the discovery that prediction, abduction, and
imputation are the same energy-descent over a constellation, differing only in which slots
are clamped vs free (`26 §11.1`).

> Honesty is the feature: completed slots are always tagged `inferred`/`provisional` (A2),
> never overwrite measured anchor values (A16). Refuse if panel insufficient (A20).

**Current state:** PH49 and PH50 complete. `complete()` is a new module; no energy-descent
logic exists yet. Reuses Forge `batched_cosine` + softmax (PH12/PH13) and Anneal autotune (PH46).

## Dependencies

- **Phases:** PH50 (oracle layer complete; `reverse_query` back-edges inform abduction
  traversal), PH37 (Gτ energy/region math — `GuardProfile` defines region members for
  the energy sum), PH46 (Anneal autotune — tunes β sharpness parameter), PH13 (Forge CUDA
  — `batched_cosine` + softmax used in energy computation)
- **Provides for:** PH52 (advanced math uses `complete` as the unified pattern-completion
  mechanism for energy-based pattern completion `26 §3`)

## Current state (build off what exists)

Greenfield within PH49/PH50 oracle infrastructure. Forge `batched_cosine` and softmax
are available from PH12/PH13. Anneal β parameter registry exists from PH46. Ward Gτ region
membership is available from PH37. New file: `src/complete.rs`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/complete.rs` | `complete(cx, clamp, free)` — energy descent; β-softmax; slot update loop; `inferred`/`provisional` tagging; sufficiency gate; confidence cap |
| `src/energy.rs` | `energy(cx, region_members, beta)` = `−log Σ exp(β·sim)`; gradient-free descent step; β Anneal integration |
| `src/types.rs` (extend) | `SlotSet`, `CompletionResult { filled_cx, confidence, filled_slots: Vec<(SlotId, SlotTag)> }`, `SlotTag { inferred | provisional | measured }` |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Energy function + β-softmax + descent step | — |
| T02 | `SlotSet`, `SlotTag`, `CompletionResult` types | T01 |
| T03 | `complete(cx, clamp, free)` — full primitive with sufficiency gate | T02 |
| T04 | FSV: partial constellation completes to known full; slots tagged `inferred` | T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

On synthetic: a partial constellation (3 of 7 slots populated) completes to the known full
within tolerance; completed slots are tagged `inferred`, never `measured`. Read:
```
calyx readback complete --cx <partial_cx_id> --clamp lens_1,lens_2,lens_3 --free lens_4,lens_5,lens_6,lens_7
```
Verify: `filled_cx` has 7 slots; free slots carry `SlotTag::Inferred`; clamped slots carry
`SlotTag::Measured`; `confidence ≤ oracle_self_consistency.ceiling`; insufficient panel →
`CALYX_ORACLE_INSUFFICIENT` fires before descent begins.

## Risks / landmines

- **β divergence:** at large β, softmax concentrates on one member (argmax); descent step may
  oscillate if the region has two near-equal attractors. Bound β via Anneal; add early-stop
  when energy change < ε between steps.
- **Slot overwrite guard:** clamped slots must never be modified by the descent; enforce with
  an immutable `clamped: SlotSet` mask checked at every step write.
- **`inferred` vs `provisional`:** `inferred` = filled by energy descent (confident given the
  panel); `provisional` = edge-case (panel near-insufficient, or descent did not converge).
  Distinguish by checking whether the descent converged (energy decrease < ε) and whether
  `I_panel_oracle >= H(outcome)`.
- **Forge batched_cosine reuse:** complete uses the same cosine kernel as Sextant HNSW — do
  not fork; call via the established Forge API.
