# PH42 — Grounded Recurrence Wiring Across Engines

**Stage:** S9 — Temporal & Dedup  ·  **Crate:** cross-crate (`calyx-assay`,
`calyx-loom`, `calyx-lodestar`, `calyx-ward`, `calyx-sextant`, `calyx-aster`)  ·
**PRD roadmap:** A29  ·  **Axioms:** A29, A2, A20

## Objective

Recurrence intelligence (frequency, cadence, oracle self-consistency, temporal
lead/lag) is computed once — during `ingest_at` (PH41) — and made available to
every engine via grounded signals on the constellation's base CF record. This
phase wires each of the seven engine consumers: Assay (frequency as grounded
anchor; `oracle_self_consistency(domain)` from recurring outcomes' anchor
agreement), Loom (temporal cross-terms / co-occurrence lead-lag), Lodestar
(frequency → kernel candidacy; time-window kernels), Ward (non-recurring =
novelty/highest-information), Sextant (AP-60 frequency/recency boost), Compression
(dedup count = meaning-compression ratio), and Anneal (importance/cadence). The
surprise term `−log p` for anomaly scoring is defined but may never inflate bits.

## Dependencies

- **Phases:** PH41 (recurrence series + frequency count — the data these engines
  consume), PH28 (KSG MI + partitioned NMI — Assay MI computation reused here
  for self-consistency), PH33 (kernel index + grounding gaps — Lodestar candidacy
  logic extended here)
- **Provides for:** PH49 (Oracle consequence prediction needs `oracle_self_consistency`
  and cadence from this phase), PH43 (Anneal importance/cadence weights), PH48
  (J objective uses recurrence signals)

## Current state (build off what exists)

`calyx-assay`, `calyx-loom`, `calyx-lodestar`, `calyx-ward`, and
`calyx-sextant` have their prerequisite Stage 5-8 surfaces implemented and
FSV-signed-off. PH41 now provides recurrence series/frequency storage and the
#578 public recurrence read APIs (`recurrence_series`, `periodic_fit`,
`periodic_recall`). PH42 should wire those grounded recurrence signals into the
already-built engine surfaces, while using an O(1) base-CF frequency anchor path
for hot consumers rather than recomputing/scanning recurrence series. This is
primarily a wiring + API-surface phase: each engine gets a small, well-defined
interface to the recurrence signals stored in the base CF.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-loom/src/recurrence/cross_terms.rs` | Temporal cross-terms: co-occurrence lead-lag between two CxIds' recurrence series |
| `crates/calyx-assay/src/recurrence_anchor.rs` | Frequency as grounded anchor; `oracle_self_consistency(domain)` from recurring outcomes |
| `crates/calyx-lodestar/src/temporal_kernel.rs` | Frequency → kernel candidacy boost; time-window kernel scope |
| `crates/calyx-ward/src/novelty.rs` | Non-recurring = novelty/highest-information signal; overdue recurrence detection |
| `crates/calyx-sextant/src/temporal/recurrence_boost.rs` | Frequency/recency contribution to AP-60 post-retrieval boost |
| `crates/calyx-aster/src/dedup/compression_ratio.rs` | Dedup count = meaning-compression ratio; expose `compression_ratio(cx_id)` |
| `crates/calyx-anneal/src/recurrence_schedule.rs` | Frequency → importance weight; cadence → adaptive retention/refresh schedule |
| `crates/calyx-loom/src/recurrence/tests.rs` | Tests for cross-terms and lead-lag |
| `crates/calyx-assay/src/tests.rs` | Tests for `oracle_self_consistency` |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Assay: frequency as grounded anchor + `oracle_self_consistency` | — |
| T02 | Loom: temporal cross-terms + co-occurrence lead-lag | T01 |
| T03 | Lodestar: frequency → kernel candidacy; time-window kernels | T01 |
| T04 | Ward: non-recurring = novelty; surprise `−log p` (never inflates bits) | T01 |
| T05 | Sextant: frequency/recency recurrence boost (AP-60) | T01 |
| T06 | Compression ratio + Anneal importance/cadence | T01 |
| T07 | FSV: recurring-agreeing → high self-consistency; recurring-differing → flaky; frequency → kernel weight | T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Two gates:
1. **Self-consistency:** recurring events with agreeing outcomes → `oracle_self_consistency` ≥ 0.90; with differing outcomes → `oracle_self_consistency` ≤ 0.60 (ceiling drops). Read `calyx readback assay-report --domain <domain>` showing the `self_consistency` scalar.
2. **Frequency → kernel weight:** a constellation ingested N=50 times (high frequency) must appear in the kernel graph node list with weight above the baseline; a one-time constellation must not. Read `calyx readback kernel-weights` and confirm the ordering.

## Risks / landmines

- **Surprise `−log p` definition:** the surprise term is the negative log probability of the event given its recurrence rate — `−log(frequency / total_events)`. It must NEVER increase the stored bits for a high-frequency event; anomaly scoring is additive to retrieval scoring only (never stored as a lens weight). Audit every call site.
- **Cross-crate circular dependencies:** wiring seven crates creates potential cycles. All recurrence signals flow from `calyx-aster` (the data source) through `calyx-loom` (the transformer) to consumers. No consumer crate imports another consumer crate.
- **PH41 readiness:** PH41 recurrence series/frequency storage and #578 public
  read APIs are available. PH42 still needs consumer-facing O(1) base-CF
  frequency anchor reads for hot paths; scan-based periodic readback APIs are
  evidence/debug surfaces, not the PH42 runtime path. PH28 Assay MI and PH33
  Lodestar kernel surfaces are already FSV-signed-off and should be reused
  directly rather than stubbed.
- **Grounded anchor immutability:** frequency is a grounded anchor (A2) — a count of what happened. It must be read from the `frequency` field in the base CF (written by PH41), never recomputed from the series on every call (O(1), not O(N)).
