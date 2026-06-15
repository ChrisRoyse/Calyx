# PH72 — Streaming + Reactive + Time-Travel + Universal Summarization

**Stage:** S20 — Critical Capabilities  ·  **Crate:** `calyx-aster` / `calyx-loom` / `calyx-lodestar` / `calyx-ward`  ·
**PRD roadmap:** `17 §8`  ·  **Axioms:** A27, A21, A26, A15, A16

## Objective

Ship four named critical capabilities that the architecture implies but requires
explicit first-class wiring: (1) **streaming / real-time ingestion** — the DB as
a native event-stream store with on-the-fly TurboQuant quantization and backpressure
(A26); (2) **reactive queries / triggers / subscriptions** — fire when a
constellation enters a new region, an event recurs, or drift is detected
(Ward novelty + temporal + new-region); a bounded, audited subsystem; (3)
**time-travel / as-of audit** — read the vault and the panel/kernel as it was at
wall-clock time `t` (MVCC snapshots keyed to time); declare a retention horizon
beyond which `as_of` fails closed with a structured error, never silently wrong;
(4) **universal summarization** — "the core of ANY slice" via the multi-scope
kernel (`08 §4b`): summarize any dataset, domain, period, or tenant on demand.
Each capability is Ledger-provenanced and FSV-proven on a real stream/corpus on
aiwonder. Cross-cutting: depends on temporal/recurrence (PH41), multi-scope
kernel (PH34), MVCC (PH08).

## Dependencies

- **Phases:**
  - PH08 (MVCC sequence numbers + snapshot reads — required for `as_of(t)` time-keyed snapshots)
  - PH34 (multi-scope kernel — required for universal summarization dispatch)
  - PH35 (Ledger hash-chain CF — required for Ledger-provenanced mutations)
  - PH37 (`Gτ` guard math — required for reactive trigger novelty check)
  - PH41 (DedupPolicy TctCosine + recurrence series — required for streaming ingest + recurrence trigger)
  - PH42 (grounded recurrence wiring — required for recurrence-fires-trigger integration)
- **Provides for:** `BUILD_DONE` — this is the final phase; all prior phases are prerequisites.

## Current state (build off what exists)

This is a cross-cutting phase over crates built in earlier stages. The relevant
existing infrastructure:
- `calyx-aster`: vault storage, WAL, MVCC seqno, CFs, recurrence series (PH41),
  ingest_at (PH41), `Vault` struct and `Snapshot` types (PH08).
- `calyx-lodestar`: `build_kernel`, `multi_scope`, `ScopeCache`, `Scope` enum
  (PH34); `kernel_answer` and grounding_gaps (PH33).
