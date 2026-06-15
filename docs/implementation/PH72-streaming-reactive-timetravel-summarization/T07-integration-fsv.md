# PH72 · T07 — Integration FSV: all four capabilities proven on a real stream/corpus

| Field | Value |
|---|---|
| **Phase** | PH72 — Streaming + Reactive + Time-Travel + Universal Summarization |
| **Stage** | S20 — Critical Capabilities |
| **Crate** | `calyx-aster` / `calyx-loom` / `calyx-lodestar` (cross-cutting) |
| **Files** | `crates/calyx-aster/src/stream/tests.rs` (≤500), `crates/calyx-loom/src/reactive/tests.rs` (≤500), `crates/calyx-aster/src/timetravel/tests.rs` (≤500), `crates/calyx-lodestar/src/summarize_tests.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05, T06 (all prior PH72 cards) |
| **Axioms** | A27, A21, A26, A15, A16, A24 |
| **PRD** | `17 §8`, `04 §6`, `08 §4b` |

## Goal

Prove all four critical capabilities on a real stream/corpus on aiwonder in a
single end-to-end scenario, meeting the PH72 FSV exit gate. One real event stream
is ingested via `StreamIngester`; a reactive trigger fires when the recurring event
recurs; `as_of(t)` returns the historical state and fails closed before the
retention horizon; `summarize(scope)` returns the kernel of a slice. Tests use
injected `FakeClock` with known timestamps for determinism; the FSV readback
commands verify bytes on aiwonder (not the test harness). All four outputs are
written to `$CALYX_HOME/fsv/ph72_*.json` or equivalent for human/agent readback.

## Implementation status

Implemented for #577 in `crates/calyx-lodestar/tests/issue577_ph72_integration_fsv.rs`
with helpers in `tests/issue577_support/mod.rs`. The integration FSV emits all
nine named `ph72_*.json` artifacts (the card says "8" in a few places but names
nine files), uses a durable Aster vault with an injected deterministic clock via
`AsterVault::new_durable_with_clock`, and reads back Aster reactive CF rows,
`time_index` rows, `as_of` snapshots, and `SUMMARIZE_INVOKED` Ledger payloads.

The corrupt `time_index` fail-closed edge returns the current canonical Aster
storage code, `CALYX_ASTER_CORRUPT_SHARD`. `CALYX_STORAGE_CF_CORRUPT` appears in
the original card text, but the implemented storage catalog and existing Aster
tests use `CALYX_ASTER_CORRUPT_SHARD` for malformed CF/time-index bytes.

## Build (checklist of concrete, code-level steps)

- [ ] `tests::streaming_fsv`: construct a real vault at `$CALYX_HOME/test/ph72`; instantiate `StreamIngester` with `FakeClock`; send 100 events from a known seeded corpus (timestamps `t=1..100` ms); call `drain_and_close`; assert `stats.ingested == 100`; write `$CALYX_HOME/fsv/ph72_stream_stats.json` with `{ ingested, backpressured, quantized }`
- [ ] `tests::streaming_backpressure_fsv`: set `BackpressureGuard { capacity: 10 }`; send 11 events without refill; assert the 11th returns `CALYX_STREAM_BACKPRESSURE`; write `$CALYX_HOME/fsv/ph72_backpressure.json` with `{ error_code: "CALYX_STREAM_BACKPRESSURE", event_index: 11 }`
- [ ] `tests::reactive_trigger_fsv`: ingest the recurring event (same content, different timestamps) 3 times via `StreamIngester`; the 3rd ingest fires the `EventRecurs { min_occurrences: 3 }` trigger; `observe_delta(sub_id)` returns exactly one `TriggerFired`; assert `fired.cx_id` matches the recurring series id; assert `fired.ledger_ref` resolves to the 3rd ingest WAL seqno; write `$CALYX_HOME/fsv/ph72_trigger_fired.json`
- [ ] `tests::reactive_audit_fsv`: after the trigger fires, call `engine.audit_log_entries()` → assert 3 entries (2 no-match, 1 match); write the audit log to `$CALYX_HOME/fsv/ph72_trigger_audit.json`
- [ ] `tests::timetravel_historical_fsv`: ingest C1 at `FakeClock(t=500ms)`, C2 at `t=1000ms`; call `as_of(t=700ms)` → assert result contains C1 but NOT C2; call `as_of(t=1000ms)` → assert result contains both; write both snapshots' cx-id lists to `$CALYX_HOME/fsv/ph72_asof_500.json` and `ph72_asof_1000.json`
- [ ] `tests::timetravel_horizon_fsv`: configure `RetentionHorizon::Absolute { horizon_millis: 300 }`; call `as_of(t=200ms)` → assert `CALYX_TIMETRAVEL_BEFORE_HORIZON` with `horizon_millis=300`; write `$CALYX_HOME/fsv/ph72_horizon_error.json` with `{ error_code, requested_millis, horizon_millis }`
- [ ] `tests::summarize_fsv`: call `summarize(Scope::Collection(coll_id))` on the 100-event corpus; assert `result.kernel_size ≥ 1`; assert `result.kernel_only_recall` is finite and `> 0.0`; assert Ledger entry kind == `SUMMARIZE_INVOKED`; write `$CALYX_HOME/fsv/ph72_summarize.json`
- [ ] `tests::summarize_as_of_fsv`: call `summarize_as_of(scope, t=500ms)` on the same vault → result kernel differs from the `t=1000ms` summary (kernel_size at t=500 ≤ at t=1000); write `$CALYX_HOME/fsv/ph72_summarize_asof.json`; assert no stale data returned when `t` before horizon
- [ ] Verify all 9 named JSON output files exist and are valid JSON before the test suite exits; a missing file is a test failure (not silent)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] All build steps above are the tests; each has a deterministic assertion on a known value from the seeded corpus
- [ ] proptest: streaming → reactive → as_of → summarize pipeline on `n_events ∈ [10, 200]` with `FakeClock`; assert `stream_stats.ingested == n_events`; `observe_delta` fires exactly once when `min_occurrences` is met; `as_of(t_mid)` returns exactly `floor(n_events/2)` cx; `summarize` kernel_size ≥ 1 and ≤ n_events
- [ ] edge: run the full integration pipeline with `n_events = 0` → no crash; `summarize` returns empty kernel; no reactive fires; `as_of(t=any)` returns `CALYX_TIMETRAVEL_NO_DATA`
- [ ] fail-closed: after streaming 50 events, corrupt the `time_index` CF with injected bad key; `as_of` → `CALYX_ASTER_CORRUPT_SHARD`; streaming and reactive are unaffected (fault-isolation per layer); assert both error and non-error code paths

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** all 9 named JSON files in `$CALYX_HOME/fsv/ph72_*.json` plus the Ledger entries; the WAL records for the 3rd ingest that fired the trigger; the `time_index` CF entries; the `SummarizeResult` in the JSON
- **Readback commands (run on aiwonder to prove each capability):**
  1. **Streaming:** `cat $CALYX_HOME/fsv/ph72_stream_stats.json` → `ingested: 100`; `cat $CALYX_HOME/fsv/ph72_backpressure.json` → `error_code: "CALYX_STREAM_BACKPRESSURE"`
  2. **Reactive:** `cat $CALYX_HOME/fsv/ph72_trigger_fired.json` → `trigger_id`, `cx_id`, `ledger_ref` present; `calyx readback ledger-entry <ledger_ref> --vault $CALYX_HOME/test/ph72` → prints the WAL record for the 3rd ingest; `cat $CALYX_HOME/fsv/ph72_trigger_audit.json` → 3 entries (2 `matched: false`, 1 `matched: true`)
  3. **Time-travel:** `cat $CALYX_HOME/fsv/ph72_asof_500.json` → 1 cx_id; `cat $CALYX_HOME/fsv/ph72_asof_1000.json` → 2 cx_ids; `cat $CALYX_HOME/fsv/ph72_horizon_error.json` → `error_code: "CALYX_TIMETRAVEL_BEFORE_HORIZON"`, `horizon_millis: 300`
  4. **Summarization:** `cat $CALYX_HOME/fsv/ph72_summarize.json | jq '{kernel_size, kernel_only_recall, grounded_fraction}'` → finite non-zero values; `cat $CALYX_HOME/fsv/ph72_summarize_asof.json` → `kernel_size` ≤ the full-corpus summarize's `kernel_size`
- **Prove:** the before→after delta that proves the goal: before running `tests::streaming_fsv`, `$CALYX_HOME/fsv/ph72_*` files do not exist; after the test suite passes, all 9 named files exist with the exact field values asserted above; the Ledger chain verifies without break (`calyx readback verify-chain --vault $CALYX_HOME/test/ph72`)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] All 9 named `ph72_*.json` FSV output files present on aiwonder and readable
- [ ] FSV evidence (all 8 JSON files + readback terminal output / screenshots) attached to the PH72 GitHub issue
- [ ] Ledger chain verification passes on the test vault (`CALYX_TIMETRAVEL_BEFORE_HORIZON` error readback matches the horizon configured in the test)
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
