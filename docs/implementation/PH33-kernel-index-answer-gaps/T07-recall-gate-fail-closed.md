# PH33 T07 - Recall below gate fails closed

| Field | Value |
|---|---|
| **Issue** | #330 |
| **Phase** | PH33 - Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `src/error.rs`, `src/recall_test.rs`, `src/lib.rs`, `tests/ph33_recall_test_tests.rs`, `tests/support/real_corpora.rs` |
| **Depends on** | T04 recall harness, T05 real-corpora FSV |

## Problem

Before #330, `kernel_recall_test` produced
`warning = CALYX_KERNEL_RECALL_BELOW_GATE` when `ratio < min_recall_ratio`, but
still returned `Ok(RecallTestReport)`. That was acceptable for diagnostics but
too weak for Stage 6 exit because callers could forget to inspect the warning.

## Fix

- [x] Add `LodestarError::RecallBelowGate` with code
  `CALYX_KERNEL_RECALL_BELOW_GATE`.
- [x] Add `kernel_recall_gate` and `kernel_recall_gate_with_clock` for
  fail-closed acceptance.
- [x] Add `enforce_recall_gate` for converting a report-only run into the same
  fail-closed contract.
- [x] Preserve `kernel_recall_test` report-only behavior for tuning and
  diagnostics.
- [x] Re-export the gate helpers from `calyx_lodestar`.
- [x] Make the real-corpora final acceptance path call `kernel_recall_gate`.

## Tests

- [x] Perfect kernel passes both report-only and gate mode.
- [x] Degraded kernel returns a warning in report-only mode.
- [x] Degraded kernel returns `Err(CALYX_KERNEL_RECALL_BELOW_GATE)` in gate mode.
- [x] Empty corpus returns `CALYX_RECALL_EMPTY_CORPUS`.
- [x] Invalid parameters return `CALYX_RECALL_INVALID_PARAMS`.
- [x] Deterministic sampling readbacks remain byte-identical for the same seed.

## FSV

- **SoT:** `/home/croyse/calyx/data/fsv-issue330-recall-gate-fail-closed-20260608`.
- **Readbacks:** `gate/recall-gate-fail-closed.json`,
  `degraded/recall-test-degraded.json`, `edges/recall-test-edges.json`,
  `01-recall-gate-test.out`, and real-corpora gate stdout.
- **Manual proof:** read back the JSON files after the test run and verify the
  exact error codes and ratios. The test result alone is not the verdict.

## Done when

- [x] #330 FSV readbacks show below-gate recall fails closed.
- [x] aiwonder gates pass: fmt, check, test, clippy, line-count.
- [x] #330 is closed with evidence and #23 is updated.
