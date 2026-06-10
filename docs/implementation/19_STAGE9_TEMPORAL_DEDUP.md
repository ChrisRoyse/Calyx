# Stage 9 — Temporal & Dedup (PH40–PH42)

Every Calyx DB understands time in two distinct roles: temporal lenses
(E2/E3/E4) for **search/retrieval only** under AP-60 (never dominant), and the
database's **event/sequence/recurrence understanding** as a capability layer.
Dedup is strictly the TCT cosine-`Gτ` guard over content slots. Built only from
the Royse corpus. Spans `calyx-sextant`/`calyx-aster`/`calyx-loom`. **Living-
system role:** the sense of time.

---

## PH40 — Temporal fusion + AP-60 post-retrieval boost
- **Current.** T01 #373, T02 #374, and T03 #375 are FSV-backed on aiwonder;
  #376 causal confidence gate is next.
- **Objective.** E2/E3/E4 bias retrieval ranking gently — never dominant, never
  during ANN retrieval.
- **Deps.** PH24 (search), PH22 (E2/E3/E4 lenses).
- **Deliverables.** `temporal_search` (post-retrieval boost 50% recency / 35%
  sequence / 15% periodic), time windows (`last_hours`/`last_days`), causal gate
  (high-conf ×1.10, low ×0.85).
- **Key tasks.** **AP-60 invariant**: temporal weight 0.0 in primary retrieval;
  boost applied after; E2 relative to query-time not ingest-time;
  timezone-aware E3.
- **FSV gate.** a recent/periodic item that doesn't match a content lens does
  **not** surface (temporal never dominant); the boost reorders only post-
  retrieval (read ranked results before/after boost).
- **Axioms/PRD.** A27, `25 §3`, `10 §6`.

## PH41 — DedupPolicy TctCosine + recurrence series + signature
- **Objective.** Deduplicate by multi-content-slot `Gτ` agreement; collapse
  recurrences into one event + a timestamp series; configurable at creation.
- **Deps.** PH37 (Gτ), PH09 (ingest).
- **Deliverables.** `DedupPolicy { Off|Exact|TctCosine{required_slots,tau,
  action} }`, `ingest_at(input,t)` → `New|DedupMerge{into,occurrence}`,
  recurrence series store, recurrence signature detector (content slots agree +
  temporal slots differ), reversible + Ledger-logged merges.
- **Key tasks.** content-only agreement (temporal excluded); **MUST NOT merge
  constellations with conflicting anchors**; recurrence series rollup/retention
  (bounded, A26); `dedup_audit` (per-slot cos, reversible).
- **FSV gate.** near-but-distinct pair → **not merged** at calibrated τ; same-
  content/opposite-anchor pair → **stays separate**; a recurring event → one
  event + a time series (read the series + the merge audit, reversible byte-for-
  byte).
- **Axioms/PRD.** A28, A3, `25 §4/§5`, `17 §7.1`.

## PH42 — Grounded recurrence wiring across engines
- **Objective.** Compute recurrence intelligence once (on ingest) and flow it to
  every engine — optimal use system-wide.
- **Deps.** PH41, PH28 (Assay), PH33 (kernel).
- **Deliverables.** wiring: Assay (frequency as grounded anchor; **oracle self-
  consistency** from recurring outcomes' agreement), Loom (temporal cross-terms
  / co-occurrence), Lodestar (frequency→kernel candidacy; time-window kernels),
  Ward (non-recurring = novelty), Sextant (AP-60 boost), Compression (dedup
  count = meaning-compression ratio), Anneal (importance/cadence).
- **Key tasks.** `oracle_self_consistency(domain)` from recurring anchors;
  temporal lead/lag (raw material for causality, Stage 11); surprise `−log p`
  for anomaly (never inflates bits).
- **FSV gate.** recurring events with agreeing outcomes → high self-consistency;
  with differing outcomes → flaky (ceiling drops) — measured natively (read);
  frequency raises kernel candidacy (read node weights).
- **Axioms/PRD.** A29, `25 §4c`, `07 §3b`, `08 §2`.

---

## Stage 9 exit
Every Calyx DB understands time (E2/E3/E4 retrieval-only under AP-60),
deduplicates strictly by TCT cosine-`Gτ` over content slots without ever merging
conflicting anchors, captures the same action recurring over time as a series,
and makes grounded recurrence (frequency, oracle self-consistency, causality,
kernel importance, surprise) flow system-wide — PRD `TEMPORAL`/`DEDUP`/
`RECURRENCE`.
