# PH35 T06 - Actor-stamp + server-stamped monotonic timestamp wiring

| Field | Value |
|---|---|
| **Phase** | PH35 - Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 - Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/append.rs` (<=500) |
| **Depends on** | T05 (this phase), PH04 (`Clock` trait in `calyx-core`) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11` section 2 |
| **Status** | DONE / FSV-signed-off in #247 |

## Goal

Ensure every `LedgerEntry` carries a verifiable `actor` (who or what caused
the mutation) and a server-stamped, monotonically increasing `ts` (never
client-supplied). The `actor` field identifies the `AgentId`, `ServiceId`, or
system actor responsible; the `ts` comes from the injected `Clock` trait, not
`SystemTime::now()` in ledger logic. Monotonicity is enforced at the appender
level so sequence ordering and timestamp ordering are consistent.

## Build

- [x] `ActorId` in `entry.rs`: tagged enum `AgentId(String)` |
  `ServiceId(String)` | `System`; max 64-byte UTF-8 for the inner string;
  return `CALYX_LEDGER_ACTOR_TOO_LONG` if over limit.
- [x] Add `CALYX_LEDGER_ACTOR_TOO_LONG` to the error catalog with remediation
  `"actor id must be <= 64 bytes UTF-8"`.
- [x] `LedgerAppender` stores `last_ts: u64` and uses
  `next_ts(&self) -> Result<u64>` to call the injected `Clock::now()`. If the
  clock value is `<= self.last_ts`, the appender uses `self.last_ts + 1` and
  fails closed only on timestamp exhaustion.
- [x] `append` computes `ts`; callers provide `actor` but never provide `ts`.
- [x] `LedgerAppender::open` recovers `last_ts` from the last row's `ts` so
  monotonicity survives restarts.
- [x] `ActorId::validate(&self) -> Result<()>` checks UTF-8 byte length `<= 64`.

## Tests

- [x] Unit: injected clock returns 1000, 1000, 1001; three appends produce
  timestamps 1000, 1001, 1002.
- [x] Unit: appender restarts after recovered `ts=5000`; injected clock returns
  4999; first new entry has `ts=5001`.
- [x] Proptest: any generated clock sequence preserves strictly increasing
  ledger entry timestamps.
- [x] Edge cases: empty actor string passes, 64-byte actor string passes,
  65-byte actor string returns `CALYX_LEDGER_ACTOR_TOO_LONG` and writes no row.
- [x] Fail-closed recovery edge: recovered `ts=0` still clamps forward to `1`
  on the next append.

## FSV

- **SoT:** `ledger` CF rows, the Aster WAL, and ledger SST bytes on aiwonder.
- **Evidence root:** `/home/croyse/calyx/data/fsv-issue247-ledger-actor-ts-20260608`
- **Trigger:** `CALYX_FSV_ROOT=<root> cargo test -p calyx-aster ph35_actor_monotonic_ts_aiwonder_fsv -- --ignored --nocapture`
- **Readback:** `actor-monotonic-ts/ledger-actor-ts-readback.json` shows the
  pre-read ledger row absent, rows 0..2 decoded with actor
  `ServiceId("calyx-aster")`, `actors_non_empty=true`, and
  `timestamps_strictly_increase=true`.
- **Issue scan:** `actor-monotonic-ts/09-issue-scan-jq.out` records compact
  `[seq, ts, actor]` rows matching the #247 acceptance wording.
- **Byte proof:** `actor-monotonic-ts/03-ledger-cf-readback.out`,
  `04-wal-readback.out`, `06-wal-prefix.hex`, and `07-ledger-sst-prefix.hex`
  contain the physical ledger CF, WAL, and SST bytes.
- **Gates:** `final-gates/10-fmt-check.out` through `14-linecount.out` record
  aiwonder fmt, check, test, clippy, and line-count gates.

## Done

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] File(s) <= 500 lines.
- [x] FSV evidence attached to issue #247.
- [x] No anti-pattern: no flattening, no `C(N,2)` past DPI, no ungrounded
  trusted claim, no frozen-lens mutation, and no harness-as-FSV.