- `calyx-ward`: `Gτ` guard, calibrated `τ`, novelty→new-region (PH37/PH38).
- `calyx-loom`: recurrence series store, recurrence signature (PH41).
This phase adds the streaming pipeline, reactive trigger/subscription subsystem,
the `as_of(t)` API over MVCC-with-time-index, retention-horizon enforcement, and
the `summarize(scope)` public surface. All four capabilities are new modules
sitting atop the above engines.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-aster/src/stream/mod.rs` | `StreamIngester`: channel-based ingest pipeline, microbatch flush, backpressure token (A26) |
| `crates/calyx-aster/src/stream/quantize_online.rs` | On-the-fly TurboQuant wrapping `ingest_at`; slot-content quantized before write; seed content-addressed per slot |
| `crates/calyx-aster/src/stream/backpressure.rs` | `BackpressureGuard`: token bucket; `CALYX_STREAM_BACKPRESSURE` when budget exhausted |
| `crates/calyx-loom/src/reactive/mod.rs` | `TriggerDef`, `TriggerCondition` (NewRegion / EventRecurs / DriftDetected), `SubscriptionId`, bounded trigger registry |
| `crates/calyx-loom/src/reactive/engine.rs` | `ReactiveEngine`: evaluates triggers post-ingest; `fire_if_matches`; bounded queue; audit log |
| `crates/calyx-loom/src/reactive/stream_api.rs` | `subscribe(condition) -> SubscriptionId`; `observe_delta(sub_id) -> impl Stream<Item=TriggerFired>` |
| `crates/calyx-aster/src/timetravel/mod.rs` | `as_of(vault, t: Timestamp) -> TimeTravelSnapshot`; time-index over MVCC seqno→wall-clock |
| `crates/calyx-aster/src/timetravel/time_index.rs` | `TimeIndex`: CF mapping wall-clock millis → MVCC seqno; written in group-commit |
| `crates/calyx-aster/src/timetravel/retention.rs` | `RetentionHorizon`: declared as `Duration`; `as_of` before horizon → `CALYX_TIMETRAVEL_BEFORE_HORIZON` |
| `crates/calyx-lodestar/src/summarize.rs` | `summarize(vault, scope, params?) -> SummarizeResult`; delegates to `build_kernel` + `kernel_answer`; Ledger-provenanced |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Streaming ingest pipeline + on-the-fly quantize + backpressure | — (needs PH41, PH14) |
| T02 | Reactive trigger/subscription engine (NewRegion/Recurs/Drift), bounded + audited | T01 |
| T03 | `subscribe` / `observe_delta` stream API | T02 |
| T04 | `as_of(t)` over MVCC time-keyed snapshots (time-index in group-commit) | — (needs PH08) |
| T05 | Retention-horizon enforcement + fail-closed before horizon | T04 |
| T06 | Universal summarization via multi-scope kernel (`summarize(scope)`) | — (needs PH34) |
| T07 | Integration FSV: recurring event fires trigger; `as_of(t)` returns historical state + fails closed before horizon; slice kernel summarizes it | T01–T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

All four capability proofs must pass, evidence attached to PH72 GitHub issue:

1. **Streaming / real-time ingestion:** ingest a real event stream (≥100 events)
   via `StreamIngester`; read back each event's quantized vector via
   `calyx readback cx-list` — vectors present and byte-identical to off-line
   TurboQuant with the same seed; emit one event past backpressure budget →
   `CALYX_STREAM_BACKPRESSURE` returned.

2. **Reactive triggers:** configure a `TriggerCondition::EventRecurs` on a known
   recurring event; ingest the event again (new timestamp) → `TriggerFired` appears
   in `observe_delta` stream; read audit log via
   `calyx readback trigger-audit <sub_id>` — entry present with Ledger ref.

3. **Time-travel / as-of audit:** ingest Constellation C at time `t1`; mutate
   at `t2 > t1`; call `as_of(t1)` → original bytes recovered (not the mutated
   state); call `as_of(t0 < retention_horizon)` → `CALYX_TIMETRAVEL_BEFORE_HORIZON`
   returned, no data returned; verify via `calyx readback as-of <t1>` and
   `xxd` the time-index CF row.

4. **Universal summarization:** call `summarize(Scope::Collection(id))` on a real
   corpus; output `SummarizeResult { kernel_ids, kernel_only_recall, grounded_fraction }`
   read via `cat $CALYX_HOME/fsv/ph72_summarize_*.json`; Ledger entry present for
   the summarize operation.

## Risks / landmines

- **Time-index CF written in group-commit:** the wall-clock→seqno mapping must be
  atomic with the data write; if written separately, `as_of(t)` can see a gap.
  Write the time-index entry in the same WAL group-commit as the data keys.
- **On-the-fly quantize seed must be content-addressed:** `quantize_online` uses
  the slot's `LensId + CxId` as the TurboQuant seed so replay is bit-identical
  (A25). Never use a random seed.
- **Retention-horizon must fail closed (not silently stale):** `as_of` before the
  horizon MUST return `CALYX_TIMETRAVEL_BEFORE_HORIZON`; it must never return the
  oldest-available snapshot as a silent approximation (A16).
- **Reactive engine is bounded (A26):** the trigger queue and audit log have hard
  capacity limits; on overflow, `CALYX_REACTIVE_QUEUE_FULL` is returned and the
  oldest un-delivered event is discarded with a Ledger warning. Never grow unbounded.
- **Clock injection required (not `SystemTime::now()`):** all four capabilities
  inject the `Clock` trait; tests use `FakeClock` with injected timestamps so
  FSV assertions on `as_of` and recurrence timings are seeded and deterministic.
- **Summarization is NOT an LLM call:** `summarize(scope)` returns the MFVS kernel
  nodes — the structural core of the slice. The "summary" is the kernel IDs +
  recall metric, not generated text. Strict Royse theory (A24).
