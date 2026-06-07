# PH55 · T01 — `CrossModelTxn`: single-writer serialization, declared isolation, cost cap

| Field | Value |
|---|---|
| **Phase** | PH55 — Cross-model transactions + universal query surface |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/txn/mod.rs` (≤500), `crates/calyx-aster/src/txn/cross_model.rs` (≤500) |
| **Depends on** | PH09 (WAL group-commit, WriteBatch), PH08 (MVCC seq), PH53 T01 (TxnPolicy) |
| **Axioms** | A15, A16, A19 |
| **PRD** | `dbprdplans/03 §8`, `dbprdplans/20 §4/§8` |

## Goal

Implement `CrossModelTxn` — a single `WriteBatch` that can accumulate writes
to any combination of paradigm-layer CFs (relational, document, KV, TS, blob,
constellation) and commit them atomically at one MVCC sequence number. A
per-vault `TxnHandle` enforces single-writer-per-vault serialization: only one
`CrossModelTxn` may be in the `Active` state per vault at a time; a second
`begin` blocks (with a configurable timeout) until the first commits or rolls
back. Declared isolation (`ReadCommitted | Serializable`) and a `cost_cap_ms`
are set at `begin`; the cap is checked against an elapsed timer before each
`commit`; a plan exceeding the cap → `CALYX_TXN_COST_CAP`.

## Build (checklist of concrete, code-level steps)

- [ ] Define `TxnHandle` in `txn/mod.rs`:
  ```rust
  pub struct TxnHandle {
      vault_id: VaultId,
      mutex: Arc<Mutex<TxnState>>,
  }
  pub enum TxnState { Idle, Active { started_at: Instant, cost_cap_ms: Option<u32> } }
  ```
- [ ] Implement `TxnHandle::begin(isolation: IsolationLevel, cost_cap_ms: Option<u32>, timeout: Duration) -> Result<CrossModelTxn>`:
  - Try to acquire `vault_id` mutex; if busy, wait up to `timeout` then
    `CALYX_TXN_TIMEOUT`.
  - Set `TxnState::Active`; return a `CrossModelTxn` borrowing the handle.
- [ ] Define `CrossModelTxn` in `txn/cross_model.rs`:
  ```rust
  pub struct CrossModelTxn<'h> {
      handle: &'h TxnHandle,
      batch: WriteBatch,
      snapshot_seq: Seq,
  }
  ```
- [ ] Implement `CrossModelTxn::put_record`, `put_doc`, `kv_set`, `ts_write`,
  `blob_put_chunk`, `put_constellation`:
  - Each appends its data key + index keys (via `IndexMaintenance::on_put`) to
    `self.batch`. Does NOT submit.
- [ ] Implement `CrossModelTxn::commit(vault: &AsterVault) -> Result<Seq>`:
  - Check elapsed time: if `elapsed > cost_cap_ms` → rollback + `CALYX_TXN_COST_CAP`.
  - Submit `self.batch` via `vault.submit_batch` (WAL group-commit + memtable).
  - Advance MVCC seq; release `TxnState` to `Idle`.
  - Return the committed `Seq`.
- [ ] Implement `CrossModelTxn::rollback()`:
  - Discard `self.batch`; set `TxnState::Idle`. No WAL entry (nothing was submitted).
- [ ] Enforce: a `CrossModelTxn` dropped without `commit` or `rollback` calls
  `rollback` implicitly (implement `Drop`).
- [ ] `IsolationLevel::Serializable`: read-your-writes within the txn (read from
  `self.batch` overlay before hitting the CF). `ReadCommitted`: read the last
  durable seq (no txn overlay on reads).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `begin` → `put_record` + `kv_set` → `commit` → read both from vault
  at the returned seq → both present with the same seq number.
- [ ] unit serialization: `begin` from thread A (holding mutex); `begin` from
  thread B with `timeout=10ms` → thread B gets `CALYX_TXN_TIMEOUT`; thread A
  `commit` → thread B retries and succeeds.
- [ ] unit cost cap: `begin(cost_cap_ms=Some(1))` → sleep 2 ms → `commit` →
  `CALYX_TXN_COST_CAP`; vault unchanged (rollback applied).
- [ ] unit rollback: `put_record(pk=99)` → `rollback()` → `get_record(pk=99)` →
  `None`.
- [ ] proptest: N sequential `CrossModelTxn`s each touching all 5 plain modes →
  all `commit`; each key readable at its committed seq; no seq gaps.
- [ ] edge (≥3): (1) empty txn (`commit` with no writes) → succeeds, seq advances;
  (2) `Drop` without commit → implicit rollback, no partial write; (3)
  `Serializable` reads uncommitted overlay before commit → returns the
  in-progress value.
- [ ] fail-closed: WAL submit fails (injected `Err`) → `commit` returns `Err`;
  `TxnState` reset to `Idle`; no partial data visible.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cf/relational/`, `cf/kv/`, `cf/slot_00/` SST shards — all at same
  seq after a cross-model commit.
- **Readback:**
  ```
  calyx txn begin   --vault /home/croyse/calyx/test-vault --isolation serializable --cost-cap 500ms
  calyx txn put-record --collection orders --pk 10 --data '{"qty":1}'
  calyx txn kv-set  --ns 1 --key sess --val active
  calyx txn commit
  calyx readback --cf relational --vault /home/croyse/calyx/test-vault --show-seq
  calyx readback --cf kv         --vault /home/croyse/calyx/test-vault --show-seq
  ```
- **Prove:** Both `--show-seq` outputs display `seq=N` for the same N.
  Cost-cap test: `begin --cost-cap 1ms` + `sleep 5ms` + `commit` → prints
  `CALYX_TXN_COST_CAP`; vault unchanged. Evidence posted to PH55 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH55 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
