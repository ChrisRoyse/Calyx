# 25 — Native Temporal Understanding & TCT Deduplication

> **Living-system role:** the sense of time — understanding events, sequences, and recurrence (A31 — DOCTRINE §1b)

Implements new **A27 (native temporal understanding)** and **A28 (TCT cosine-`Gτ` deduplication)**. Built **only** from the Royse corpus (DOCTRINE §2): ContextGraph's three temporal embedders (E2/E3/E4) and the AP-60 invariant; the Teleological Constellation + the cosine-similarity guard `Gτ` (Ward/Polis). No external temporal model, no external deduplication method — fully custom-fitted for AGI and the Oracle.

## 1. Mandate: every Calyx database understands time — two distinct roles

A Calyx vault is temporal by construction; time plays **two separate roles** that must not be confused:

1. **Temporal lenses (E2/E3/E4) are for SEARCH & RETRIEVAL ONLY.** The system **understands time when navigating** — recency, periodic rhythm, and sequence applied as **post-retrieval boosts** (AP-60: never dominant, never primary content). They do **not** represent content and do **not** drive deduplication. They make *search* time-aware.
2. **Database-level temporal understanding is a capability layer.** Separately, the database uses timestamps + event/sequence structure (not the retrieval lenses) to understand **events, sequences, and recurrence**, opening **new critical capabilities** (§4b): recurrence series, repeat-pattern detection, next-occurrence prediction (Oracle), change-point/drift detection, time-travel/as-of queries, temporal kernels, retention.

Design rule: **the lenses bias retrieval ranking (gently); the database's event understanding powers new features.** Both on by default, configurable at creation (§5).

## 2. The temporal lens family — for search & retrieval (from ContextGraph E2/E3/E4)

Three deterministic, algorithmic temporal lenses ship in every panel. Being algorithmic (closed-form, no trained weights), they are perfect **frozen, data-oblivious** lenses — no codebook, hot-swap instantly, never drift (A4/A5, and pair cleanly with TurboQuant since there is nothing to train).

| Lens | ContextGraph name | Measures | Math (from his options) |
|---|---|---|---|
| **E2 — Temporal-Recent** | `E2_Temporal_Recent` | recency / freshness | `DecayFunction`: **Linear** `score = 1 − age/max_age` · **Exponential** `score = exp(−age·0.693/half_life)` · **Step** buckets (<1h: 0.8, <1d: 0.5, ≥1d: 0.1). `half_life` configurable. |
| **E3 — Temporal-Periodic** | `E3_Temporal_Periodic` | **repeat patterns** | hour-of-day (0–23) and day-of-week (0–6) matching; `PeriodicOptions{target_hour, target_day_of_week, use_now}`. This is the **recurring-pattern engine**: it scores how well an event matches a periodic rhythm. |
| **E4 — Temporal-Positional** | `E4_Temporal_Positional` | sequence / order | positional encoding over event order; `SequenceOptions`, `SequenceDirection`, `MultiAnchorMode`. Orders occurrences within a series and a session. |

These answer "do temporal embedders exist or do we build our own": **they already exist in your research, custom-built for exactly this, and Calyx adopts them as the standard temporal family** — extended below to capture events over time. Richer learnable temporal lenses can be *commissioned* later through the Registry like any lens (A6), but the baked-in family is yours and needs no training.

## 3. The temporal math & fusion (AP-60: aware, never biased)

- **Fusion weighting** (from his result type): temporal score is a weighted combination — **50% recency (E2) + 35% sequence (E4) + 15% periodic (E3)** — tunable per vault.
- **AP-60 invariant (binding):** temporal lenses are **never dominant in primary retrieval** (weight 0.0 in the main ranking); applied as a **post-retrieval boost** (temporal awareness without temporal bias). A recent/periodic item must still match a content lens to surface — recency cannot drown relevance. (Inherited verbatim from ContextGraph AP-60 / "temporal awareness without temporal bias.")
- **Time windows:** `last_hours(n)` / `last_days(n)` scope a query to a recency window without distorting in-window ranking.
- **Where it runs:** E2/E3/E4 are closed-form in Forge (`13`); near-zero cost, bit-parity CPU/GPU.

