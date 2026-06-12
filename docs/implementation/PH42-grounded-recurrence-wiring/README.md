# PH42 - Grounded Recurrence Wiring Across Engines

**Stage:** S9 - Temporal & Dedup
**Crate:** cross-crate (`calyx-assay`, `calyx-loom`, `calyx-lodestar`, `calyx-ward`, `calyx-sextant`, `calyx-aster`)
**PRD roadmap:** A29
**Axioms:** A29, A2, A20

## Objective

PH41 provides recurrence series, frequency, and cadence storage and readback.
PH42 derives the grounded recurrence intelligence that spans engines:

- Assay: frequency as grounded anchor; `oracle_self_consistency(domain)` from recurring outcomes' anchor agreement.
- Loom: temporal cross-terms and co-occurrence lead-lag.
- Lodestar: frequency-to-kernel candidacy and time-window kernels.
- Ward: non-recurring means novelty/highest information; overdue recurrence detection.
- Sextant: AP-60 frequency/recency boost.
- Compression: dedup count as meaning-compression ratio.
- Anneal: importance and cadence scheduling.

The surprise term `-log2(p)` for anomaly scoring is defined here, but it may never inflate stored constellation bits.

## Dependencies

- **Phases:** PH41 recurrence series and frequency count, PH28 KSG MI and partitioned NMI, PH33 kernel index and grounding gaps.
- **Provides for:** PH49 Oracle consequence prediction, PH43 Anneal importance/cadence weights, PH48 J objective recurrence signals.

## Current State

`calyx-assay`, `calyx-loom`, `calyx-lodestar`, `calyx-ward`, and
`calyx-sextant` have their prerequisite Stage 5-8 surfaces implemented and
FSV-signed-off. PH41 provides recurrence series/frequency storage, #578 public
recurrence read APIs (`recurrence_series`, `periodic_fit`, `periodic_recall`),
and #621 concurrency-safe occurrence allocation across multi-handle durable
opens.

PH42 wires those grounded recurrence signals into existing engine surfaces while
using an O(1) base-CF frequency anchor path for hot consumers instead of
recomputing or scanning recurrence series.

## Canonical Timezone Model

Occurrence timestamps are stored as UTC epoch seconds. Engines that bucket
hour-of-day or day-of-week must apply an explicit fixed `tz_offset_secs`
provided by the query or vault context before computing buckets. Existing
compatibility wrappers use `tz_offset_secs = 0` and are therefore UTC-only by
construction. Named IANA timezone and DST database conversion is not implicit in
Loom, Oracle, or Sextant; callers must provide the effective offset for the
context being scored/read back.

## Rolled Recurrence Semantics

Aster stores recurrence history as active occurrence rows plus a rolled summary
and base `recurrence.frequency`. Loom and Oracle treat active occurrence rows as
the only source that can define hour/day phase and cadence. Rolled summaries and
base frequency contribute total support/confidence and readback evidence after
active rows establish cadence. If retention leaves too few active rows to define
cadence or phase, consumers fail closed or return no periodic match with explicit
active/rolled support fields rather than inferring phase from rollup bytes alone.

Entry discipline update (2026-06-12): GitHub issue state records PH40
follow-ups #616/#618/#619, PH41 follow-ups #620/#626/#627/#628, and PH42
readback-surface gate #625 closed and FSV-backed. Those stale gates no longer
block PH42 sign-off; newer PH42 gaps such as #634/#635/#636 are tracked
separately.

## Deliverables

| File | Responsibility |
|---|---|
| `crates/calyx-loom/src/recurrence/cross_terms.rs` | Temporal cross-terms: co-occurrence lead-lag between two CxIds' recurrence series |
| `crates/calyx-assay/src/recurrence_anchor.rs` | Frequency as grounded anchor; `oracle_self_consistency(domain)` from recurring outcomes |
| `crates/calyx-lodestar/src/temporal_kernel.rs` | Frequency-to-kernel candidacy boost; time-window kernel scope |
| `crates/calyx-ward/src/novelty.rs` | Non-recurring novelty/highest-information signal; overdue recurrence detection |
| `crates/calyx-sextant/src/temporal/recurrence_boost.rs` | Frequency/recency contribution to AP-60 post-retrieval boost |
| `crates/calyx-aster/src/dedup/compression_ratio.rs` | Dedup count = meaning-compression ratio; expose `compression_ratio(cx_id)` |
| `crates/calyx-anneal/src/recurrence_schedule.rs` | Frequency-to-importance weight; cadence-to-retention/refresh schedule |
| `crates/calyx-loom/tests/recurrence_cross_terms.rs` | Tests for cross-terms and lead-lag |
| `crates/calyx-loom/tests/recurrence_cross_terms_fsv.rs` | Ignored FSV trigger that writes the PH42 temporal cross-term artifact |
| `crates/calyx-assay/tests/recurrence_anchor.rs` | Tests for `oracle_self_consistency` |
| `crates/calyx-assay/tests/recurrence_anchor_fsv.rs` | Ignored FSV trigger that writes the PH42 Assay report artifact |

