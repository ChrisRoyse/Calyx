# PH55 - T04 - `ASK`: multi-lens + `kernel_answer` + Oracle + provenance tag

| Field | Value |
|---|---|
| **Phase** | PH55 - Cross-model transactions + universal query surface |
| **Stage** | S12 - Universal data layer |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/query/ask.rs`, `crates/calyx-sextant/src/query/ask/*`, query planner/executor surfaces |
| **Depends on** | T03 executor context, PH24 RRF fusion, PH33 kernel-answer semantics, PH35 ledger provenance |
| **Issue** | `#466` |
| **Status** | Implemented and FSV-read on aiwonder |

## Goal

Implement the retrieval and provenance boundary for `ASK`: given a
natural-language question and a set of candidate `CxId`s from prior pipeline
steps, rank relevant constellations and prove the grounding rows from stored
`LedgerRef`s. Until a real answer synthesis/oracle implementation is wired,
answer mode must fail closed with `CALYX_ANSWER_SYNTHESIS_UNAVAILABLE`; it must
not return a synthetic answer or append a successful `QueryResult` answer row.

## Implementation Notes

- `AskSpec` now carries:
  - `question: String`
  - `context_cx_ids: Vec<CxId>`
  - `top_k: usize` with serde default `10`
  - `oracle: bool`
- `AskResult` is the eventual success shape:
  - `answer`
  - `grounding: Vec<ProvenancedRow>`
  - `gaps`
  - `oracle_conf`
- `query::ask::ask(...)` validates the question, pins the caller's snapshot, builds the candidate set from explicit context or full `Base` CF, ranks available per-slot vectors with PH24 restricted RRF, and tags every grounding row from `vault.get(cx_id, snapshot).provenance`.
- `PlanStep::Ask` now includes `top_k` and `oracle`; the planner propagates both fields from `AskSpec`.
- The executor now calls `query::ask::ask(...)`, using prior `ExecState` CxId candidates when the step has no explicit context. Because synthesis/oracle execution is not wired, `Ask` currently returns `CALYX_ANSWER_SYNTHESIS_UNAVAILABLE` after grounded retrieval and appends no partial `QueryResult.rows`.
- Empty question fails closed with `CALYX_INVALID_ARGUMENT`.
- No visible candidates or empty grounding fails closed with `CALYX_ANSWER_UNGROUNDED`.
- Candidate rows with no available lens slots fail closed with `CALYX_LENS_NOT_FOUND`.
- Oracle/synthesis is intentionally unwired in this phase. The allowed observable behavior is the explicit fail-closed error plus the grounding IDs in the error message for audit/debugging.

## Boundary Note

`calyx-lodestar` currently depends on `calyx-sextant`, so Sextant cannot
directly call Lodestar's concrete `kernel_answer` API without creating a crate
dependency cycle. This implementation keeps the PH33/PH49 call site as an
explicit fail-closed compatibility boundary until the shared kernel-answer
interface is split into a lower-level crate or a real synthesis/oracle adapter is
wired.

## Tests

- `query::ask` unit tests cover:
  - grounded retrieval failing closed with `CALYX_ANSWER_SYNTHESIS_UNAVAILABLE`
  - provenance tag from stored constellation ledger refs
  - empty context full-vault search failing with the same synthesis-unavailable code
  - `top_k=1` limiting grounding before fail-closed synthesis
  - empty question error
  - empty grounding error
  - unavailable lens error
- `query::executor` tests cover `PlanStep::Ask` returning `CALYX_ANSWER_SYNTHESIS_UNAVAILABLE` with no state mutation or partial `QueryResult`.
- Planner tests cover propagation of the expanded `AskSpec`.

## aiwonder Gates

Run from `/home/croyse/calyx/repo` on branch `issue466-ask`:

- `cargo fmt --all -- --check` - passed
- Rust line-count gate (`*.rs <= 500`) - passed
- `cargo check -p calyx-sextant` - passed
- `cargo clippy -p calyx-sextant --all-targets -- -D warnings` - passed
- `cargo test -p calyx-sextant --lib query:: -- --nocapture` - passed: 29 passed, 3 ignored
- `cargo test -p calyx-sextant --lib query::ask::fsv_tests::issue466_ask_fsv_writes_readback_artifacts -- --ignored --nocapture` - passed

## FSV Evidence

FSV root:

`/home/croyse/calyx/data/fsv-issue466-ask-20260614T161015Z`

Historical readback file:

`/home/croyse/calyx/data/fsv-issue466-ask-20260614T161015Z/issue466-ask-readback.json`

Manual SoT readback:

- Readback JSON was read directly with `cat`.
- Physical Aster files were listed under `vault/cf/base`, `vault/cf/ledger`, and `vault/cf/slot_00`.
- SHA-256 hashes were read for the JSON and every SST file in those CFs.
- SST bytes were sampled with `xxd` from Base, Ledger, and slot_00 files.

Superseded historical artifact state:

- Before: `base_rows=0`, `ledger_rows=0`, `slot_00_rows=0`, `latest_seq=0`.
- After: `base_rows=3`, `ledger_rows=3`, `slot_00_rows=3`, `latest_seq=3`.
- The original artifact predated the hardened ASK answer contract and showed an
  obsolete generated-result path. That behavior is no longer valid.
- The only reusable evidence from this artifact is the raw Base/Ledger/slot_00
  readback shape; current tests must assert the fail-closed contract below.
- Edge codes:
  - empty question: `CALYX_INVALID_ARGUMENT`
  - no visible grounding: `CALYX_ANSWER_UNGROUNDED`
  - unavailable lens: `CALYX_LENS_NOT_FOUND`

Current contract:

- Grounded ASK retrieval returns `CALYX_ANSWER_SYNTHESIS_UNAVAILABLE` until a
  real answer synthesis/oracle path is wired.
- The fail-closed path preserves the candidate's stored `LedgerRef`, leaves the
  vault sequence unchanged, and returns no successful answer row.
- If grounding-only output is needed before synthesis is wired, it must be added
  as a separate explicit non-answer mode rather than by weakening `ASK` answer
  mode.

## Done

- [x] `AskSpec` extended with `top_k` and `oracle`.
- [x] `ask(...)` implemented with snapshot-pinned candidate retrieval, restricted RRF, provenance tags, and fail-closed synthesis-unavailable behavior.
- [x] Executor `ASK` step wired to fail closed without appending partial rows until real synthesis/oracle execution exists.
- [x] Fail-closed errors implemented.
- [x] Unit tests and executor/planner tests updated.
- [x] aiwonder gates passed.
- [x] Manual FSV readback captured against durable Aster bytes.
