# PH49 — Consequence Prediction + Sufficiency Gate

**Stage:** S11 — Oracle & AGI Layer  ·  **Crate:** `calyx-oracle`  ·
**PRD roadmap:** `21 §1/§2`  ·  **Axioms:** A20, A2, A8

## Objective

Implement `oracle_predict(action, domain)` — the core Oracle API that returns a
calibrated outcome + consequences + the sufficiency bound + provenance + guard.
Confidence is capped at `oracle_self_consistency` (measured from grounded
recurrence). If `I(panel; oracle) < H(outcome)`, the call refuses to fabricate a
confident prediction: it returns `sufficient: false` with a per-sensor deficit and
`CALYX_ORACLE_INSUFFICIENT`. This honesty gate — falsifying the architecture's own
ability to predict before any model is trained — is the defining feature of the Oracle
layer. Prediction is a ME-JEPA step: `(panel_t, action) → panel_{t+1} / outcome`.

> Honesty is the feature: the same machine that predicts also falsifies its own
> ability to predict, cheaply, before training. (`21 §8`)

**Current state:** `calyx-oracle` is a 9-line stub (one metadata test); greenfield.

## Dependencies

- **Phases:** PH48 (J objective — oracle self-consistency ceiling from Anneal),
  PH42 (grounded recurrence — empirical rate/cadence as predictive evidence),
  PH30 (panel sufficiency + per-sensor attribution — `I(panel;oracle)` vs `H(outcome)`),
  PH28 (KSG MI machinery — the estimator used by the honesty gate),
  PH37 (Gτ guard math — `guard: GuardVerdict` field in `Prediction`)
- **Provides for:** PH50 (super-intelligence predicate reads the `Prediction` type and
  oracle self-consistency), PH51 (`complete()` extends `oracle_predict` to the unified
  energy descent)

## Current state (build off what exists)

Crate is a 9-line stub; greenfield. All types, logic, and tests are new. Depends on
`calyx-assay` (MI/sufficiency), `calyx-ward` (guard), `calyx-loom` (recurrence edges),
`calyx-ledger` (provenance ref). No existing oracle logic to preserve.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/lib.rs` | Crate root: re-exports, module declarations |
| `src/types.rs` | `Prediction`, `Consequence`, `SufficiencyBound`, `OracleSelfConsistency`, `ConsequenceTree`, `OracleError` types |
| `src/self_consistency.rs` | `oracle_self_consistency(domain)` — measure flakiness + validity ceiling from grounded recurrence outcomes (`07 §3b`); returns `{flakiness, validity, ceiling}` |
| `src/honesty_gate.rs` | `check_sufficiency(panel, domain)` — delegates to Assay; produces `SufficiencyBound`; emits `CALYX_ORACLE_INSUFFICIENT` with per-sensor deficit when `I(panel;oracle) < H(outcome)` |
| `src/predict.rs` | `oracle_predict(vault, action, domain)` — JEPA step `(panel_t, action)→panel_{t+1}/outcome`; caps confidence; calls honesty gate; returns `Prediction` |
| `src/butterfly.rs` | `expand(consequence)` / `select(branch)` — butterfly tree traversal (hop-attenuated consequences); `expand` recurses to depth via recurrence edges; `select` returns the branch whose consequences match a desired outcome |
| `src/error.rs` | `CALYX_ORACLE_INSUFFICIENT`, `CALYX_ORACLE_FLAKY_ANCHOR`, `CALYX_ORACLE_NO_RECURRENCE` structured error catalog |
| `tests/predict_tests.rs` | FSV-supporting deterministic tests |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Oracle types + error catalog | — |
| T02 | `oracle_self_consistency` from grounded recurrence | T01 |
| T03 | Honesty gate: `check_sufficiency` + `CALYX_ORACLE_INSUFFICIENT` | T02 |
| T04 | `oracle_predict` JEPA step + confidence ceiling | T03 |
| T05 | Butterfly tree: `expand` + `select` (hop-attenuated) | T04 |
| T06 | FSV: SWE-bench Lite ≈0.46-bit deficit triggers sufficiency-refusal | T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

On a real deterministic-oracle domain (SWE-bench Lite on aiwonder):
1. `oracle_predict` with a real code-change action returns a `Prediction` with
   `confidence` ≤ `oracle_self_consistency.ceiling`.
2. On the **form-only panel** (lenses that measure code appearance only), `check_sufficiency`
   measures `I(panel; oracle) ≈ 0.46 bits` against `H(outcome) ≈ 1 bit`; `sufficient: false`
   is returned with `CALYX_ORACLE_INSUFFICIENT` and a per-sensor deficit vector.
3. Confidence never exceeds the ceiling in any call: read the `confidence` and
   `bound.dpi_ceiling` fields from the returned `Prediction` bytes and verify
   `confidence ≤ dpi_ceiling`.
4. Readback: `calyx readback oracle_predict <domain>` prints the `Prediction` struct
   as JSON; `xxd` the ledger entry proves provenance was written.

## Risks / landmines

- **Circular sufficiency call:** `check_sufficiency` must delegate to Assay (PH30) via
  the crate interface, not reimplement KSG — duplicate MI code drifts. Wire via trait.
- **Ceiling enforcement:** confidence cap must occur after all aggregation steps;
  a post-hoc clamp is correct only if grounded recurrence CI is propagated faithfully.
- **Butterfly tree depth explosion:** `expand` must hard-limit hop depth (configurable,
  default 4) and apply hop-attenuation to confidence per step; unbounded recursion on
  cyclic graphs → stack overflow.
- **JEPA step is a forward-association query, not a neural forward pass:** implement as
  a weighted recurrence-edge traversal over the stored transition constellations, not
  a learned model call — strictly Royse corpus (`21 §2`, A24).
