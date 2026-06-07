# PH35 · T07 — Integration smoke: PH09 constellation write → chained ledger entry in WAL

| Field | Value |
|---|---|
| **Phase** | PH35 — Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` (+ `calyx-aster` integration test) |
| **Files** | `crates/calyx-ledger/src/tests/integration_smoke.rs` (≤500) |
| **Depends on** | T05, T06 (this phase) · PH09 |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §1`, `11 §6`, `04 §5` |

## Goal

Run a complete end-to-end smoke test: write N constellations through the PH09
path (with the group-commit hook active), then read back the WAL and the
`ledger` CF to prove (1) every write produced a chained entry, (2) the chain
links are byte-exact, and (3) no entry stores a secret value. This is the
primary FSV evidence for the PH35 GitHub issue.

## Build (checklist of concrete, code-level steps)

- [ ] `fn smoke_ingest_n_constellations(n: usize, vault_dir: &Path) -> Vec<LedgerRef>` —
  opens an in-process vault with the ledger hook wired, writes `n` synthetic
  `Constellation` values via the PH09 `IngestWriter`, returns the `LedgerRef`
  for each.
- [ ] After writing, call `LedgerAppender::open` on the same CF and iterate all
  rows; verify the chain: `assert_eq!(entry[i].prev_hash, entry[i-1].entry_hash)`
  for all `i > 0`; assert `entry[0].prev_hash == [0u8; 32]`.
- [ ] Read the WAL binary with `WalReader` (PH05 type); find each commit record;
  assert that every commit record containing a base-CF write also contains a
  ledger-CF write at the same commit group offset.
- [ ] Assert no ledger payload contains the string `"secret"`, `"password"`,
  or `"token"` (inline heuristic scan of decoded payload bytes).
- [ ] Run the smoke test for `n=1`, `n=5`, and `n=100` within the same test
  function (parameterized); use a fixed `MockClock(start=1_785_000_000)`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `n=1` — single constellation write → ledger entry seq=0,
  prev_hash=[0;32], kind=Ingest; `entry_hash` matches golden (hard-coded 32-byte
  constant derived from the fixed synthetic constellation bytes).
- [ ] unit: `n=5` — chain intact for all 5; `LedgerRef` returned from each
  `IngestWriter::put` matches the row stored in CF.
- [ ] unit (crash-recovery): write 3 entries, simulate crash (drop vault without
  flush), reopen → WAL replay recovers all 3 entries; chain intact after replay.
- [ ] edge (≥3): `n=0` (empty write, no entries — no error); duplicate
  constellation bytes (idempotent re-ingest from PH09) → second ingest still
  emits a new ledger entry (idempotent data write, non-idempotent provenance
  record); `n=100` entries written in one batch.
- [ ] fail-closed: corrupt a WAL record mid-batch (flip one byte in the ledger
  sub-record) → WAL replay returns `CALYX_WAL_TORN_RECORD` and the torn entries
  are discarded (no partial ledger entries admitted).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ledger` CF rows + WAL binary after running the smoke test on aiwonder
- **Readback:**
  1. `cargo test -p calyx-ledger -- --nocapture integration_smoke 2>&1` — must
     print `"chain OK: 100 entries, all links verified"`.
  2. `calyx readback --vault smoke_test --cf ledger --range 0..5` — prints seq
     0–4 with `prev_hash` / `entry_hash`; confirm linkage manually.
  3. `xxd $CALYX_HOME/vault/smoke_test/wal/wal-0.bin | grep -c "ledger"` —
     count must equal `n` (one ledger write per group-commit).
- **Prove:** before: no ledger rows in WAL; after: `n` rows in `ledger` CF;
  WAL contains ledger entries co-located with data entries; chain verification
  passes; no secret values in payloads.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH35 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
