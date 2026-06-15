# 17 — Johari Window: Blind Spots & Unknown Unknowns

Enumerate what we know, expose what we don't, search for connections we haven't drawn. The *boring* failure modes (durability, concurrency, corruption, cost) kill more systems than the exciting ones. This is the disciplined adversarial pass over the whole PRD.

## 1. The four quadrants

| | Calyx knows | Calyx doesn't know |
|---|---|---|
| **We know** | OPEN: the constellation/DDA/kernel/guard design, the hardware, the heritage systems | BLIND: the operational/distributed-systems failure modes below (§3) |
| **We don't know** | HIDDEN: capabilities we have but haven't surfaced (§4) | UNKNOWN: emergent risks we can only find by building + measuring (§5) |

The job: shrink BLIND with a risk register (§3), mine HIDDEN for free wins (§4), instrument UNKNOWN so reality tells us (§5).

## 2. The load-bearing truth (why we trust the connection)

Three Royse systems are proven-true: ContextGraph (multi-lens memory + ME-JEPA), Polis (21-slot guarded constellation), Leapable (provenance RAG vault + the aiwonder substrate). Calyx is the highest-probability connection between them. The *theory* (Calculus of Association) is sound and the *parts* exist. The risk is **not** "will the idea work" — it's **engineering a correct, durable, concurrent, affordable database** out of those parts. So this register is dominated by database-engineering risk, not theory risk.

## 3. BLIND — the risk register (known unknowns)

Each: risk · severity · mitigation · the FSV that proves it's handled.

### 3.1 Durability & data loss
- **Single-host, no-redundancy `hotpool`.** Sev: high. A dead NVMe loses hot state. Mitigation: WAL+fsync, ZFS snapshots, restic to the mirror, rebuildable derived indexes; **accept whole-host loss as posture** (inherited) and say so. FSV: kill mid-write, restore from restic, byte-readback. *Unsolved without a second box → flagged for a future HA doc; do not claim HA.*
- **Torn writes / partial group-commit.** Sev: high. Mitigation: WAL is SoT, atomic manifest `rename()`, replay discards torn tail. FSV: `kill -9` during commit; verify last-acked constellation present, un-acked absent.
- **Codebook/panel corruption.** Sev: med. Immutable + content-addressed; corruption is hash-detectable and restorable. FSV: flip a byte, reader fails closed.

### 3.2 Concurrency & consistency
- **~~Replacing PostgreSQL's control plane~~ — REMOVED FROM SCOPE.** Originally the highest-severity risk; locked scope (`15`) is **Vault-only**, so PostgreSQL is untouched and this entire risk class (multi-writer control plane, dual-write reconciliation, outbox correctness on a new backend, rollback) **no longer exists.** Calyx must only avoid *coupling* to PostgreSQL: the Vault adapter MUST implement the existing `vault-sqlite.ts` contract so the PG control plane sees no behavioral change. FSV: a real Vault swaps SQLite→Calyx and the control-plane queries/billing/listing for that Vault return identical results.
- **MVCC snapshot correctness across CFs.** Sev: high. A read must see a consistent seq across base/slots/anchors/ledger. Mitigation: single sequence number gates all CFs; derived carry build-seq. FSV: concurrent write+read race test asserting no partial-constellation read.
- **Cross-vault grants / isolation leak.** Sev: high (privacy). Default-deny; content-addressed lens sharing never shares vectors. FSV: cross-vault read without grant → denied, audited.

