# PH09 · T04 — Ledger stub entry in group commit

| Field | Value |
|---|---|
| **Phase** | PH09 — Constellation CRUD + CxId + idempotent ingest |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault/ledger_stub.rs` (≤500), `crates/calyx-aster/src/vault.rs` (≤500) |
| **Depends on** | T02 (WAL-integrated write path) |
| **Axioms** | A15 |
| **PRD** | `dbprdplans/04 §5`, `dbprdplans/11 §1` (Ledger, PH35 stub) |

## Goal

Every `put` that advances the vault seq must write a Ledger stub entry to the
`ledger` CF as part of the same `commit_batch` so the Ledger row is durable
before the ack. The stub is `seq → [0u8; 32]` (32 zero bytes) — PH35 replaces
this with a real blake3 hash-chain entry. This satisfies DOCTRINE A15
(provenance always) at the scaffold level and ensures PH35 can rely on the
`ledger` CF being populated for every write.

## Build (checklist of concrete, code-level steps)

- [x] In `vault/ledger_stub.rs`: define `fn write_ledger_stub(seq: Seq) ->
  (ColumnFamily, Vec<u8>, Vec<u8>)` returning `(CF::Ledger, ledger_key(seq),
  [0u8; 32].to_vec())`.
- [x] In `AsterVault::put`, after building the CF row list and before calling
  `commit_batch`, append the ledger stub row to the batch.
- [x] The WAL payload must include the ledger stub row (so recovery in PH10 also
  sees it).
- [x] Write a test: `put(cx)` → `flush_all_cfs()` → `CfRouter::get(Ledger,
  ledger_key(seq))` returns `Some([0u8; 32])`.
- [x] Write a test: ledger seq matches the MVCC seq of the write: after one put,
  `ledger` CF contains exactly one row at `seq = 1`; after a second put, at `seq = 2`.
- [x] Add a `// FIXME(PH35): replace stub with blake3 hash-chain entry` comment.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: put N constellations → ledger CF has N rows, seqs [1..N], all values
  are `[0u8; 32]`.
- [x] unit: ledger row is included in WAL payload (decode the WAL record and
  confirm the ledger CF tag is present in the write batch encoding).
- [x] unit: cold-open vault → ledger CF readable from SST.
- [x] edge (≥3): (1) idempotent put → ledger CF row count unchanged; (2) anchor
  write (no new constellation seq) → ledger CF row count unchanged (anchor does
  not write a new ledger entry at this stage); (3) ledger key ordering: seq 100
  sorts after seq 99.
- [x] fail-closed: if `write_ledger_stub` panics or returns Err, the whole
  `commit_batch` is aborted (no partial write).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Ledger CF SST at `/home/croyse/calyx/test-vault/cf/ledger/000001.sst`.
- **Readback:**
  ```
  calyx readback --cf ledger --vault /home/croyse/calyx/test-vault
  xxd /home/croyse/calyx/test-vault/cf/ledger/000001.sst | head -4
  ```
- **Prove:** After ingesting N constellations and flushing, `calyx readback` shows
  N ledger rows with keys `[0,0,0,0,0,0,0,1]` through `[0,0,0,0,0,0,0,N]` (big-
  endian u64 seqs) and values `[0x00 * 32]`. Screenshot posted to PH09 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH09 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
