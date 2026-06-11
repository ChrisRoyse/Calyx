# PH56 · T08 — Bounded concurrent-query admission with deadline reject

| Field | Value |
|---|---|
| **Phase** | PH56 — Bounded caches/queues/memtables + arenas/pools |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/query_admission.rs` (≤500), `crates/calyx-sextant/tests/query_admission_fsv.rs` (≤500) |
| **Depends on** | PH24 search surface · PH56 resource boundedness discipline |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §6`, `dbprdplans/18 §6` |

## Goal

Bound the concurrent query/read path so query bursts cannot create an unbounded heap or waiter
pile-up. `SearchEngine` admits up to a configured concurrent cap, queues a bounded number of
excess queries with a deadline, and rejects over-deadline or over-queue callers with
`CALYX_BACKPRESSURE`.

## Build

- [x] Add `QueryAdmissionController` with `max_concurrent`, `max_queued`, and `queue_timeout`.
- [x] Add RAII query permits so in-flight gauges release even when search returns early.
- [x] Add counters/gauges for in-flight, queued, admitted, queued, rejected, deadline-rejected,
  queue-full-rejected, and max observed queue/concurrency.
- [x] Wire `SearchEngine::search_inner` through the admission guard after cheap query validation
  and before index reads.
- [x] Expose `query_admission_stats()` and `query_admission_metrics_text()` as the readback
  surface until the later calyxd Prometheus endpoint owns metric export.

## Tests

- [x] unit: immediate admits stop at `max_concurrent`; next query returns `CALYX_BACKPRESSURE`.
- [x] unit: a queued waiter admits when the active permit releases before deadline.
- [x] unit: a queued waiter rejects with `CALYX_BACKPRESSURE` after deadline.
- [x] unit: `max_concurrent == 0` fails closed without queue growth.
- [x] integration: a blocking Sextant index proves real `SearchEngine::search` calls saturate,
  queue, deadline-reject, and record metrics.

## FSV

- **SoT:** `/home/croyse/calyx/data/fsv-issue589-query-admission-<ts>/query-admission-readback.json`
  and `query-admission-metrics.prom` produced on aiwonder by the ignored FSV test.
- **Readback:** `cat` the JSON and metric text after the trigger; verify
  `calyx_query_admission_rejected_total` and
  `calyx_query_admission_deadline_rejected_total` are `1`, max observed queue is `1`, final
  in-flight is `0`, and RSS delta stays bounded for the synthetic probe.
- **Prove:** first query is admitted and blocks in the real Sextant index, second query exceeds
  the deadline and returns `CALYX_BACKPRESSURE`, queue depth returns to zero, and no unbounded
  waiter growth occurs.