### 3.3 The intelligence machinery's own failure modes
- **Ungrounded everything.** Sev: high (trust). Without anchors, bits/kernel/guard are circular (A2). Mitigation: hard `provisional` tagging; high-stakes paths refuse provisional. Blind: users may never anchor → kernel/guard silently provisional. Mitigation: `grounding_gaps` nags the cheapest anchors; UI surfaces "ungrounded" prominently.
- **MI estimation on tiny samples.** Sev: med. KSG is noisy at low n. Mitigation: min-sample quorum (≥50), bootstrap CIs, fail-closed below quorum. Blind: high-dim k-NN bias even above quorum → random-projection pre-step + report CI, never a point estimate alone.
- **Differentiation contract gaming.** Sev: med. A lens can pass 0.05 bits / 0.6 corr on the probe set yet be useless in production. Mitigation: re-assay on live anchors (Anneal), park on decay.
- **Kernel ≠ 1% for real corpora.** Sev: low-med. The 1%/99% is a dictionary/lexicon observation, not a law. Mitigation: report *measured* kernel size + kernel-only recall; never assume 1%. Blind: pathological dense graphs → big kernel, low compression; Calyx must say so, not force a tiny kernel.
- **`Gτ` false-accepts / over-rejects.** Sev: med. Mis-calibrated `τ` lets injections in or blocks valid novelty. Mitigation: conformal calibration with guaranteed FAR bound, per-slot, drift-monitored; novelty→new-region not silent-accept. FSV: injection corpus blocked ≥99% at the calibrated point; valid-novelty path verified.
- **Anneal regressions / oscillation.** Sev: med. Self-tuning can thrash or quietly degrade recall. Mitigation: tripwires + shadow-first + reversible + Ledger-logged; bandit with hysteresis. FSV: 1e6-query soak shows latency win, no recall regression, no oscillation.
- **Frozen-lens drift via environment.** Sev: low. "Frozen" weights still produce different outputs if the runtime (CUDA/driver/ORT version) changes numerics. Mitigation: determinism mode + bit-parity golden tests pinned to the runtime; treat a runtime bump as a possible new `LensId` if numerics move beyond tolerance. *Subtle unknown-unknown — instrument it.*

### 3.4 Cost, scale, performance
- **GPU contention** with 3 resident TEI + marketplace on one RTX 5090. Sev: med. Mitigation: VRAM budgeter, Anneal yields, off-peak heavy jobs. FSV: search p99 SLO holds under concurrent TEI load.
- **Cross-term explosion.** Sev: med. `C(N,2)` storage if eager-materialized. Mitigation: lazy-by-default, `n_eff` budget, Assay-gated. FSV: storage stays `O(n·n_eff)`.
- **Billion-scale embedded** is unrealistic on a laptop. Sev: low. Embedded targets 1e6–1e7; server targets 1e9 (DiskANN/SPANN). Be explicit; don't promise laptop billion-scale.
- **Lens latency dominates ingest.** Sev: med. Mitigation: microbatching, resident lenses; a slow `external-cmd` lens bottlenecks. Surface per-lens cost in the capability card.

### 3.5 Build / process
- ~~**No `rustc` on aiwonder** → must cross-build + ship binary~~ **RESOLVED (2026-06):** Rust via rustup is installed on aiwonder; build natively. Cross-build retained only as an optional minimal-deploy path (`docs/implementation/01_AIWONDER_ENVIRONMENT.md`).
- **Scope = "a real database."** Sev: high (project). A durable, concurrent storage engine is a multi-year systems effort even Vault-only. Mitigation: phased `BUILD_DONE` (`19`); ship the embedded multi-lens Vault (V0–V2) as the first and sufficient milestone. *Biggest risk is over-reach; with control-plane replacement removed, scope is a contained Vault engine, not an RDBMS.*
- **FSV discipline at scale.** Sev: med. Manual SoT verification is slow but mandatory (banned harnesses). Mitigation: build readback *tools* (not FSV harnesses) that print bytes for a human/agent to judge.

## 4. HIDDEN — capabilities we have but haven't surfaced (free wins)

