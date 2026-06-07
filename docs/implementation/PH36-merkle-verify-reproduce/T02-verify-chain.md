# PH36 · T02 — `verify_chain(range)` + `CALYX_LEDGER_CHAIN_BROKEN` + quarantine

| Field | Value |
|---|---|
| **Phase** | PH36 — Merkle checkpoints + verify_chain + reproduce() |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/verify.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH35-T02 (binary codec / `decode_header`) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §5`, `11 §7` |

## Goal

Implement `verify_chain(vault, range)` — the tamper detection path. It walks
every ledger entry in `[seq_a, seq_b)`, re-verifies `entry_hash = blake3(seq ‖ prev_hash ‖ kind ‖ subject ‖ payload ‖ actor ‖ ts)` and checks each `prev_hash`
equals the previous entry's `entry_hash`. On the first discrepancy it returns
`CALYX_LEDGER_CHAIN_BROKEN` with the exact `seq` of the broken link and writes
a quarantine marker to the vault manifest (fail-closed). It never silently
continues past a broken link.

## Build (checklist of concrete, code-level steps)

- [ ] `pub enum VerifyResult { Intact { count: u64 }, Broken { at_seq: u64, expected: [u8;32], found: [u8;32] } }`
- [ ] `pub fn verify_chain(cf_reader: &dyn LedgerCfReader, range: KeyRange) -> Result<VerifyResult>` —
  iterates entries in ascending seq order; for each entry: (a) re-compute
  `entry_hash` via `LedgerEntry::verify()`, (b) check `entry.prev_hash ==
  prev_entry_hash`; on first failure return `VerifyResult::Broken { at_seq, … }`.
- [ ] `CALYX_LEDGER_CHAIN_BROKEN` added to `calyx-core/src/error.rs` with
  remediation `"ledger chain integrity violation — affected range quarantined; do not serve results from this range"`.
- [ ] On `VerifyResult::Broken`, write a quarantine record to the vault
  manifest (not to the `ledger` CF): `QuarantineRecord { range_start, range_end, broken_at_seq, detected_at_ts }`.
  Subsequent reads from the quarantined range must return `CALYX_LEDGER_CHAIN_BROKEN`
  immediately (checked in the read path).
- [ ] `pub fn is_quarantined(manifest: &VaultManifest, seq: u64) -> bool` —
  returns `true` if `seq` falls in any quarantined range; used by the read path.
- [ ] The genesis invariant: entry at `seq=0` must have `prev_hash == [0u8;32]`;
  if not, return `VerifyResult::Broken { at_seq: 0, … }`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: build a chain of 10 valid entries (using `LedgerAppender`);
  `verify_chain(0..10)` returns `VerifyResult::Intact { count: 10 }`.
- [ ] unit: build a chain of 10 entries; corrupt `entry[5]`'s `prev_hash`
  (flip byte 0); `verify_chain(0..10)` returns `VerifyResult::Broken { at_seq: 5 }`.
- [ ] unit: corrupt `entry[5]`'s `entry_hash` (not `prev_hash`) — the error
  shows up at `seq=6` (since `entry[6].prev_hash` won't match the original);
  assert `at_seq == 6`.
- [ ] unit: quarantine record written after a broken chain → `is_quarantined(6)`
  returns `true`; read of any seq in `[5,10)` returns `CALYX_LEDGER_CHAIN_BROKEN`.
- [ ] edge (≥3): empty range → `Intact { count: 0 }`; single entry (genesis) →
  `Intact { count: 1 }`; `seq=0` with wrong `prev_hash` → `Broken { at_seq: 0 }`.
- [ ] fail-closed: CF reader returns an I/O error mid-walk → propagate as
  `CALYX_ASTER_IO_ERROR` (not silently treated as intact); verify no partial
  quarantine is written on I/O error.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ledger` CF + vault manifest quarantine record on aiwonder after the
  flip-byte tamper test
- **Readback:**
  1. Write 20 entries via `smoke_ingest_n_constellations(20)`.
  2. Flip one byte in the raw ledger CF row for seq=7:
     `calyx raw-edit --vault test --cf ledger --seq 7 --byte-offset 8 --flip`.
  3. `calyx verify-chain --vault test --range 0..20` →
     must print `CALYX_LEDGER_CHAIN_BROKEN at seq=7` (exactly seq=7).
  4. `calyx readback --vault test --cf ledger --seq 8` →
     must return `CALYX_LEDGER_CHAIN_BROKEN` (quarantine active).
- **Prove:** the broken-at seq matches the seq of the corrupted entry (not an
  off-by-one); the quarantined range blocks reads; intact chains report
  `Intact { count: 20 }`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH36 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