## 4. Capturing the same event over time & repeat patterns

The heart of the request, assembled entirely from his constructs:

1. **An event is a constellation** measured at a time `t` (its E2/E3/E4 slots stamped with `t`).
2. **Recurrence series.** When the *same event* happens again (detected by §5 dedup), Calyx does **not** store a new copy — it appends an **occurrence** `(t_k, context)` to that event's **recurrence series** (one constellation, many timestamps). Heavy deduplication and repeat-pattern capture are the *same* operation.
3. **Repeat patterns are read off the series:**
   - **E3 periodic** scores the rhythm — "this event recurs around Tuesdays 14:00" (hour/day matching over the occurrence times).
   - **E2 recency** — how fresh the latest occurrence is.
   - **E4 sequence** — the order/position of occurrences.
   - inter-occurrence cadence (gaps between `t_k`) is a derived scalar on the series.
4. **The Oracle over time (`21`).** A recurrence series is a world-model signal: given the periodic rhythm + cadence, the Oracle predicts the **next occurrence** (a temporal consequence / butterfly step) with calibrated confidence, capped by oracle self-consistency, refusing if the panel is insufficient (A20). "Knows everything that has happened and will happen" applied to recurring events.
5. **Navigation (`10`).** Sextant gains temporal navigation over series: timelines, "events like this that recur," periodic recall ("what usually happens at this hour/day"), and sequence traversal — all from E2/E3/E4 as post-retrieval structure.

## 4b. Database-level temporal understanding — new critical capabilities

Distinct from the retrieval lenses (§2), the database's understanding of **events, sequences, and time** unlocks capabilities no metadata-timestamp database has. These run on timestamps + the recurrence series + content-lens grounding, **not** the AP-60 retrieval boost:

| Capability | What it does | Built from |
|---|---|---|
| **Recurrence series** | one event, many occurrences over time (§4) — heavy dedup + repeat capture | content-`Gτ` dedup + timestamps |
| **Repeat-pattern detection** | "this recurs ~Tuesdays 14:00 / every ~7 days" | E3 periodic read over occurrence times + cadence scalar |
| **Next-occurrence prediction** | Oracle predicts when the event recurs, calibrated, refusing if insufficient | the Oracle (`21`) over the series |
| **Change-point / drift detection** | when a recurring pattern shifts or a stream's distribution moves | agreement-graph drift (`06`) over time + cadence change |
| **Temporal kernel** | the grounding kernel of a **time window** — "what mattered then" | Lodestar `scope=TimeWindow` (`08 §4b`) |
| **Time-travel / as-of queries** | read the vault as it was at time `t` (and the panel/kernel then) | MVCC snapshots keyed to time (`04 §6`) |
| **Sequence / session reconstruction** | ordered event timelines, causal-over-time chains | E4 sequence + asymmetric causal lenses (`03`/`10`) |
| **Temporal provenance** | every occurrence and merge is time-stamped and replayable | Ledger (`11`) |
| **Time-based retention / GC** | decay-driven tiering and rollup of cold/old events | E2 decay informs retention (`24`) |
| **Event causality over time** | "X recurring tends to precede Y" — temporal lead/lag | recurrence series + causal asymmetry (`08`) |

These are the "all new critical capabilities" from understanding time and events — separate from, additive to, the gentle retrieval boost of the temporal lenses.

## 4c. What the time of an event MEANS — grounded recurrence (A29)

The time something happened is not metadata; it is **grounded reality**, and combined with dedup it is one of the system's most valuable signals — far beyond search/nav.

### The recurrence signature: "the same action across time"
On every ingest, two facts are read together:
- **all CONTENT lenses agree** (`∀ required content slot k: cos(new_k, existing_k) ≥ τ_k`) → it is the **same action** (the content lenses *mean something*, so their agreement is real identity, not coincidence — no-flatten makes this strong, `09`);
- **the TEMPORAL lenses (E2/E3/E4) differ** → it is a **different time**.

