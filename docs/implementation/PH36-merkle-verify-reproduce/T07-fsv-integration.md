# PH36 T07 - FSV integration: flip-byte tamper test + reproduce bit-parity test

| Field | Value |
|---|---|
| **Phase** | PH36 - Merkle checkpoints + verify_chain + reproduce() |
| **Stage** | S7 - Ledger Provenance |
| **Crate** | `calyx-ledger`, `calyx-cli` |
| **Files** | `crates/calyx-cli/tests/ph36_fsv_integration.rs`, `crates/calyx-cli/tests/support/ph36_fsv/*`, `scripts/fsv_ph36.sh` |
| **Depends on** | T02, T05, T06 |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11` sections 3, 5, 7 |
| **Status** | Done for #255 on aiwonder |

## Goal

Produce the two byte-level FSV proofs required by the PH36 exit gate and attach
them as evidence on GitHub issue #255. These are storage readbacks, not harness
claims: the test flips real Aster ledger-CF bytes at seq 11, proves
`verify_chain` detects and quarantines the break, then replays a synthetic
answer through `reproduce_with_input_resolver` and proves score byte parity.

## Build

- [x] `ph36_fsv_integration_aiwonder` writes 20 durable Aster ledger entries.
- [x] The target sequence is deterministic: `target_seq = 11`.
- [x] The FSV helper reads the raw ledger-CF SST rows and flips byte offset 8 in
      the seq 11 value.
- [x] `calyx verify-chain --vault <vault> --range 0..20` returns
      `CALYX_LEDGER_CHAIN_BROKEN at seq=11`.
- [x] Aster manifest quarantine records `range_start=0`, `range_end=20`,
      `broken_at_seq=11`, and `is_quarantined(11)=true`.
- [x] `calyx get-provenance` and `calyx readback --cf ledger --seq 11` fail
      closed with `CALYX_LEDGER_CHAIN_BROKEN` after quarantine.
- [x] `run_reproduce_fsv` writes two Measure rows, one Answer row, then one
      Admin `reproduce_v1` row with `reproduced=true` and `max_drift=0.0`.
- [x] Original and reproduced score bytes are printed in the JSON readback.
- [x] The FSV test is ignored for normal unit runs and invoked by
      `scripts/fsv_ph36.sh`.
- [x] `scripts/fsv_ph36.sh` captures `ph36-fsv.log`, writes `ph36-fsv.log.xxd`,
      and fails non-zero if the expected tamper/reproduce summary lines are
      missing.

## Tests

- [x] Unit edge: tolerance accepts sub-millidrift and rejects drift over
      `1e-3`.
- [x] Unit edge: intact 20-row chain returns `VerifyResult::Intact { count: 20 }`.
- [x] Unit edge: flipping a seq 0 row reports `Broken { at_seq: 0 }`.
- [x] Unit edge: flipping the final `entry_hash` field at seq 11 reports
      `Broken { at_seq: 11 }`.
- [x] Ledger audit regression: provenance/answer-trace check quarantine before
      returning rows, and #349 makes filtered `audit()` result-set-aware:
      explicit `seq_range` overlap or matching/relevant quarantined rows fail
      closed, while unrelated quarantined rows outside the filtered result set
      do not poison the query.
- [x] Ignored aiwonder FSV: flip-byte tamper at seq 11.
- [x] Ignored aiwonder FSV: reproduce bit-parity with fixed `0xDEAD_BEEF`
      Forge seed and `max_drift=0.0`.

## FSV Evidence

- **aiwonder root:**
  `/home/croyse/calyx/data/fsv-issue255-ph36-integration-20260609`
- **Readback JSON:**
  `/home/croyse/calyx/data/fsv-issue255-ph36-integration-20260609/ph36-exit-fsv/ph36-fsv-integration-readback.json`
- **Readback JSON SHA-256:**
  `006ef67bdb9db189b1142c6d4bb45c1181f8b6b31d1fb2cd8a51392553993fea`
- **FSV log SHA-256:**
  `9ca1c532d305c8b45f2141e7bb5513c7e03796ca03d56ac9b717085fe02eb403`
- **FSV log xxd SHA-256:**
  `e54e93b614538e45b03e8914e24cdfa31981c02923fd70031c7dcbf108862cff`

Manual SoT readback on aiwonder proved:

- The FSV summary printed
  `PH36 FSV PASS: tamper detected at seq=11; reproduce max_drift=0.000000`.
- The tamper manifest contains one quarantine with `range_start=0`,
  `range_end=20`, `broken_at_seq=11`.
- `calyx readback --cf ledger --vault <tamper-vault> --seq 11` returns
  `CALYX_LEDGER_CHAIN_BROKEN: ledger seq 11 is quarantined`.
- The tampered SST bytes contain the seq 11 prefix
  `000000000000000b187621a625d6774cef7989f28bbbd3ef98e9db7a7f5377e0`;
  the recorded before prefix differed at offset 8:
  `000000000000000b197621a625d6774cef7989f28bbbd3ef98e9db7a7f5377e0`.
- The reproduce ledger has four rows: Measure seq 0, Measure seq 1, Answer seq
  2, Admin seq 3. The Admin payload contains `type=reproduce_v1`,
  `reproduced=true`, and `max_drift=0.0`.
- Original score bytes equal reproduced score bytes:
  `4f71c93c`, `8c31c63c`.
- The reproduce ledger chain readback is intact with `count=4`.

## Done

- [x] `cargo fmt --check` green on aiwonder.
- [x] `cargo check -p calyx-core -p calyx-ledger -p calyx-cli` green on aiwonder.
- [x] `cargo test -p calyx-core`, `cargo test -p calyx-ledger`, and
      `cargo test -p calyx-cli` green on aiwonder.
- [x] `cargo clippy -p calyx-core -p calyx-ledger -p calyx-cli --all-targets -- -D warnings`
      green on aiwonder.
- [x] `scripts/linecount.sh` green; every touched `.rs` file is at most 500
      lines.
- [x] Diff secret scan clean.
- [x] No PH36 anti-pattern: no quarantined row is served, no silent drift, no
      frozen-lens mutation, and no harness-only FSV verdict.
