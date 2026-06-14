# PH55 T03 - Query executor: relational filter -> graph hop -> vector fusion -> aggregate -> TS range

| Field | Value |
|---|---|
| **Phase** | PH55 - Cross-model transactions + universal query surface |
| **Stage** | S12 - Universal data layer |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/query/executor.rs` plus `query/executor/` modules, all under 500 lines |
| **Depends on** | T02 (CrossModelPlan + PlanStep), PH53 (all paradigm layers), PH54 T02/T03 (index queries), PH24 (RRF fusion), PH08 (MVCC snapshot) |
| **Axioms** | A15, A16, A19 |
| **PRD** | `dbprdplans/20 A-4`, `dbprdplans/10 A-0` |

## Goal

Execute a `CrossModelPlan` in one pass, pinned to a single MVCC snapshot, and
return a `QueryResult` with matching rows/values and provenance references where
the row key is a constellation ID. The executor is a pipeline: each `PlanStep`
filters or augments the result set from the previous step. Reads do not advance
across different seq values during a query.

## Build

- [x] Define `QueryResult` and `ProvenancedRow` in `query/mod.rs`:
  ```rust
  pub struct QueryResult {
      pub rows: Vec<ProvenancedRow>,
      pub total_scanned: u64,
      pub elapsed_ms: u32,
      pub explain: Option<ExplainOutput>,
  }
  pub struct ProvenancedRow {
      pub key: RecordKey,
      pub value: Option<Row>,
      pub score: Option<f32>,
      pub ledger_ref: Option<LedgerRef>,
  }
  ```
- [x] Implement `execute(vault: &AsterVault, plan: CrossModelPlan) -> Result<QueryResult>`.
- [x] Pin `snapshot_seq = vault.latest_seq()` at entry and use it for all reads.
- [x] Execute steps in order:
  1. `RelationalScan`: btree range when a planned index exists, otherwise full relational CF scan.
  2. `DocScan`: scan document IDs when no input rows exist, then call `get_subtree_at`.
  3. `KvGet`: point-read with absent/expired values skipped.
  4. `TsRangeScan`: snapshot-pinned range read emitting `(ts, value)` rows.
  5. `GraphHop`: PH27 stub returns input `CxId`s unchanged and logs `[INFO] graph hop stubbed`.
  6. `VectorFusion`: call PH24 restricted RRF over accumulated candidate `CxId`s.
  7. `Aggregate`: count/sum/min/max/avg over accumulated numeric values.
  8. `Ask`: delegated to T04 and fails closed with `CALYX_SEXTANT_QUERY_SHAPE`.
- [x] Add snapshot-pinned Aster helpers `KvLayer::kv_get_at` and `TimeSeriesLayer::ts_range_at`.
- [x] Annotate result rows with `LedgerRef` when the row key is a constellation `CxId`; plain-mode rows keep `None`.

## Tests

- [x] Unit relational: `[RelationalScan(qty >= 3)]` on five records returns pks `3,5,7`.
- [x] Unit document: `DocScan` filters a nested subtree value through `DocumentLayer::get_subtree_at`.
- [x] Unit multi-mode: `[RelationalScan, KvGet]` returns relational rows plus the KV row at one snapshot.
- [x] Unit TS: `[TsRangeScan(0..MAX)]` returns three points in ascending timestamp order.
- [x] Unit aggregate: `[RelationalScan, Aggregate(count)]` returns one count row with value `3`.
- [x] Proptest: non-empty combinations of relational/KV/TS/aggregate storage steps exclude post-snapshot writes.
- [x] Edge cases: empty collection returns zero rows; expired KV returns zero rows without error; graph stub passes input `CxId`s through; vector fusion with no candidates returns empty.
- [x] Fail-closed: ASK returns `CALYX_SEXTANT_QUERY_SHAPE` and no partial `QueryResult`.

## FSV

- **SoT:** Aster durable CF files under `/home/croyse/calyx/data/fsv-issue465-query-executor-final-20260614T154344Z/vault/cf` on aiwonder, plus `issue465-query-executor-readback.json`.
- **Trigger:** `CALYX_FSV_ROOT=<root> cargo test -p calyx-sextant --lib query::executor::fsv_tests::issue465_query_executor_fsv_writes_readback_artifacts -- --ignored --nocapture`.
- **Manual readback:** `cat` of the readback JSON, `find` of physical CF files, `sha256sum` of SST files, and `xxd` hex samples for relational, KV, timeseries, and ledger SSTs.
- **Proved:** before counts were zero; after counts were `relational_rows=6`, `kv_rows=2`, `timeseries_rows=6`, `ledger_rows=10`, `latest_seq=11`; happy path returned relational keys `3,5,7` and KV bytes `active`; TS returned `10,20,30`; aggregate returned count `3`; empty collection, expired KV, graph stub, vector-empty, pinned-snapshot post-write exclusion, and fail-closed ASK matched expected results.

## Completion Evidence

- Commit: `Implement PH55 query executor` on the issue branch; final SHA is recorded by GitHub/PR after merge.
- aiwonder gates:
  - `cargo fmt --all -- --check`
  - source line gate excluding target artifacts
  - `cargo check -p calyx-sextant`
  - `cargo clippy -p calyx-sextant --all-targets -- -D warnings`
  - `cargo test -p calyx-sextant --lib query::executor -- --nocapture` (11 passed, 1 ignored FSV test)
- FSV root: `/home/croyse/calyx/data/fsv-issue465-query-executor-final-20260614T154344Z`.

## Done When

- [x] `cargo check` + `clippy -D warnings` + tests are green on aiwonder.
- [x] Rust source/test files are under 500 lines.
- [x] FSV evidence is captured from aiwonder bytes and attached to issue/PR evidence.
- [x] No anti-pattern: no flattening, no ungrounded trusted claim, no frozen-lens mutation, and no harness-as-FSV.
