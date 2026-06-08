# PH33 · T06 — Kernel build/answer → Ledger provenance wiring (`kind=Kernel`)

| Field | Value |
|---|---|
| **Phase** | PH33 — Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` + `calyx-ledger` |
| **Issue** | #239 |
| **Depends on** | PH33-T02, PH35/PH36 real Ledger append/query/reproduce |
| **Axioms** | A10, A11, A15 |
| **PRD** | `dbprdplans/08 §6`, `dbprdplans/11 §1/§3` |

## Goal

Replace PH33's structured provenance stubs with real Ledger entries. Kernel build
must append a `kind=Kernel` evidence row containing `kernel_id`, members hash,
MFVS approximation factor, recall ratio, and graph sequence. `kernel_answer` must
stamp every hop with a real `LedgerRef` so answer traces can be audited and
reproduced.

## Build

- [ ] `build_kernel` appends a `kind=Kernel` Ledger entry in the same provenance
  path as the kernel artifact.
- [ ] `kernel_answer` replaces hop stubs with real Ledger appends.
- [ ] `get_answer_trace` returns the kernel entry and hop entries in order.
- [ ] `reproduce` reruns the answer path and detects drift or tampering.
- [ ] Missing Ledger integration is fail-closed; Stage 6 exit cannot mark real
  provenance complete from a stub.

## FSV

- **SoT:** Ledger column-family bytes on aiwonder after a kernel build and
  `kernel_answer` execution.
- **Readback:** `calyx readback --cf ledger` plus `get_answer_trace` / `reproduce`
  output for the same answer.
- **Prove:** readback shows one `kind=Kernel` entry and one entry per answer hop;
  `reproduce` passes on the untouched path and fails after a byte-flip tamper test.

## Done when

- [ ] PH35/PH36 real Ledger primitives are available
- [ ] cargo check + clippy `-D warnings` + test green on aiwonder
- [ ] file(s) ≤ 500 lines
- [ ] Ledger readback evidence attached to #239 and Stage 6 exit #240
- [ ] no provenance stub is counted as real Stage 6 exit evidence