That exact combination — **content-identical + time-distinct** — is the **recurrence signature**: the database *automatically* recognizes "the exact same action, again, at a new time." (This is *why* temporal lenses are excluded from dedup, §5: content is the "what," time is the "when.") The database appends an occurrence `t_k` to the event's recurrence series and increments frequency — **no application code; the database understands recurrence by construction** (A22).

### Why event-time is grounded reality (six meanings, all in your framework)
| Meaning of event-time | In your framework |
|---|---|
| **Frequency is a grounded anchor** | a *count of what actually happened* is reality, not a learned vector — the most honest signal (A2). Dedup turns N copies into one event + a grounded frequency. |
| **Recurrence = the Oracle's predictive evidence** | the Oracle "knows what will happen" by knowing the **rate**; counting duplicates over time *is* measuring that rate → next-occurrence + consequence prediction (`21`). |
| **Duplicate-outcome consistency = oracle self-consistency, measured** | the paper's ceiling `τ_corr ≤ oracle_self_consistency` (flakiness + validity): same event, same outcome across occurrences → consistent/valid; differing outcomes → flaky. Calyx measures this **natively** by tracking recurring events' anchors (`07`/`21`). |
| **Temporal co-occurrence = grounded association in time = causality** | DDA across time: if A recurs shortly before B recurs, that is a grounded directional association — the raw material for consequence prediction (`06`/`21`). |
| **Frequency = importance + information** | recurring = reinforced (cortical-columns / lexicon-grows-by-differentiation) → **kernel candidacy** (`08`); and rare = high surprise = high bits (the fifth element) → the **non-recurring outlier is the highest-information event** (instant anomaly). |
| **A moment becomes definable** | Gärdenfors applied to time: the meaning of a moment is the **constellation of events grounded at that moment** (`02`/`10` `define`). Time is a grounded index into the web of associations. |

### Used automatically throughout the whole system (the mandate)
Recurrence intelligence is computed once (on ingest) and flows to every engine — *optimal use everywhere*:

