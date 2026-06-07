# PH55 · T03 — Query executor: relational filter → graph hop → vector fusion → aggregate → TS range

| Field | Value |
|---|---|
| **Phase** | PH55 — Cross-model transactions + universal query surface |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/query/executor.rs` (≤500) |
| **Depends on** | T02 (CrossModelPlan + PlanStep), PH53 (all paradigm layers), PH54 T02/T03 (index queries), PH24 (RRF fusion), PH08 (MVCC snapshot) |
| **Axioms** | A15, A16, A19 |
| **PRD** | `dbprdplans/20 §4`, `dbprdplans/10 §0` |

## Goal

Execute a `CrossModelPlan` in one pass, pinned to a single MVCC snapshot, and
return a `QueryResult` with all matching rows/values and their provenance
references. The executor is a pipeline: each `PlanStep` filters or augments
the result set from the previous step. No partial reads across different seq
values; no deadlocks (single-writer serialization + read-only snapshot).
The final result is a `ProvenancedResult` (each row/value carries a
`LedgerRef` or `None` for plain-mode collections).

## Build (checklist of concrete, code-level steps)

- [ ] Define `QueryResult` and `ProvenancedResult` in `query/mod.rs`:
  ```rust
  pub struct QueryResult {
      pub rows: Vec<ProvenancedRow>,
      pub total_scanned: u64,
      pub elapsed_ms: u32,
      pub explain: Option<ExplainOutput>,
  }
  pub struct ProvenancedRow {
      pub key: RecordKey,
      pub value: Option<Row>,          // None for index-only results
      pub score: Option<f32>,          // for vector/FTS ranked results
      pub ledger_ref: Option<LedgerRef>,
  }
  ```
- [ ] Implement `execute(vault: &AsterVault, plan: CrossModelPlan) -> Result<QueryResult>`:
  - Pin `snapshot_seq = vault.current_seq()` at entry; ALL reads use this seq.
  - Execute steps in order:
    1. `RelationalScan`: call `btree_range` or full-scan depending on `index` field;
       collect `Vec<RecordKey>`.
    2. `DocScan`: for each `RecordKey`, `get_subtree` on the path filter; keep
       matching.
    3. `KvGet`: point-read; emit one row or skip if absent/expired.
    4. `TsRangeScan`: call `ts_range`; emit `(ts, val)` pairs as rows.
    5. `GraphHop`: for each input `CxId`, look up `xterm` CF keys
       `(cx_id, *, *, Agreement)` (or stub if PH27 not built: pass-through);
       collect reachable `CxId`s.
    6. `VectorFusion`: call `sextant::fuse_rrf` (PH24) with the accumulated
       `CxId` set as candidate filter; return ranked `(CxId, score)` pairs.
    7. `Aggregate`: compute count/sum/min/max/avg over the numeric values in the
       accumulated rows.
    8. `Ask`: delegated to T04.
  - After all steps: annotate each row with `ledger_ref` from the `ledger` CF
    (for Constellation-mode rows) or `None` (for plain-mode rows).
  - Return `QueryResult`.
- [ ] All reads use the pinned `snapshot_seq`; never advance the snapshot mid-query.
- [ ] `RelationalScan` with no `IndexSpec` → full CF scan (accepted only when
  `estimated_cost` was under cap — planner already checked this).
- [ ] `GraphHop` stub (when PH27 not available): return `input_cx_ids` unchanged
  (0-hop). Log a structured `[INFO]` "graph hop stubbed".

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit relational: plan `[RelationalScan(qty>=3)]` on a 5-record collection →
  `rows.len() == 3` (pks with qty 3,5,7); `elapsed_ms < 1000`.
- [ ] unit multi-mode: plan `[RelationalScan, KvGet]` on a vault with both CFs →
  2 entries in `rows` (one relational row + one KV row); same `snapshot_seq`.
- [ ] unit TS: plan `[TsRangeScan(0..MAX)]` on 3 written points → 3 rows in
  ascending ts order.
- [ ] unit aggregate: plan `[RelationalScan, Aggregate(count)]` → `rows` has 1
  entry with `value = count` of matching relational rows.
- [ ] proptest: for any combination of non-empty plan steps with seeded data, all
  result rows were visible at the pinned `snapshot_seq` (no row created after
  `snapshot_seq` appears in results).
- [ ] edge (≥3): (1) empty collection → `rows.len() == 0`; (2) KvGet on expired
  TTL → 0 rows (not error); (3) `GraphHop` stub → input cx_ids passed through
  unchanged; (4) VectorFusion on empty candidate set → empty result.
- [ ] fail-closed: if any step returns `Err`, the whole `execute` returns `Err`;
  no partial result emitted.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-sextant query::executor` on aiwonder.
- **Readback:**
  ```
  cargo test -p calyx-sextant executor -- --nocapture 2>&1 | tail -20
  calyx query --vault /home/croyse/calyx/test-vault \
    --filter 'orders.qty >= 1' --kv 'ns=1,key=sess' --agg count --explain
  ```
- **Prove:** Query returns `rows` covering both relational and KV collections
  at the same snapshot_seq; `--explain` shows `snapshot_seq=N` for all steps.
  Evidence posted to PH55 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH55 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
