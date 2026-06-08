# PH33 T04 - Recall test harness and gate API

| Field | Value |
|---|---|
| **Phase** | PH33 - Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/recall_test.rs` (<=500) |
| **Depends on** | T01 (`kernel_search`), T02 (`kernel_answer`), T03 (`grounding_gaps`) |
| **Axioms** | A10 |
| **PRD** | `dbprdplans/08`, `16_STAGE6_LODESTAR.md` |

## Goal

Measure kernel-only recall against the full index with deterministic held-out
queries. PH33 now exposes two explicit modes:

- `kernel_recall_test(...)`: report-only diagnostics. If recall is below the
  configured gate, the report carries `warning =
  CALYX_KERNEL_RECALL_BELOW_GATE`.
- `kernel_recall_gate(...)`: acceptance/exit mode. If recall is below the
  configured gate, the call returns `Err(CALYX_KERNEL_RECALL_BELOW_GATE)`.

Stage 6 exit and real-corpora FSV must use `kernel_recall_gate`; tuning and
exploratory diagnostics may use the report-only API.

## Build

- [x] `RecallTestParams` records held-out fraction, `top_k`, RNG seed, and
  `min_recall_ratio`.
- [x] `kernel_recall_test` samples deterministic queries, compares kernel ANN
  hits to full-index hits, and writes a structured `RecallTestReport`.
- [x] Report-only mode preserves warning bytes for below-gate runs so FSV can
  inspect the exact diagnostic state.
- [x] `kernel_recall_gate` and `kernel_recall_gate_with_clock` enforce
  fail-closed acceptance semantics.
- [x] `enforce_recall_gate` converts an already-produced report into an error
  when `ratio < min_recall_ratio`.
- [x] `rng_seed = 0` uses the injected `Clock`; non-zero seeds are exact and do
  not use `thread_rng`.
- [x] Empty corpus or zero selected queries returns
  `CALYX_RECALL_EMPTY_CORPUS`.
- [x] Invalid parameters return `CALYX_RECALL_INVALID_PARAMS`.

## Tests

- [x] Perfect kernel: report ratio is `1.0`, and gate mode passes.
- [x] Degraded kernel: report-only mode returns `Ok(report)` with
  `CALYX_KERNEL_RECALL_BELOW_GATE`.
- [x] Degraded kernel: gate mode returns
  `Err(CALYX_KERNEL_RECALL_BELOW_GATE)`.
- [x] Determinism: same corpus and seed produce identical held-out query IDs.
- [x] Edge coverage: full held-out fraction, zero held-out fraction, empty
  corpus, invalid params, and clock-derived seed.

## FSV

- **Issue:** #330.
- **SoT:** aiwonder files under
  `/home/croyse/calyx/data/fsv-issue330-recall-gate-fail-closed-20260608`.
- **Readbacks:** `gate/recall-gate-fail-closed.json`,
  `degraded/recall-test-degraded.json`, `edges/recall-test-edges.json`, and
  test stdout captured in `01-recall-gate-test.out`.
- **Prove:** the degraded report contains warning bytes, the degraded gate
  returns `CALYX_KERNEL_RECALL_BELOW_GATE`, empty corpus returns
  `CALYX_RECALL_EMPTY_CORPUS`, and deterministic runs match exactly.

## Done when

- [x] `cargo check`, `clippy -D warnings`, and `test` pass on aiwonder.
- [x] `.rs` files remain <=500 lines.
- [x] FSV readback files prove report-only and fail-closed modes separately.
- [x] Stage 6 docs direct acceptance flows to `kernel_recall_gate`.
