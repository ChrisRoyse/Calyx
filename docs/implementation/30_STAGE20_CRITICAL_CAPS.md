# Stage 20 — Critical Capabilities (PH72)

The high-value capabilities the architecture *implies* but that need explicit
first-class wiring (PRD `17 §8`): streaming ingestion, reactive triggers/
subscriptions, time-travel/as-of audit, and universal summarization (multi-scope
kernel). Each falls out of engines already built; this stage names and ships the
highest-value ones. Cross-cutting; depends on temporal (S9), kernel (S6), MVCC
(S1).

---

## PH72 — Streaming + reactive + time-travel + universal summarization
- **Objective.** Ship the four named capabilities, each FSV-proven on a real
  stream/corpus.
- **Deps.** PH41 (temporal/recurrence), PH34 (multi-scope kernel), PH08 (MVCC).
- **Deliverables.**
  - **Streaming / real-time ingestion** — the DB as a native event-stream store
    (temporal layer + events-over-time); quantize-on-the-fly (TurboQuant, data-
    oblivious).
  - **Reactive queries / triggers / subscriptions** — "fire when a constellation
    enters region X / an event recurs / drift detected" (Ward novelty +
    temporal + new-region); `subscribe`/`observe_delta`-style streams.
  - **Time-travel / as-of audit** — read the vault (and the panel/kernel) as it
    was at time `t` (MVCC snapshots keyed to time); declare a retention horizon,
    beyond which `as_of` fails closed (not silently wrong).
  - **Universal summarization** — "the core of ANY slice" via the multi-scope
    kernel (`08 §4b`): summarize any dataset/domain/period/tenant on demand.
- **Key tasks.** reactive triggers as a bounded, audited subsystem; as-of within
  the retention horizon; streaming under backpressure (A26); each capability
  Ledger-provenanced.
- **FSV gate.** each capability proven on a real stream/corpus on aiwonder:
  a recurring event fires a trigger; `as_of(t)` returns the historical state (and
  fails closed before the horizon); a slice's kernel summarizes it (read the
  outputs).
- **Axioms/PRD.** `17 §8`, A27, A21, `04 §6`, A26.

---

## Stage 20 exit
The architecture's implied capabilities are named and shipped — streaming
ingestion, reactive triggers, time-travel, universal summarization — each FSV-
proven. The standing completeness-critic question continues: *what capability
does this architecture imply that we still haven't named?* (a recurring Anneal
job, PRD `17 §5`).

---

## After Stage 20 — `BUILD_DONE`

With Stages 0–20 complete and every gate FSV-proven on aiwonder, the PRD's
mechanical `BUILD_DONE` predicate (`dbprdplans/19 §5`) holds: an agent can
`add_lens`, `ingest`, `anchor`, then `search`/`kernel_answer`/`guard` a real
Leapable vault — multi-lens, kernel-grounded, drift-guarded, fully provenanced,
self-optimizing — on aiwonder, with every claim proven by reading the bytes.
Calyx is the universal, association-native, self-optimizing, living-intelligence
database the PRD specifies.
