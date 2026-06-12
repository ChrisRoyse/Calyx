# Stage 9 — Temporal & Dedup (PH40–PH42)

Every Calyx DB understands time in two distinct roles: temporal lenses
(E2/E3/E4) for **search/retrieval only** under AP-60 (never dominant), and the
database's **event/sequence/recurrence understanding** as a capability layer.
Dedup is strictly the TCT cosine-`Gτ` guard over content slots. Built only from
the Royse corpus. Spans `calyx-sextant`/`calyx-aster`/`calyx-loom`. **Living-
system role:** the sense of time.

---

## PH40 — Temporal fusion + AP-60 post-retrieval boost
- **Status.** Complete and FSV-backed on aiwonder: T01-T06 #373-#378 plus
  post-sweep AP-60 final-surface hardening #615 and follow-ups #616/#618/#619.
  Those follow-ups are closed and FSV-backed for bounded overfetch before
  window filtering, negative `FusionWeights` validation, and public periodic
  scorer scope/query-time semantics.
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
  retrieval and non-positive hits are filtered from final `temporal_search`
  results (read ranked results before/after boost).
- **Axioms/PRD.** A27, `25 §3`, `10 §6`.

## PH41 — DedupPolicy TctCosine + recurrence series + signature
- **Status.** T01 #379 is complete and FSV-backed on aiwonder at
  `/home/croyse/calyx/data/fsv-issue379-dedup-policy-20260610-0083015`;
  T02 #380 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue380-dedup-validation-20260610-5af9a20`;
  T03 #381 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue381-anchor-conflict-20260610-00c0540`;
  T04 #382 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue382-ingest-at-20260610-1a0c560`;
  T05 #383 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue383-recurrence-series-20260610-bacf9d2`;
  T06 #384 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue384-recurrence-signature-20260610-8b0d0bb`;
  post-T06 recurrence fallback hardening #623 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue623-recurrence-fallback-20260610-1dc61cf`;
  PH41 T07 #385 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue385-dedup-audit-20260610-cc9f57b`;
  PH41 T08 #386 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue386-dedup-invariants-20260610-5fdab01`
  (`dedup-invariants-readback.json` BLAKE3
  `f568a21145a811671c79f2cba56b08eee36b6536fa64dbd598ee73d5d527e140`).
  PH41 public recurrence read API follow-up #578 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue578-periodic-recall-20260610-240de5a`
  (`periodic-recall-readback.json` BLAKE3
  `7973b14e446ddd9d1901648d5dd66cf1afac2fbc9a6806b191f4bb0682921c79`).
  PH41 recurrence occurrence allocation concurrency hardening #621 is complete
  and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue621-recurrence-concurrency-20260610-b1fdf5d`
  (`recurrence-concurrency-readback.json` BLAKE3
  `91e0ad19b81589f49591a9ed65ee6efb3c656a82ebc545a27c62820d1cfa96d8`).
  PH41 WAL recovery/open serialization #624 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue624-wal-recovery-lock-20260610-1e4b34c`
  (`wal-recovery-lock-readback.json` BLAKE3
  `1c2c255e517691660f8ba45c78b625dd5c4d6eb68b5d7609a69cc8bf2b5bff84`).
  PH41 durable dedup policy validation parity #617 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue617-dedup-panel-validation-20260610-07884d9`
  (`dedup-policy-readback.json` BLAKE3
  `9e7636d173dd188b52f3aa232c70fe279e18ad89988a179ec4296e1287ce7423`).
  PH41 WAL failure error-code contract #622 is complete and FSV-backed at
  `/home/croyse/calyx/data/fsv-issue622-recurrence-wal-failure-20260610-bf0d380`.
  The stable code remains PRD 18's `CALYX_DISK_PRESSURE`; no
  `CALYX_WAL_WRITE_ERROR` was added. PH41 follow-ups #620/#626/#627/#628 are
  closed and FSV-backed.
- **Objective.** Deduplicate by multi-content-slot `Gτ` agreement; collapse
  recurrences into one event + a timestamp series; configurable at creation.
- **Deps.** PH37 (Gτ), PH09 (ingest).
- **Deliverables.** `DedupPolicy { Off|Exact|TctCosine{required_slots,tau,
  action} }`, `ingest_at(input,t)` → `New|DedupMerge{into,occurrence}`,
  recurrence series store, recurrence signature detector (content slots agree +
  temporal slots differ), public `recurrence_series`/`periodic_fit`/
  `periodic_recall` read APIs, reversible + Ledger-logged merges.
- **Key tasks.** content-only agreement (temporal excluded); **MUST NOT merge
  constellations with conflicting anchors**; recurrence series rollup/retention
  (bounded, A26); `dedup_audit` (per-slot cos, reversible).
- **FSV gate.** near-but-distinct pair → **not merged** at calibrated τ; same-
  content/opposite-anchor pair → **stays separate**; a recurring event → one
  event + a time series (read the series + the merge audit, reversible byte-for-
  byte); temporal slots are excluded from agreement; recurrence frequency reads
  back accurately at 10 occurrences.
- **Axioms/PRD.** A28, A3, `25 §4/§5`, `17 §7.1`.

## PH42 — Grounded recurrence wiring across engines
- **Status.** PH40 follow-ups #616/#618/#619, PH41 follow-ups
  #620/#626/#627/#628, and PH42 readback-surface gate #625 are closed and
  FSV-backed. Newer PH42 gaps such as #634/#635/#636 are tracked separately.
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
