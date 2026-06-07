# PH35 · T06 — Actor-stamp + server-stamped monotonic timestamp wiring

| Field | Value |
|---|---|
| **Phase** | PH35 — Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/append.rs` (≤500) |
| **Depends on** | T05 (this phase) · PH04 (`Clock` trait in `calyx-core`) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §2` |

## Goal

Ensure every `LedgerEntry` carries a verifiable `actor` (who or what caused
the mutation) and a server-stamped, monotonically increasing `ts` (never
client-supplied). The `actor` field identifies the `AgentId` or `ServiceId`
responsible; the `ts` comes from the `Clock` trait injected at startup (not
`SystemTime::now()` in logic). Monotonicity is enforced at the appender level
so sequence ordering and timestamp ordering are consistent.

## Build (checklist of concrete, code-level steps)

- [ ] `ActorId` in `entry.rs`: tagged enum
  `AgentId(String)` | `ServiceId(String)` | `System`; max 64-byte UTF-8 for
  the inner string; return `CALYX_LEDGER_ACTOR_TOO_LONG` if over limit.
- [ ] Add `CALYX_LEDGER_ACTOR_TOO_LONG` to error catalog with remediation
  `"actor id must be ≤64 bytes UTF-8"`.
- [ ] `LedgerAppender` stores `last_ts: u64` (nanoseconds, server-stamped);
  `fn stamp_ts(&mut self) -> u64` — calls `self.clock.now_ns()`; if result
  ≤ `self.last_ts`, returns `self.last_ts + 1` (monotonic clamp, not panic);
  updates `last_ts`.
- [ ] `fn append` uses `stamp_ts()` for `ts`; the caller provides `actor` but
  never provides `ts` (it is server-stamped).
- [ ] On `LedgerAppender::open`, recover `last_ts` from the last row's `ts`
  field so monotonicity survives restarts.
- [ ] `ActorId::validate(&self) -> Result<()>` — checks UTF-8 byte length ≤ 64.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: inject a `MockClock` that returns 1000, 1000, 1001 on successive
  calls; append 3 entries; assert `ts` values are 1000, 1001, 1002 (monotonic
  clamp fires on the second call).
- [ ] unit: restart appender with last recovered `ts=5000`; inject clock
  returning 4999; assert first new entry has `ts=5001` (monotonic clamp across
  restart).
- [ ] proptest: for any sequence of clock values (possibly non-monotone),
  `entry[i].ts <= entry[i+1].ts` always holds.
- [ ] edge (≥3): `actor = AgentId("")` (empty string) → `Ok(())` (empty is
  valid); `actor = AgentId("x".repeat(64))` → `Ok(())`; `actor =
  AgentId("x".repeat(65))` → `CALYX_LEDGER_ACTOR_TOO_LONG`.
- [ ] fail-closed: `ts` field in recovered last row is corrupted (0) → appender
  sets `last_ts=0` and monotonicity still holds going forward (no panic).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ledger` CF rows — `ts` and `actor` fields in each entry
- **Readback:** `calyx scan --cf ledger --range 0..10 | jq '[.seq, .ts, .actor]'`
  — prints a table; confirm `ts[i] <= ts[i+1]` for all i; confirm `actor`
  field is present and non-empty on every row.
- **Prove:** before: no `actor` or monotonic `ts` enforcement; after: scan
  output shows non-decreasing `ts`; `actor` field contains `ServiceId("calyx-aster")`
  (or the configured service id) on ingest entries; injected-clock test
  proves `SystemTime::now()` is never called in logic (grep confirms absence).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH35 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
