# PH55 - T05 - FSV: cross-model transaction + universal query gate

| Field | Value |
|---|---|
| Phase | PH55 - Cross-model transactions + universal query surface |
| Stage | S12 - Universal data layer |
| Crates | `calyx-aster`, `calyx-sextant` |
| Issue | #467 |
| Status | Implemented and FSV-proven on aiwonder |

## Implemented

- Added the PH55 integration gate in `crates/calyx-sextant/tests/ph55_fsv.rs`.
- Kept test files under the hard 500-line gate by moving helpers into:
  - `crates/calyx-sextant/tests/ph55_fsv/support.rs`
  - `crates/calyx-sextant/tests/ph55_fsv/artifact.rs`
- Fixed `AsterVault::write_cf_batch_with_ledger_entry` so ledgered batches stamp any Base CF constellation rows with the staged ledger ref before commit.
- Updated Sextant vector fusion so Cx candidate rows are narrowed/replaced, while unrelated relational/KV rows remain in the same universal result set.

## Scenarios

### A - Atomic cross-model transaction

The FSV creates `orders`, `kv_state`, and `cxs`, then commits one transaction containing:

- `orders[pk=1] = { item: "order #1 placed", qty: 7 }`
- `kv_state(ns=1, key="last_order") = "1"`
- one constellation for `order #1 placed`

The readback proves:

- before commit: relational, KV, Base, and `slot_00` rows are absent at seq 3
- after commit: all four rows are visible at seq 4
- a competing begin while the first txn is active returns `CALYX_TXN_TIMEOUT`
- after commit, a new begin succeeds
- the transaction-written constellation has a non-zero ledger hash

### B - One cross-model query result set

The query plans and executes a single `UniversalQuery` with:

- relational filter `qty >= 1`
- KV lookup `kv_state:1 / last_order`
- graph hop from the transaction CxId
- vector fusion over the stub lens

The result set contains relational, KV, and vector/Cx rows in one pass. The vector row carries the transaction constellation ledger ref read from the Base CF row.

### C - Unbounded plan rejection

The FSV asks the planner for an estimated 1,000,000-row full scan with no explicit cap. Planning fails before execution with:

`CALYX_PLANNER_COST_CAP`

The before/after sequence remains unchanged.

### D - ASK provenanced result

The FSV ingests a second provenanced constellation through the normal durable vault path and runs an ASK query with `top_k=1`. The single returned row carries the stored ledger ref.

## aiwonder Evidence

Evidence root:

`/home/croyse/calyx/data/fsv-issue467-ph55-20260614T163258Z`

Readback artifact:

`/home/croyse/calyx/data/fsv-issue467-ph55-20260614T163258Z/issue467-ph55-fsv-readback.json`

Manual SoT reads performed on aiwonder:

- `cat issue467-ph55-fsv-readback.json`
- `find .../vault -type f | sort`
- `sha256sum` over the readback JSON, relational/KV/Base/slot_00/Ledger SSTs, and WAL
- `xxd` over the commit-seq relational, KV, Base, `slot_00`, Ledger SSTs, and WAL

Key observed values:

- `ph55 FSV: A=PASS B=PASS C=PASS D=PASS`
- Scenario A commit seq: `4`
- Scenario A before seq: `3`
- Scenario A after rows: relational seq `4`, KV seq `4`, Base seq `4`, `slot_00` seq `4`
- Transaction constellation ledger hash: `cc91f867c30ac201ab7203205ce534ee0e425d2f90090df1eb779d20ce56e11c`
- Scenario C exact error: `CALYX_PLANNER_COST_CAP`
- Scenario D ASK ledger hash: `c5ce018cb5d951f1c618911b5b92db9f616121fcddf0b1ec22ca2fdf60c8e13e`

## Gates

Passed on aiwonder:

- `cargo fmt --all -- --check`
- line-count gate:
  - `crates/calyx-sextant/tests/ph55_fsv.rs` - 67
  - `crates/calyx-sextant/tests/ph55_fsv/support.rs` - 487
  - `crates/calyx-sextant/tests/ph55_fsv/artifact.rs` - 74
  - `crates/calyx-sextant/src/query/executor.rs` - 480
  - `crates/calyx-aster/src/vault/layer_commit.rs` - 106
- `cargo check -p calyx-sextant`
- `cargo clippy -p calyx-sextant --all-targets -- -D warnings`
- `cargo test -p calyx-sextant --lib query:: -- --nocapture`
- `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue467-ph55-20260614T163258Z cargo test -p calyx-sextant --test ph55_fsv -- --nocapture`
- `cargo test -p calyx-aster txn:: -- --nocapture`
- `cargo test -p calyx-aster vault::ledger -- --nocapture`
