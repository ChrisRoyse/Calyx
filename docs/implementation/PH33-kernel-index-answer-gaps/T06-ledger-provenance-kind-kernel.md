# PH33 - T06 - Kernel build/answer -> Ledger provenance wiring

| Field | Value |
|---|---|
| **Phase** | PH33 - Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` + `calyx-ledger` |
| **Issue** | #239 |
| **Depends on** | PH33-T02, PH35 real Ledger append |
| **Axioms** | A10, A11, A15 |
| **PRD** | `dbprdplans/08 Section 6`, `dbprdplans/11 Section 1/3` |

## Goal

Replace PH33's stub-only provenance path with real PH35 Ledger appends on the
ledger-backed build and answer APIs. Kernel build appends one `kind=Kernel`
evidence row containing `kernel_id`, `members_hash`, MFVS approximation factor,
recall ratio, and graph sequence. `kernel_answer_with_ledger` stamps every hop
with a real `LedgerRef` from an appended `kind=Answer` row.

Full audit query and reproduce surfaces were later closed in PH36 work
(#252-#255); the checked PH36 lines below record that later closure.

## Build

- [x] `build_kernel_pipeline_with_ledger` appends a `kind=Kernel` Ledger entry
  through the PH35 `LedgerAppender`.
- [x] `kernel_answer_with_ledger` appends one real `kind=Answer` Ledger entry
  per hop and returns those refs in `AnswerPath.provenance`.
- [x] The existing pure `kernel_answer` compatibility path remains deterministic,
  but is not valid Stage 6 exit evidence for real provenance.
- [x] Missing/corrupt Ledger integration fails closed with the underlying
  `CALYX_LEDGER_*` code surfaced through `LodestarError`.
- [x] PH36 `get_answer_trace` returns kernel and hop entries in order (#254).
- [x] PH36 `reproduce` reruns the answer path and detects drift/tamper (#253/#255).

## FSV

- **SoT:** real PH35 `DirectoryLedgerStore` row bytes on aiwonder after a kernel
  build and a three-hop `kernel_answer_with_ledger` execution.
- **Readback:** evidence root
  `/home/croyse/calyx/data/fsv-issue239-kernel-ledger-provenance-20260608`.
  Readbacks include:
  - `ph33-ledger-provenance-readback.json`
  - `ph33-ledger-decoded-rows.json`
  - `04-ledger-row-files.out`
  - `04b-ledger-row-sizes.out`
  - `05-ledger-row-hex.out`
  - `07-secret-grep-count.out`
- **Prove:** before count is 0; after count is 4; row seq 0 is `kind=Kernel`;
  row seq 1..3 are `kind=Answer`; every `prev_hash` equals the previous
  `entry_hash`; answer path refs match the decoded rows; secret grep count is 0.

## Done when

- [x] PH35 real Ledger append primitives are available.
- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] File(s) <= 500 lines.
- [x] Ledger row byte/hex readback evidence attached to #239.
- [x] No provenance stub is counted as real Stage 6 exit evidence.
