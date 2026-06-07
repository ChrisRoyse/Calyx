# PH55 · T05 — FSV: one txn spans modes atomically; cross-model query one provenanced pass; unbounded plan rejected

| Field | Value |
|---|---|
| **Phase** | PH55 — Cross-model transactions + universal query surface |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-sextant` + `calyx-aster` |
| **Files** | `crates/calyx-sextant/tests/ph55_fsv.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04 |
| **Axioms** | A15, A16, A17, A19 |
| **PRD** | `dbprdplans/20 §4/§8`, `dbprdplans/22 §PH55` |

## Goal

A seeded integration test — the phase FSV gate — proving all three exit
conditions on aiwonder: (1) one transaction touching relational + constellation
+ graph collections commits at a single seq with no partial read and no
deadlock; (2) a cross-model query returns one provenanced result set in one
pass; (3) an unbounded plan is rejected with `CALYX_PLANNER_COST_CAP` before
any execution. The bytes on disk are the proof; no harness assertion counts.

## Build (checklist of concrete, code-level steps)

- [ ] Create `tests/ph55_fsv.rs`; use `/tmp/calyx-ph55-fsv-test`; clean at start.

### Scenario A — Cross-model atomic txn (no partial read, no deadlock)
- [ ] Create 3 collections: `orders` (Records), `kv_state` (KV), `cxs` (Constellations,
  1 lens attached via `add_lens` stub).
- [ ] `CrossModelTxn::begin(isolation=Serializable, cost_cap_ms=Some(5000))`.
- [ ] `put_record(orders, pk=1, {qty:7})`.
- [ ] `kv_set(kv_state, ns=1, key=b"last_order", val=b"1")`.
- [ ] `put_constellation(cxs, input="order #1 placed")` (via PH09 path).
- [ ] `commit()` → capture `seq=N`.
- [ ] Read each CF: `get_record(orders, pk=1)` at seq N → `Some({qty:7})`;
  `kv_get(kv_state, b"last_order")` at seq N → `Some(b"1")`;
  `get(cxs, cx_id)` at seq N → `Some(header)`.
- [ ] Assert all three reads return `seq=N` (same MVCC sequence).
- [ ] Deadlock check: from a second thread, call `CrossModelTxn::begin` with
  `timeout=50ms` while first txn is still `Active` → `CALYX_TXN_TIMEOUT`.
  After first txn commits, second `begin` succeeds within 50 ms.

### Scenario B — Cross-model query, one provenanced result set
- [ ] Build a `UniversalQuery` with:
  - `relational: Some(RelationalFilter { collection: "orders", predicate: qty>=1 })`
  - `kv: Some(KvLookup { ns=1, key=b"last_order" })`
  - `vector: Some(VectorQuery { lens_ids: [stub], query: "order", limit: 5 })`
  - `cost_cap_ms: Some(10_000)`
  - `explain: true`
- [ ] `plan(vault, query)` → `CrossModelPlan` with steps ≥ 2.
- [ ] `execute(vault, plan)` → `QueryResult`:
  - Assert `rows` contains entries from both relational and KV.
  - Assert all rows were read at the same `snapshot_seq`.
  - Assert `explain.steps` lists each mode with a non-zero cost.
- [ ] Confirm one-pass: measure elapsed_ms ≤ 10_000 (the cost cap).

### Scenario C — Unbounded plan rejection
- [ ] Build `UniversalQuery { relational: Some(full-scan on a 1M+ row collection),
  cost_cap_ms: None }`.
- [ ] `plan(vault, query)` → `Err(CALYX_PLANNER_COST_CAP)` with the estimated
  cost in the error message.
- [ ] Assert NO read was executed (executor was never called).

### Scenario D — `ASK` returns one provenanced result
- [ ] `UniversalQuery { ask: Some(AskSpec { question: "test question", top_k: 1, oracle: false }), cost_cap_ms: Some(5_000) }`.
- [ ] `execute` → `QueryResult.rows` has ≥1 row with `ledger_ref` field populated.
- [ ] Print the `ledger_ref` hex to stdout (FSV evidence).

- [ ] All assertions `assert_eq!` with exact expected values; seed RNG `42`;
  inject `Clock` with fixed timestamp.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] This test is the phase FSV gate; 100% deterministic and self-contained.
- [ ] All four scenarios must pass in sequence.
- [ ] Print "ph55 FSV: A=PASS B=PASS C=PASS D=PASS" on success.
- [ ] fail-closed: any `Err` result prints the `CALYX_*` code and fails the test
  with a descriptive message.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** SST shards in `/tmp/calyx-ph55-fsv-test/cf/`; test stdout.
- **Readback:**
  ```
  cargo test -p calyx-sextant ph55_fsv -- --nocapture 2>&1 | tail -40
  calyx readback --cf relational --vault /tmp/calyx-ph55-fsv-test --show-seq
  calyx readback --cf kv         --vault /tmp/calyx-ph55-fsv-test --show-seq
  calyx readback --cf slot_00    --vault /tmp/calyx-ph55-fsv-test --show-seq
  ```
- **Prove:**
  - Test prints "ph55 FSV: A=PASS B=PASS C=PASS D=PASS" and exits 0.
  - All three `readback --show-seq` outputs show `seq=N` for the same N (Scenario A).
  - Scenario C error message contains `CALYX_PLANNER_COST_CAP` and a cost estimate.
  - Scenario D result contains a non-empty `ledger_ref` hex string.
  - Screenshot posted to PH55 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (test output + readback screenshots) attached to the PH55 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