## Tasks

| Card | Title | Depends |
|---|---|---|
| T01 | Assay: frequency as grounded anchor + `oracle_self_consistency` | - |
| T02 | Loom: temporal cross-terms + co-occurrence lead-lag | T01 |
| T03 | Lodestar: frequency-to-kernel candidacy; time-window kernels | T01 |
| T04 | Ward: non-recurring = novelty; surprise `-log2(p)` never inflates bits | T01 |
| T05 | Sextant: frequency/recency recurrence boost (AP-60) | T01 |
| T06 | Compression ratio + Anneal importance/cadence | T01 |
| T07 | FSV: recurring-agreeing -> high self-consistency; recurring-differing -> flaky; frequency -> kernel weight | T06 |

## FSV Exit Gate

The phase is done only when these claims are byte-proven on aiwonder:

1. **Self-consistency:** recurring events with agreeing outcomes produce
   `oracle_self_consistency >= 0.90`; recurring events with differing outcomes
   produce `oracle_self_consistency < 0.60`. Persist the Assay report and read
   `calyx readback assay-report --artifact <assay-report.json> --field oracle_self_consistency`.
2. **Frequency-to-kernel weight:** a constellation ingested N=50 times appears in
   the kernel graph node list with weight above baseline; a one-time
   constellation does not. Persist the kernel-weight report and read
   `calyx readback kernel-weights --artifact <kernel-weights.json>`.

#625 closed the cross-cutting readback-surface gate for these FSV claims. Tests
may trigger PH42 computations, but the verdict must be persisted JSON,
Aster/Ledger/CF/WAL bytes, or CLI readback output with BLAKE3-indexed artifacts.
The #625 CLI contract remains the artifact-backed bridge until individual PH42
engine cards add native vault/domain readers:

`calyx readback <surface> --artifact <json> [--field <path>]`

`<surface>` is one of `assay-report`, `temporal-cross-term`, `kernel-weights`,
`kernel-window`, `ward-novelty`, `compression-ratio`, or `anneal-schedule`.

PH42 readback artifacts are fail-closed v1 envelopes. The artifact root must be
a JSON object with:

- `schema_version: 1`
- `surface: <the requested readback surface>`
- `artifact_kind: "ph42.<surface>.v1"`
- `source_of_truth: "PH42 persisted artifact"`

`calyx readback <surface> --artifact <json>` rejects arbitrary JSON, mismatched
surfaces, missing required fields, and unsupported schema versions with
`CALYX_PH42_ARTIFACT_SCHEMA` before selecting any `--field` value.

## Implementation Progress

- #387 implemented Assay recurrence anchors and `oracle_self_consistency`.
- #388 implemented Loom temporal cross-terms and the persisted `temporal_xterm` CF/WAL row.
- #389 implemented Lodestar recurrence frequency kernel weighting and time-window kernels, with artifact-backed readback surfaces `kernel-weights` and `kernel-window`.
- #390 implements Ward recurrence novelty classification, overdue recurrence scanning, and retrieval-only `SurpriseScore` anomaly scoring with artifact-backed `ward-novelty` readback.
- #391 implements Sextant AP-60 recurrence boost from Base CF `recurrence.frequency` plus Recurrence CF last occurrence time, with `recurrence_boost` explain evidence on `TemporalSearchResult` hits.
- #392 implements Aster meaning-compression ratio from Base CF frequency and Anneal recurrence importance/cadence scheduling, with artifact-backed `compression-ratio` and `anneal-schedule` readbacks.
- #393 implements the PH42 exit-gate FSV across Assay, Lodestar, Ward, and Loom, with durable aiwonder artifacts proving self-consistency thresholds, frequency-ranked kernel weights, retrieval-only surprise bits, and directional temporal lead/lag.

## Risks / Landmines

- **Surprise `-log2(p)` definition:** the surprise term is the negative log probability of the event given its recurrence rate: `-log2(frequency / total_events)`. It must NEVER increase stored constellation bits; anomaly scoring is retrieval-only and never stored as a lens weight or information score. Audit every call site.
- **Cross-crate circular dependencies:** recurrence signals flow from `calyx-aster` as the data source to consumers. No consumer crate imports another consumer crate.
- **Sextant recurrence is read-only:** `calyx-sextant` may read Aster Base/Recurrence CFs for AP-60 recurrence evidence, but it must never write recurrence state. `TemporalPolicy.recurrence_boost = None` must skip those reads, and missing/invalid recurrence rows must fail closed with `CALYX_SEXTANT_RECURRENCE_READ_ERROR`.
- **PH41 readiness:** PH41 recurrence series/frequency storage, #578 public read APIs, and #621 concurrency-safe allocation are available. PH41 follow-ups #620/#626/#627/#628 are closed and FSV-backed; newer PH42 gaps #634/#635/#636 are tracked separately.
- **Grounded anchor immutability:** frequency is a grounded anchor (A2), a count of what happened. It must be read from the `frequency` field in the base CF written by PH41, not recomputed from the series on every hot-path call.
