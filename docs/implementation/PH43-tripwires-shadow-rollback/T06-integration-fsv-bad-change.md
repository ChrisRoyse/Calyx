# PH43 - T06 - Integration: bad-change auto-revert FSV scenario

| Field | Value |
|---|---|
| **Phase** | PH43 - Tripwires + Shadow-First + Reversible/Rollback |
| **Stage** | S10 - Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/integration_fsv.rs` (<=500) + `crates/calyx-anneal/tests/fsv_bad_change.rs` (<=500) + `crates/calyx-anneal/tests/support/fsv_bad_change.rs` (<=500) |
| **Depends on** | T01, T02, T03, T05 |
| **Axioms** | A14, A15, A16 |
| **PRD** | `dbprdplans/12 A-6`, `dbprdplans/27 A-4` |

## Goal

Wire `TripwireRegistry` + `ShadowExecutor` + `RollbackStore` + `AnnealLedger`
into the `AnnealSubstrate` facade and prove the full safety loop end-to-end:
a deliberately bad change triggers the tripwire in shadow, the revert fires
before promotion touches the live path, the Ledger records the revert entry,
and the live config pointer is byte-identical to the prior artifact.

## Build

- [x] `AnnealSubstrate` facade added in `crates/calyx-anneal/src/integration_fsv.rs`.
- [x] `propose_change` and `propose_change_with_description` prepare a rollback snapshot, run shadow, and return `ChangeOutcome`.
- [x] Promotion path writes Ledger `Promote` before `rollback.promote`, so `CALYX_LEDGER_WRITE_FAIL` leaves the live pointer unchanged.
- [x] Revert path calls `rollback.rollback`, writes Ledger `Revert`, and returns `ChangeOutcome::Reverted`.
- [x] `rollback_explicit` restores the prior pointer and writes a second Ledger `Revert` for the same `change_id`.
- [x] `status` reports tripwire states, budget status, and recent Anneal ledger entries.
- [x] Clocks are injected and tests use `FixedClock`; synthetic actions are deterministic.

## Tests

- [x] bad-recall candidate -> `ChangeOutcome::Reverted`; Ledger has `Revert`; live ptr unchanged.
- [x] good candidate -> `ChangeOutcome::Promoted`; Ledger has `Promote`; live ptr updates to candidate.
- [x] explicit rollback after promotion -> live ptr back to prior; Ledger has second `Revert`.
- [x] budget exhausted -> `ChangeOutcome::Reverted { reason: BudgetExhausted }`; no promote row.
- [x] ledger write failure before promotion -> `CALYX_LEDGER_WRITE_FAIL`; live ptr unchanged.

## FSV

- **Fresh evidence root:** `/home/croyse/calyx/data/fsv-issue399-bad-change-20260610-1940`
- **Discarded root:** `/home/croyse/calyx/data/fsv-issue399-bad-change-20260610-1930` was invalidated by an operator-side verification command mistake and is not used as evidence.
- **SoT:** Aster `ledger` CF rows, Aster `anneal_rollback` CF rows, WAL bytes, Ledger chain verification, and physical SST hexdumps on aiwonder.
- **Trigger:** `CALYX_ISSUE399_FSV_ROOT=/home/croyse/calyx/data/fsv-issue399-bad-change-20260610-1940 cargo test -p calyx-anneal --test fsv_bad_change fsv_bad_change_aiwonder -- --ignored --nocapture`
- **Manual readback:** `audit`, `scan --cf ledger`, `readback --cf ledger`, `readback --cf anneal_rollback`, `readback --wal`, `verify-chain`, `merkle-root`, `find -ls`, `xxd`, and `b3sum` were run separately against the SoT.
- **Observed:** bad recall wrote one Ledger `Revert` and live ptr stayed `0x11`; good candidate wrote `Promote` and live ptr moved to `0x22`; explicit rollback wrote second `Revert` and live ptr returned to `0x11`; budget exhaustion wrote only `Revert`; injected ledger failure returned `CALYX_LEDGER_WRITE_FAIL` with `promoted=false` and live ptr still `0x11`.

## Done

- [x] `cargo fmt --check`
- [x] `bash scripts/linecount.sh`
- [x] `git diff --check`
- [x] `cargo check -p calyx-anneal --quiet`
- [x] `cargo test -p calyx-anneal --quiet`
- [x] `cargo clippy -p calyx-anneal --tests --quiet -- -D warnings`
- [x] `cargo check -p calyx-cli --quiet`
- [x] FSV evidence captured at the fresh evidence root and summarized in GitHub issue #399.
