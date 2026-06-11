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
with a real `LedgerRef` from an appended `kind=Answer` row, then appends a
final complete `kind=Answer` row for `get_answer_trace`. Post-#647, the
direct-hit `query_cx == anchor` path also appends that complete Answer row with
`expected_hops=0` before returning.

Full audit query and reproduce surfaces were later closed in PH36 work
(#252-#255); the checked PH36 lines below record that later closure.

## Build

- [x] `build_kernel_pipeline_with_ledger` appends a `kind=Kernel` Ledger entry
  through the PH35 `LedgerAppender`.
- [x] `kernel_answer_with_ledger` appends one real `kind=Answer` Ledger entry
  per hop and returns those refs in `AnswerPath.provenance`.
- [x] `kernel_answer_with_ledger` appends the final complete `kind=Answer` row
  even for direct-hit zero-hop answers (#647).
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
- **Prove:** before count is 0; after rows include one `kind=Kernel`, per-hop
  `kind=Answer` rows when hops exist, and a final complete `kind=Answer` row.
  For #647 direct-hit FSV, after count is 2, seq 1 is `kind=Answer` with
  `complete=true`, `expected_hops=0`, and `path=[]`; `get_answer_trace` returns
  `trace_trusted=true` with `trace_path_len=0`. Evidence root:
  `/home/croyse/calyx/data/fsv-issue647-direct-hit-ledger-20260611T073538Z`;
  primary readback SHA-256
  `c14c9e985ed3b63ed5faba5ac91fcd6f324a8bd5f2762f7525992082ca57c9d8`.

## Done when

- [x] PH35 real Ledger append primitives are available.
- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] File(s) <= 500 lines.
- [x] Ledger row byte/hex readback evidence attached to #239.
- [x] No provenance stub is counted as real Stage 6 exit evidence.