Fall out of the architecture; name them so they're not lost:
- **Cross-lens anomaly = data-quality + security signal.** Agreement-graph disagreement flags mislabeled data, poisoned inputs, drift — a built-in monitor (`06`/`10`).
- **The kernel = onboarding/summarization.** "Here's the 1% that explains your corpus" is a killer UX (`08`).
- **`grounding_gaps` = an active-learning labeler.** Tells you the cheapest outcomes to label to ground a domain — sell as guided labeling.
- **`reproduce()` = compliance/audit product.** Replayable answers are a regulated-industry feature (`11`).
- **Capability cards = a lens marketplace.** Lenses are content-addressed, shareable, profileable → a marketplace of commissioned lenses (ties to Leapable's marketplace).
- **Bits report = "should I even build this?"** Panel-sufficiency answers viability before training (`07`) — a research-accelerator.
- **Same engine embedded + served** = offline-first AND cloud with identical semantics (A18) — rare and valuable.

## 5. UNKNOWN — instrument the dark (emergent risks we can only measure)

Cannot reason these out; build minimally and let reality report (the FSV ethos):
- Real kernel sizes / kernel-only recall across genuinely different corpora (text vs code vs civic vs media). *Measure on 4 corpora before promising compression numbers.*
- Whether DDA's reduced-sample-complexity gain materializes on real downstream tasks, and at what `n_eff`.
- Whether `Gτ` calibration stays stable as a vault grows and lenses change panel_version.
- Long-horizon Anneal behavior (does self-tuning converge or wander over months?).
- Numerical reproducibility across driver/CUDA upgrades on Blackwell (the frozen-lens-drift unknown).
- User behavior: do end users ever anchor outcomes? If not, the whole trusted layer stays provisional — a *product* unknown bigger than any *engineering* one.
- Multi-tenant noisy-neighbor effects on the shared GPU at real load.

**Instrumentation requirement:** every UNKNOWN above has a metric in the Prometheus surface (`16`) and a line in `abundance_report`/`anneal.status` so unknowns become measured knowns as soon as data exists. The standing completeness critic: *what modality/claim/path haven't we measured yet?* — itself a recurring Anneal job.

## 7. Expanded blind spots (the now-larger system: universal + AGI + temporal + dedup + compression + arrays)

A fresh adversarial pass over everything now in scope. Each: hazard · mitigation · FSV.

### 7.1 Deduplication hazards (A28)
- **False-merge / data loss.** Two genuinely-different things agree on the required content slots → wrongly collapsed. Mitigation: strict per-slot `τ` on identity slots, **all merges reversible + Ledger-logged**, `dedup_audit` shows per-slot cosine; a merge is provisional until confirmed. FSV: inject near-but-distinct pair → not merged at calibrated `τ`; merged pair reversible byte-for-byte.
- **Merging across conflicting anchors (the dangerous one).** Two occurrences look like the same content but carry **opposite grounded outcomes** (one passed, one failed). Merging would corrupt grounding (A2). **Rule: dedup MUST NOT merge constellations with conflicting anchors** — they become distinct anchored events (or a contested region flagged for review). FSV: same-content/opposite-anchor pair stays separate.
- **Dedup vs the kernel.** Over-aggressive dedup could erase nodes the kernel needs. Mitigation: kernel computed after dedup; kernel-only recall gate (`08`) catches loss. FSV: recall unregressed post-dedup.

### 7.2 Temporal hazards (A27)
- **Timezone / DST for E3 periodic.** Hour-of-day / day-of-week depend on a timezone; naive UTC misreads local rhythms. Mitigation: store occurrence times as UTC **+ a vault/collection timezone**; E3 computes rhythm in the configured tz; DST handled by the tz database. FSV: an event recurring at local 14:00 across a DST boundary still scores periodic.
- **Backfill / historical "now".** Ingesting old events with `now`-relative recency (E2) poisons decay. Mitigation: ingest takes explicit `at: t`; E2 is relative to query-time, not ingest-time. FSV: backfilled event scores recency by its real `t`.
- **Time-travel vs GC/compaction.** As-of queries can't read past GC'd MVCC versions or compacted history. Mitigation: declare a **time-travel retention horizon**; beyond it, as-of fails closed with the horizon, not silently wrong. FSV: as-of before horizon → clear error, not stale data.
- **Recurrence-series unbounded growth.** A hot recurring event accrues millions of occurrences. Mitigation: series **rollup/retention** (downsample old occurrences to cadence stats), bounded like any store (A26). FSV: high-frequency event → series bytes bounded.
- **Next-occurrence overconfidence.** Small/irregular series → bad prediction. Mitigation: Oracle sufficiency gate + calibrated confidence + refuse below quorum (A20). FSV: sparse series → `sufficient:false`, not a confident guess.
- **Clock skew / ordering.** Mitigation: server-stamped monotonic seq for ordering, wall-clock only for human time (`24 §7 row 19`).

### 7.3 Universal-DB hazards (A19)
- **Cross-model transaction semantics.** One txn touching relational + constellation + graph collections — isolation, deadlock, partial visibility. Mitigation: single-writer-per-vault serializes; cross-collection writes are one transaction over Aster CFs (ACID per vault); declared isolation level. FSV: concurrent cross-model writers → no partial read, no deadlock.
- **General-data-layer schema evolution.** Adding/changing a record schema mid-life. Mitigation: schemaful = versioned migrations; schemaless = tolerant reads. FSV: schema change, old + new records both readable.
- **Blast radius (one engine, all paradigms).** A core bug affects everything. Mitigation: layered crates with hard interfaces; general data layer and Association Engine fault-isolate; per-collection degradation. FSV: fault-injection in one layer degrades only it.
- **Query-planner complexity / pathological cross-model plans.** Mitigation: planner cost caps + timeouts + explain; reject unbounded plans. FSV: adversarial cross-model query → bounded or rejected.

### 7.4 Compression / array hazards (A25)
- **Quant determinism across versions.** TurboQuant's random rotation + QJL seed must be **versioned and reproducible** or replay (`11`) breaks. Mitigation: seed content-addressed per slot/version; recorded in Ledger. FSV: re-quant with recorded seed = bit-identical.
- **Partial / ragged bundles in grouped GEMM.** Some slots absent (lazy backfill) → variable problem list mid-batch. Mitigation: absent slots explicit (`SlotState::Absent`), skipped in the group; never zero-filled (A16). FSV: mixed-completeness batch → correct per-constellation result.
- **Backfill storm on `add_lens` at scale.** A new lens triggers measuring billions of constellations. Mitigation: lazy, priority-ordered backfill (kernel/hot first), throttled, resumable (`05`). FSV: add lens on large vault → serving SLO holds, backfill completes.

### 7.5 AGI/Oracle hazards (A20/A23)
- **`reverse_query` hallucinating causes.** Answer→cause inversion could fabricate. Mitigation: reverse traversal over **grounded** association/causal edges only; results `provisional` unless anchored (A2). FSV: reverse a known cause → recovers it; ungrounded → labeled provisional.
- **Super-intelligence-predicate gaming (Goodhart).** Passing the six tiers without real capability. Mitigation: the predicate *includes* Goodhart-defense (`Gτ` + cross-lens anomaly) and online mistake-closure; tiers measured against held-out oracle outcomes. FSV: a gamed predictor fails the held-out tier.

## 8. Newly-named critical capabilities (HIDDEN quadrant — make these first-class)

The expanded system implies capabilities to name and ship, not leave latent:

| Capability | Falls out of | Product value |
|---|---|---|
| **Streaming / real-time ingestion** | temporal layer + events-over-time | the DB is natively an event-stream store, not just batch |
| **Reactive queries / triggers / subscriptions** | Ward novelty + temporal + new-region | "fire when a constellation enters region X / an event recurs / drift detected" — alerting, automation |
| **Change-point & drift monitoring** | agreement graph over time (`06`/`25`) | observability/quality product, data-poisoning + concept-drift alarms |
| **Time-travel / as-of audit** | MVCC snapshots keyed to time | compliance, debugging, "what did we know at time t" |
| **Next-occurrence forecasting** | Oracle over recurrence series | scheduling, predictive maintenance, demand forecast |
| **Root-cause analysis** | `reverse_query` (answer→cause) over grounded/causal/temporal edges | incident analysis, abductive reasoning |
| **Universal summarization** | multi-scope kernel — "the core of ANY slice" | onboard/summarize any dataset, domain, period, tenant on demand |
| **Grounded dictionary / ontology generation** | the `define` op (constellation at a lens index) | auto-build a grounded glossary/ontology of a corpus |
| **Event-stream compression / curation** | TCT dedup + recurrence series | trillion-scale corpus curation with **no external dedup tool** (yours only) |
| **Anomaly = the non-recurring** | dedup/recurrence (an outlier never merges) | fraud/novelty detection for free |
| **Federated / cross-vault kernel** | kernel scope = union of vaults | the grounded core across many projects/tenants |
| **Identity-locked generation as a service** | Ward + speaker/style lens (`09`) | voice/persona products with injection-proof character |
| **"Should I even build this?" viability check** | `I(panel; oracle)` sufficiency (`07`/`21`) | research accelerator — falsify before training |
| **Continual-learning substrate** | Anneal + anchors + recurrence + mistake-closure | a DB that genuinely learns from reality over time |
| **Explainability/interpretability product** | per-lens contribution + provenance + define + bits | every answer is justified, audited, and reproducible |

Each is a candidate first-class feature; the roadmap (`19`) surfaces the highest-value ones (streaming ingestion, reactive triggers, time-travel, universal summarization) as named milestones. Standing completeness-critic question: *what capability does this architecture imply that we still haven't named?*

## 9. The one caveat, stated plainly (inherited from the paper)

Calyx computes only the associations present in the chosen corpora; the `C(N,2)` abundance is an upper bound under approximate independence capped by the DPI; realized gain is reduced sample complexity up to the panel's effective rank; and **association is circular unless grounded** — what grounds Calyx is contact with real outcomes (anchors). Every "trusted" surface is gated on that grounding; where grounding is absent the system says `provisional` rather than pretending.
