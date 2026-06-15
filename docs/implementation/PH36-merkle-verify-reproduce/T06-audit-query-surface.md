# PH36 T06 - Audit query surface: `get_provenance`, `get_answer_trace`, `audit(filter)`

| Field | Value |
|---|---|
| **Phase** | PH36 - Merkle checkpoints + verify_chain + reproduce() |
| **Stage** | S7 - Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/audit.rs`, `crates/calyx-ledger/src/lib.rs`, `crates/calyx-cli/src/provenance.rs` |
| **Depends on** | T02 (this phase) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 section 5`, `11 section 7` |

## Goal

Expose the public audit API defined in `11 section 5` so calyx-mcp, calyx-cli, and
downstream crates (PH61, PH62, PH63) have a stable, typed surface to query
ledger provenance. Every function that returns results from a quarantined range
returns `CALYX_LEDGER_CHAIN_BROKEN` (fail-closed). Every result that cannot be
traced to a complete ledger entry is tagged `CalyxWarning::Unprovenanced` and is
not trusted.

## Build

- [x] `pub fn get_provenance(cf_reader, quarantine, cx_id: CxId) -> Result<Vec<LedgerEntry>>` returns all entries whose typed subject or explicit cx payload fields reference `cx_id` (for example `cx_id`, `from_id`, `to_id`, `source_cx_id`, `target_cx_id`, `nearest_cx`, `matched_cx_id`, `query_id`, `anchor_kernel_node_id`), ignores arbitrary comment/note/string matches, and checks quarantine before returning any row.
- [x] `pub fn get_answer_trace(cf_reader, quarantine, answer_id: QueryId) -> Result<AnswerTrace>` returns `answer_entry`, linked `kernel_entry` and `guard_entry`, ordered path hops, fusion weights, guard result, freshness timestamp, completeness, and warnings.
- [x] `get_answer_trace` is all-or-nothing for trusted answer provenance: only an explicit `complete: true` Answer summary with a canonical path can be trusted; per-hop rows or unmarked paths are returned with `Unprovenanced`.
- [x] `verify_chain`, `merkle_root`, and `reproduce` remain re-exported from their PH36 modules for the stable public API.
- [x] `pub fn audit(cf_reader, quarantine, filter: AuditFilter) -> Result<Vec<LedgerEntry>>` filters by kind, actor, timestamp range, and sequence range. Explicit `seq_range` overlap and matching/relevant result rows fail closed on quarantine; unrelated quarantined rows outside the filtered result set do not poison the query. Physical ledger row-key/encoded-seq mismatches fail closed.
- [x] `CalyxWarning::Unprovenanced { surface: String }` is available from `calyx-core`.
- [x] CLI commands added: `calyx get-provenance --vault <dir> --cx <cx-id>`, `calyx get-answer-trace --vault <dir> --answer <answer-id-or-hex>`, and `calyx audit --vault <dir> --kind <kind>`.

## Tests

- [x] Unit: 3 constellations -> `get_provenance(cx_id[0])` returns only that cx's provenance rows.
- [x] Unit: known complete Answer row -> `get_answer_trace(answer_id)` returns exact hop count, fusion weights, linked Kernel row, linked Guard row, and no warnings.
- [x] Unit: `audit(AuditFilter { kind: Some(Ingest), .. })` over 10 entries (5 Ingest, 5 Measure) returns exactly 5 entries.
- [x] Edges: missing cx returns `Ok(vec![])`; quarantined answer returns `CALYX_LEDGER_CHAIN_BROKEN`; excluded timestamp range returns `Ok(vec![])`; unmarked path rows are `Unprovenanced`.
- [x] Fail-closed: `get_provenance` and `get_answer_trace` refuse quarantined rows/ranges before returning data; filtered `audit` refuses explicit `seq_range` overlap and matching/relevant quarantined rows while ignoring unrelated quarantined rows outside the result set.
- [x] Post-Stage-5 addendum: injected `kernel_answer_with_ledger` mid-hop append failure leaves only partial hop rows, and `get_answer_trace` refuses them as complete/trusted.

## FSV

**SoT:** aiwonder bytes under
`/home/croyse/calyx/data/fsv-issue254-audit-query-20260609`.

**Readback artifacts:**

- `audit-query-surface/audit-query-readback.json`
  - SHA-256: `c72fd19bb132533ffdf613d6ca4563e97e458bd54ac4074937f07fea1c94c09d`
- `ph36-audit-mid-hop-failure/ph36-audit-mid-hop-failure-readback.json`
  - SHA-256: `5948a107fff864195659b9cffe89ae4475a21d04afb943efcc438860fb731c25`

**Proven outcomes:**

- `get-provenance` returned 5 rows covering Ingest, Measure, Assay, Guard, and Answer provenance for the synthetic cx.
- `get-answer-trace` returned `complete=true`, 2 ordered hops, linked Kernel and Guard rows, fusion weights, and `warnings=[]`.
- `audit --kind ingest` returned exactly 3 rows.
- The manifest-quarantined Answer seq 8 returned `CALYX_LEDGER_CHAIN_BROKEN` immediately.
- Partial per-hop Answer rows returned `complete=false` and `Unprovenanced`.
- Injected `kernel_answer_with_ledger` mid-hop failure wrote 2 disk rows (Kernel + one Answer hop), returned `CALYX_LEDGER_CHAIN_BROKEN`, and `get_answer_trace` read `trace_complete=false`, `trace_trusted=false`, `trace_path_len=1`, and `Unprovenanced { surface: "answer_trace.partial_or_unmarked" }`.
- #349 addendum: `/home/croyse/calyx/data/fsv-issue349-audit-query-hardening-20260609-5697553` proved a filtered `audit --kind ingest` ignores unrelated quarantined seq `1` and returns seqs `[0,2]`; `audit --kind measure` fails closed on that matching quarantined row; `get-provenance` returns typed/explicit cx rows `[0,4]` and ignores arbitrary comment/note strings; durable Ledger SST rows and `sha256-manifest.txt` were read back from aiwonder.

## Done when

- [x] `cargo check` green on aiwonder.
- [x] `cargo clippy -D warnings` green on aiwonder.
- [x] `cargo test` green on aiwonder.
- [x] File line-count gate passed (`.rs` files <= 500 lines).
- [x] FSV evidence attached to GitHub issue #254.
- [x] No PH36 anti-pattern: no partial answer trace is labeled trusted, no quarantined row is served, and CLI output includes payload hex for byte evidence.