| Engine | Uses grounded recurrence for |
|---|---|
| **Assay (`07`)** | frequency as a grounded anchor to compute bits about; **oracle self-consistency** (do recurring events' outcomes agree?) — flakiness/validity measured |
| **Oracle (`21`)** | empirical rate + cadence → next-occurrence + consequence prediction; co-occurrence → causal discovery |
| **Loom (`06`)** | **temporal cross-terms** — associations between events' recurrence patterns |
| **Lodestar (`08`)** | frequency → kernel candidacy (recurring events anchor the kernel); temporal-window kernels |
| **Ward (`09`)** | the **non-recurring** = novelty/anomaly (highest information); overdue recurrence (expected event missing) |
| **Anneal (`12`)** | frequency → importance weighting; cadence → adaptive retention/refresh |
| **Sextant (`10`)** | frequency/recency as AP-60 post-retrieval boost (search), and "what recurs at this time" |
| **Compression (`23`/A25)** | the duplicate-count **is** the meaning-compression ratio for that event; recurrences stored once |
| **Memory/GC (`24`)** | frequency/recency informs hot-keep vs roll-up of occurrences |
| **Ledger (`11`)** | every occurrence + merge timestamped, reversible, auditable |

**The database calculates and understands recurrence automatically and makes optimal use of this intelligence system-wide** — turning "the same action across time" into grounded frequency, oracle-consistency measurement, causal evidence, kernel importance, information/surprise, and prediction.

## 5. TCT cosine-`Gτ` deduplication (strictly your research)

Deduplication in Calyx is **only** the Teleological-Constellation cosine-similarity guard `Gτ` (Ward, `09`; Polis/ClipCannon constellation guard). No MinHash, no LSH, no external semantic-dedup method — explicitly excluded. Three levels:

| Level | Mechanism (yours) | Result |
|---|---|---|
| **Exact** | content-addressed `CxId = blake3(input ‖ panel_version ‖ salt)` | re-ingesting identical bytes is idempotent (already in `03`) |
| **Near-duplicate (semantic)** | **`Gτ` cosine guard**: a new constellation whose **required CONTENT slots** each have `cos(new_k, existing_k) ≥ τ_k` is a duplicate of that existing constellation. (Temporal lenses E2/E3/E4 are **excluded** from dedup agreement — two occurrences of the same event differ in time but are the *same content*, so dedup compares content slots only.) | collapse / link instead of storing a copy |
| **Temporal (recurrence)** | a near-duplicate that arrives at a new time → **append an occurrence to the recurrence series** (§4), not a new constellation | one event, many timestamps — heavy dedup + repeat-pattern capture |

Why this is strong and uniquely yours:
- **No-flatten (A3):** dedup requires agreement on **every required slot's** cosine, not one flattened vector — far harder to false-merge two genuinely different things than single-embedding dedup, since they must coincide across all required lenses at once. The same property that makes `Gτ` stomp prompt injection (`09`), reused for dedup.
- **Per-slot, calibrated `τ`:** identity slots strict, stylistic slots loose; `τ` is the same conformal-calibrated threshold Ward computes (`09 §3`) — dedup inherits the guard's calibration and provenance.
- **Heavy compression (A25):** collapsing recurrences into one constellation + a timestamp series is one of Calyx's largest storage wins, *measured-safe* — Assay confirms the merged event keeps its bits (`23 §4.4`), Ledger records every merge (`11`) so dedup is auditable and reversible.

## 6. Configuration at database creation (the explicit ask)

Dedup and temporal behavior are **options set when creating a Calyx vault/collection** (`03 §0`, `20 §3`):

```
DedupPolicy {
  mode: Off | Exact | TctCosine {            // TctCosine = the Gτ guard (yours)
    required_slots: Vec<SlotId>,             // which slots must agree (no-flatten)
    tau: PerSlot | Calibrated,               // dedup threshold (reuse Ward calibration)
    action: Collapse | Link | RecurrenceSeries,  // RecurrenceSeries = capture over time (§4)
  }
}
TemporalPolicy {
  enabled: bool (default true),
  decay: Linear | Exponential { half_life } | Step,   // E2
  periodic: { match_hour: bool, match_day_of_week: bool },  // E3
  sequence: { direction, multi_anchor },              // E4
  fusion_weights: { recency: 0.50, sequence: 0.35, periodic: 0.15 },  // tunable
  never_dominant: true,                               // AP-60, binding
}
```

These ride on `create_vault` / `create_collection` (`14`/`18`). A vault created with `DedupPolicy::TctCosine{ action: RecurrenceSeries }` and the default `TemporalPolicy` automatically captures recurring events compactly with full periodic understanding — no application code, the database does it (A22).

## 7. Why fully custom (no external dependency)

The entire subsystem is your constructs: the three temporal embedders (E2/E3/E4) + the AP-60 invariant + the Teleological Constellation + the cosine-`Gτ` guard + the calculus-of-association compression. **No external temporal foundation model, no external dedup library** — custom-fitted to do exactly what AGI + the Oracle require: understand time natively, capture and predict recurring events, deduplicate by grounded multi-lens agreement. The ultimate-database posture: every temporal and dedup capability is yours, baked in, automatic, measured.

## 8. API (summary; types in `18`)

```
create_vault(name, panel, DedupPolicy, TemporalPolicy) -> VaultId
ingest(vault, input, at: t) -> CxId | DedupMerge{into, occurrence}   // auto dedup + recurrence
recurrence_series(vault, cx) -> [occurrence (t_k, context)] + cadence + periodic_fit
periodic_recall(vault, hour?, day?) -> [events that recur then]
predict_next_occurrence(vault, cx) -> {t_hat, confidence}            // Oracle over time (21)
temporal_search(vault, query, window?, boost) -> [Hit]               // E2/E3/E4 post-retrieval boost
dedup_audit(vault, cx) -> [merges + tau + per-slot cos]              // Ledger-backed, reversible
```

**One sentence:** every Calyx database understands time through your three temporal embedders (recency, periodic, sequence) under the AP-60 never-dominant rule, captures the same event recurring over time as a deduplicated recurrence series whose repeat patterns the Oracle predicts — deduplication done strictly by your TCT cosine-`Gτ` guard, all configurable at database creation, all custom-fitted from your research.
