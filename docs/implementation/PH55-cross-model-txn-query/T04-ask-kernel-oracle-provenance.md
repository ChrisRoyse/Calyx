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

Implement the `ASK` query mode: given a natural-language question and a set of candidate `CxId`s from prior pipeline steps, rank relevant constellations, produce a grounded answer, and tag each grounding row with its `LedgerRef`.

## Implementation Notes

- `AskSpec` now carries:
  - `question: String`
  - `context_cx_ids: Vec<CxId>`
  - `top_k: usize` with serde default `10`
  - `oracle: bool`
- `AskResult` returns:
  - `answer`
  - `grounding: Vec<ProvenancedRow>`
  - `gaps`
  - `oracle_conf`
- `query::ask::ask(...)` validates the question, pins the caller's snapshot, builds the candidate set from explicit context or full `Base` CF, ranks available per-slot vectors with PH24 restricted RRF, and tags every grounding row from `vault.get(cx_id, snapshot).provenance`.
- `PlanStep::Ask` now includes `top_k` and `oracle`; the planner propagates both fields from `AskSpec`.
- The executor now calls `query::ask::ask(...)` and appends grounding rows to `QueryResult.rows`, using prior `ExecState` CxId candidates when the step has no explicit context.
- Empty question fails closed with `CALYX_INVALID_ARGUMENT`.
- No visible candidates or empty grounding fails closed with `CALYX_ANSWER_UNGROUNDED`.
- Candidate rows with no available lens slots fail closed with `CALYX_LENS_NOT_FOUND`.
- Oracle is wired as a PH49-compatible stub returning `oracle_conf=None`.
- Kernel answer is wired as a PH33-compatible stub returning `answer="[kernel stub]"`, grounding equal to top RRF CxIds, and no gaps.

## Boundary Note

`calyx-lodestar` currently depends on `calyx-sextant`, so Sextant cannot directly call Lodestar's concrete `kernel_answer` API without creating a crate dependency cycle. This implementation keeps the PH33/PH49 call site as a local compatibility boundary and preserves fail-closed grounding/provenance semantics until the shared kernel-answer interface is split into a lower-level crate.

## Tests

- `query::ask` unit tests cover:
  - stub answer with at least one grounding row
  - provenance tag from stored constellation ledger refs
  - empty context full-vault search
  - `top_k=1`
  - `oracle=false` returning `None`
  - empty question error
  - empty grounding error
  - unavailable lens error
- `query::executor` tests cover `PlanStep::Ask` appending grounding rows with provenance.
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

Readback file:

`/home/croyse/calyx/data/fsv-issue466-ask-20260614T161015Z/issue466-ask-readback.json`

Manual SoT readback:

- Readback JSON was read directly with `cat`.
- Physical Aster files were listed under `vault/cf/base`, `vault/cf/ledger`, and `vault/cf/slot_00`.
- SHA-256 hashes were read for the JSON and every SST file in those CFs.
- SST bytes were sampled with `xxd` from Base, Ledger, and slot_00 files.

Observed state:

- Before: `base_rows=0`, `ledger_rows=0`, `slot_00_rows=0`, `latest_seq=0`.
- After: `base_rows=3`, `ledger_rows=3`, `slot_00_rows=3`, `latest_seq=3`.
- Happy path answer: `[kernel stub]`.
- Happy path grounding count: `2`.
- Happy path ledger refs present with observed seqs `[1, 0]`.
- Executor ASK grounding count: `1`, ledger ref present.
- Full-vault search with empty context returned `2` grounding rows.
- `top_k=1` returned exactly `1` grounding row.
- Edge codes:
  - empty question: `CALYX_INVALID_ARGUMENT`
  - no visible grounding: `CALYX_ANSWER_UNGROUNDED`
  - unavailable lens: `CALYX_LENS_NOT_FOUND`
  - oracle stub confidence: `null`

## Done

- [x] `AskSpec` extended with `top_k` and `oracle`.
- [x] `ask(...)` implemented with snapshot-pinned candidate retrieval, restricted RRF, kernel stub, Oracle stub, and provenance tags.
- [x] Executor `ASK` step wired to append grounding rows.
- [x] Fail-closed errors implemented.
- [x] Unit tests and executor/planner tests updated.
- [x] aiwonder gates passed.
- [x] Manual FSV readback captured against durable Aster bytes.
