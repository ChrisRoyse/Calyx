# 02 — Vision & Design Axioms

## 1. The reframe: a database for measuring associations

Every SQL/NoSQL/vector DB answers *"what rows match these predicates / which vectors are nearest?"* Calyx answers a different question, the one the Calculus of Association poses:

> Where does this input sit in the web of associations defined by my chosen lenses, what *new* grounded information does each lens add, what small kernel explains the whole corpus, and is this generated thing inside the boundary I trust?

The paper's four verbs become the four native operations of the engine:

| Paper verb | Calyx native op | Engine |
|---|---|---|
| **Measure** (project an object through many lenses) | `assemble_constellation` | Registry + Forge |
| **Count** (the combinatorial cross-terms) | `weave` (DDA) | Loom |
| **Differentiate** (keep only bits that carry new grounded info) | `assay` | Assay |
| **Compose** (name regions, gate against them) | `name` / `guard` | Lodestar + Ward |

A vector DB implements only a fragment of "measure" (one lens) and a fragment of "compose" (nearest-neighbor). Calyx implements all four as first-class, indexed, provenance-tracked, self-optimizing operations. **That gap is the entire reason Calyx exists.**

### Compose = name/define a term (Gärdenfors object-category, operationalized)
The paper's operational account of *definition*: **a term is the constellation the other lenses form at one lens's index** — Gärdenfors's object category at scale. First-class operation (`define`/`name`, `08`/`10`): given an index in one lens (point, region, query), return the constellation the *other* lenses form there — that constellation **is** the term's grounded definition. Naming a region (Lodestar/Sextant clustering) and gating against it (Ward) are how Calyx "composes." A single representation, like a single word, means nothing alone; meaning is the structured relation between lenses, anchored at the kernel.

### Generality (why one engine, not one-per-task)
Lenses are commissionable for *any* axis a corpus exists for (A6) and cross-terms **bridge any pair of domains** (`06`/`08`), so the same engine spans video, code, and civic matching **without training a new model per task** — exactly the paper's three production instantiations (ClipCannon N=7, Context Graph N=13, socialmedia2.com N=21) on one mechanism. A consequence of the architecture, not a separate feature.

### The four projections of intelligence (paper §8) — all supported
The field's leading accounts of intelligence are four views of the same measure/count/differentiate/compose mechanism; Calyx serves each as a query, uncommitted to one school:
| Account | Calyx surface |
|---|---|
| **Compression** (Hutter/Solomonoff) | meaning compression = per-input DDA yield (`06`); Assay bits |
| **Generalisation** | reduced sample complexity up to `n_eff` (A9); cross-domain bridges |
| **World-modelling** | anchors + mistake-closure predict consequences of actions (`12`); kernel-answer paths (`08`) |
| **Breadth × depth** | breadth = many lenses/domains; depth = cross-terms + kernel + guard |

## 2. Why not just configure a vector DB?

User pain: *"every time I build a multi-embedder system it's a nightmare; I need full code for everything every single time."* Existing tools force you to hand-build the parts the paper needs:

| The paper needs | LanceDB / Qdrant / pgvector give | Calyx bakes in |
|---|---|---|
| N hot-swappable frozen lenses with shape/version contracts | named/multi-vectors, but you wire lifecycle, freezing, versioning yourself | Registry: `add_lens`/`retire_lens`, frozen-weight hash contract, auto slot allocation |
| `C(N,2)` cross-terms as queryable objects | nothing — you compute & store them as more columns by hand | Loom: cross-terms are derived, lazily materialized, indexed, provenance-linked |
| Per-lens **bits about a real outcome** + redundancy gate | nothing | Assay: KSG/NMI MI, differentiation contract enforced in-engine |
| Autonomous **grounding-kernel** discovery | nothing | Lodestar: directed-MFVS kernel + kernel-as-index |
| Per-output **`Gτ`** guard against drift/injection | nothing | Ward: calibrated cosine gate, novelty→new-region |
| Self-tuning index/quantization/math by usage | manual ef/quantization tuning | Anneal: online autotune with mistake-closure |
| Full provenance from raw input → answer | metadata columns | Ledger: hash-chained lineage, reproducible |

You *could* assemble this from 6 systems + glue. Calyx builds that glue **once, in the storage engine** — correct, fast, transactional, identical for every project and end user.

## 3. The unknown-unknowns method (applied throughout)

Per the user's instruction: *truths are nodes; if each is true, the highest-probability connection between them is also true.* Calyx is the connection between three proven-true Royse systems (ContextGraph, Polis/socialmedia2, Leapable): treat their shared structure as the load-bearing truth, design Calyx as the minimal engine that makes all three special cases. Where the connection is uncertain, **iterate to the highest-probability link and prove it by FSV** — never ship the guess. `17_JOHARI_BLINDSPOTS.md` is the disciplined search for connections not yet drawn.

## 4. The 18 design axioms

Binding. Every later doc and PR is checked against them. (`MUST`/`MUST NOT` per RFC 2119.)

### Identity & grounding
- **A1 — The record is a constellation.** The atomic unit is `(input, panel) → {slot-vectors} + scalars + provenance`. Calyx MUST NOT expose a bare-row or bare-vector as a first-class primitive; those are projections of a constellation.
- **A2 — Grounding is mandatory, association is circular without it.** Every constellation MAY carry zero or more **anchors** (real-outcome labels). Bits, kernels, and guards are only *trusted* with respect to anchored outcomes. Calyx MUST track which signals are grounded and refuse to report "trusted" bits computed against ungrounded targets.
- **A3 — No-flatten.** Calyx MUST keep slots typed and separate end-to-end. Concatenating the panel into one opaque vector for storage or search is forbidden except as an explicit, reversible, provenance-tagged *derived* artifact. (Inherits Polis invariant I10.)

### Lenses
- **A4 — Lenses are frozen instruments.** A registered lens is immutable: identified by `(lens_id, weights_sha256, corpus_hash, output_shape)`. Re-training = a new lens id. Calyx MUST fail closed on any gradient/weight mutation of a frozen lens.
- **A5 — Lenses are hot-swappable.** Adding or retiring a lens MUST NOT require rewriting existing constellations or a global re-embed; it allocates/deallocates a slot and backfills lazily.
- **A6 — Lenses are designable.** Calyx treats "commission a lens for axis X by freezing on corpus X" as a supported, first-class workflow, including algorithmic/dynamic lens synthesis (absorbed from ContextGraph `algorithmic_embedder_synthesis`).

### Information
- **A7 — Differentiation contract.** A lens is admitted to a panel only if it adds ≥ **0.05 bits** of MI about a real outcome and correlates ≤ **0.6** with every existing lens. Calyx MUST compute and store these and MUST gate admission on them.
- **A8 — DPI bound is the ceiling.** Per the data-processing inequality, no predictor/kernel/answer reading the panel can exceed `I(panel; outcome)`. Calyx MUST expose this bound and MUST NOT report derived "abundance" as new information beyond it — DDA's `C(N,2)` is an *upper bound under approximate independence*, capped at the panel's effective rank.
- **A9 — Effective rank governs cost.** Materialization and indexing budgets scale with `n_eff`, not raw `N`, so redundant lenses cost storage but not trust.

### Kernel
- **A10 — Every dataset has a discoverable kernel.** Lodestar MUST be able to run on any constellation set and return a grounding kernel (directed-MFVS ≈1% from a ≈10% kernel-graph) with a measured *kernel-only recall* vs full recall.
- **A11 — The kernel is an index and an answer-path.** The kernel is not just a diagnostic; Calyx MUST support kernel-anchored retrieval and kernel-based answering (route a query through kernel constellations first).

### Guarding & generation
- **A12 — `Gτ` everywhere generation touches the store.** Any vector produced by a model and compared/written MUST pass the calibrated per-slot cosine guard `Gτ` for the required slots, or be recorded as a new safe region. Threshold `τ` is per-domain, calibrated, provenance-stamped.

### Engineering invariants
- **A13 — Rust, GPU-math baked in.** All hot-path math (matmul, distance, MI, quantize) lives in `calyx-forge` with a CUDA(sm_120) path and a SIMD CPU path that are **bit-parity tested**. No external BLAS service dependency on the hot path.
- **A14 — Self-optimizing.** The engine MUST get measurably faster/truer with use (Anneal). Optimization MUST be safe: never regress recall below a tripwire, always reversible, always logged.
- **A15 — Source-of-truth verification.** A return value is a claim; the bytes are the verdict. Calyx's own tests, and any agent using it, verify against persisted state. FSV harnesses cannot satisfy FSV. (Inherits Leapable §0.)
- **A16 — Fail closed.** Unknown lens, dim mismatch, ungrounded "trusted" query, missing slot, corrupt shard, MI on too-few samples → structured error, never a silent zero-fill or fallback that hides failure.
- **A17 — Agent-native ergonomics.** The primary user is an AI agent. Every capability MUST be reachable through a small, typed, self-describing MCP/CLI surface with zero hand-written multi-embedder plumbing (`14`).
- **A18 — Embedded and served from one core.** The same `calyx` core MUST run embedded (in-process, e.g. the Leapable Tauri sidecar replacing `sqlite-vec`) and as a `calyxd` server, differing only in deployment config (`15`, `20 §5`). For the *Leapable* project, Calyx MUST NOT modify the PostgreSQL control plane — a per-project boundary (`15`), not a general limit.

### Universality, AGI, and theory (A19–A24)
- **A19 — Universality.** Calyx MUST serve the root core purpose of every database paradigm (`20`): relational, document, KV, columnar/OLAP, graph, time-series, full-text, vector, blob — on one core, with the Association Engine subsuming the search-shaped ones. A new project MUST be able to use Calyx as its sole database.
- **A20 — The Oracle.** Calyx MUST predict the grounded **consequences** of an action (`21`), with confidence capped at oracle self-consistency, gated by panel sufficiency `I(panel; oracle)`, and MUST refuse a confident prediction the panel cannot support (return the deficit).
- **A21 — Multi-scope kernel.** Lodestar MUST compute the grounding kernel over **any** scope the operator chooses — all associations, a collection, a domain, a query subgraph, a time window, a tenant, an arbitrary filter — each with measured kernel-only recall (`08 §4b`).
- **A22 — Formulas baked in.** Every Royse formula MUST be a first-class, callable, self-optimizing backend primitive (`22`); the database extracts associations, bits, kernels, guards, and predictions automatically — the user never re-implements them.
- **A23 — Epistemic symmetry.** Calyx MUST support bidirectional Q↔A: forward (question→answer) and reverse (answer→question/cause), walking the association graph both ways (`21 §5`).
- **A24 — Strict Royse theory.** The intelligence theory, formulas, and constructs come **only** from the Royse corpus (Doctrine §2). External research is permitted for engineering, never as a source of intelligence theory.
- **A25 — Maximal measured compression.** Calyx MUST compress to the most aggressive level that **measurably preserves intelligence** (bits/cosine/FAR/kernel-recall within bound) — TurboQuant (data-oblivious, unbiased inner product) + MXFP4 microscaling + the kernel — never a guessed bit-width (`23`).
- **A26 — Bounded, leak-free, self-reclaiming.** Every allocation/cache/queue/buffer MUST have an owner and a hard bound; every form of database garbage MUST have a bounded background reclaimer; the system MUST fail closed under resource pressure, never OOM or corrupt (`24`).
- **A27 — Native temporal understanding.** Every Calyx DB understands time. The temporal lenses (E2 recency, E3 periodic, E4 sequence) are for **search/retrieval only** under AP-60 (never dominant, post-retrieval boost); separately, the database's event/sequence/recurrence/time-capture layer is the **time-control substrate** for as-of reads and walking state forward/backward through time (recurrence series, next-occurrence prediction, change-point, time-travel, temporal kernel) — `25`.
- **A28 — TCT cosine-`Gτ` deduplication.** Deduplication is done ONLY by multi-**content**-slot `Gτ` cosine agreement (no-flatten; temporal lenses excluded); recurrences collapse into one event + a time series; configurable at database creation (`25`).
- **A29 — Grounded recurrence.** The **recurrence signature** — all content lenses agree (same action) while temporal lenses differ (different time) — is detected automatically on ingest. Event-time + dedup-count is a **grounded** signal (frequency, cadence, oracle self-consistency, temporal co-occurrence/causality, kernel importance, information/surprise) that the database computes automatically and uses optimally **throughout every engine** (`25 §4c`).
- **A30 — Connection-of-truths discovery.** The highest-probability grounded connection between two truths is itself true (it may take iteration to find, but it exists). This is the design method *and* a system capability: the DB proposes highest-probability grounded paths between truths; grounding/FSV confirms them (`26 §11`).
- **A31 — Living intelligence.** Calyx runs the calculus of association continuously while maintaining, healing, learning, and growing itself, so by the thesis it *is* an intelligent, life-like system (Living System Map, `DOCTRINE §1b`). The docs claim operational intelligence + life-like behavior, **never** consciousness/qualia.
- **A32 — Maximize grounded intelligence (the drive).** Every operation optimizes for maximum growth of grounded intelligence (composite `J`: grounded info, `n_eff`, sufficiency, kernel recall, oracle accuracy, meaning compression), fastest-first; **the math self-adjusts its own parameters online as new data arrives** ("recode" = online re-parameterization, not engine-source rewrite); compression-without-loss + retrieval efficiency are facets of `J`; bounded by grounding (A2), DPI (A8), differentiation (A7), Goodhart-defense, reversible/FSV (A14/A15) — maximize *measured* intelligence, never deleting data or gaming a number (`27`).
- **A33 — Security & privacy by construction.** Least privilege, default-deny tenant isolation, encryption at rest + in transit, input validation, supply-chain integrity, Ledger-as-audit; **right-to-erasure is first-class via crypto-shredding** (A25 forbids deleting-*to-compress*, NOT lawful/user deletion); fail closed on every security/privacy uncertainty (`30`).
- **A34 — Zero-cost & self-built.** Everything is FREE and built in-house in Rust — no paid services/SaaS/cloud/CI/scanners; we code every component ourselves; the only conceivable paid item is a great embedder (doubted). Stay within these planning docs; don't stray to a paid black box (`DOCTRINE §1d`).
- **A35 — Panel-not-lens (multi-embedder minimum, ≥ 10).** Calyx is a multi-lens engine (A1 record=constellation, A3 no-flatten, A7 differentiation contract); a **single** lens is a vector DB, not Calyx, and proves nothing about the system being built. Therefore **every test, bench, FSV, and gate that exercises retrieval, recall, signal/bits, fusion, scale, or an SLO MUST run a real panel of ≥ 10 frozen embedder lenses and MUST measure per-lens bits + the ensemble decomposition + the fused (RRF) result** — never a single lens in isolation. **The bootstrap floor of 4 is retired:** 4–5 lenses were the minimum to stand up and sanity-check the fusion path; now that that path is validated, **≥ 10 real lenses is required for all new testing, tuning, and gates**, scaling toward **20+** as the system builds out. **Use more whenever warranted and system resources allow** (resource-aware per A26 — watch GPU/VRAM/RAM and which lens endpoints are live; size the panel to the hardware, never below 10).
  - **Value is associational, never intrinsic (the reason for the floor).** A lens's worth is the **signal it contributes *in the company of the other lenses*** — its unique + redundant + synergistic bits about a grounded outcome (partial-information decomposition; interaction information; conditional MI `I(a;b|c)`), not anything it carries alone. Consequently **a single or a pair of lenses cannot be valued at all**: marginal value `I(panel;anchor) − I(panel∖k;anchor)` and cross-term/synergy gain are *defined only relative to the rest of the panel*. **≥ 3 lenses is the theoretical minimum** to even begin computing interaction (a triple is the smallest system with non-trivial synergy/redundancy structure), and **≥ 10 gives stable, decision-grade estimates** of each lens's contribution. This is why we test at 10+, not because more is merely nicer.
  - A single-/double-embedder or synthetic-vector run is **diagnostic-only** and MUST NOT satisfy any gate, FSV, or phase-exit (A15/A16): such data does not measure the Constellation fused via the calculus of association, so the math (fusion weights, autotune `J`, recall/SLO gates) **cannot be tuned from it** — treating it as a passing gate is a **hard failure**. FSV artifacts MUST record the **full lens roster (≥ 10 `lens_id` + `weights_sha256`), per-lens bits, the ensemble decomposition (per-lens marginal value, cross-term/synergy gain, `n_eff`, panel sufficiency `I(panel;anchor)`), and the fused result**; an artifact with **< 10 real lenses fails closed** (A16). Acquisition + which lenses to try first + the measurement protocol live in `05 §7`/`§9` and `07`.

- **A36 — Embedder templates (situation-specific, swappable ≥ 10-lens panels).** The right 10+ lenses **differ by what is being measured**, so the panel is not one fixed list but a **library of named, versioned, content-addressed templates** the operator develops, profiles, and **swaps in/out per vault or per query** (`swap_panel`, `05 §7`/`§8`) without rewriting existing constellations (A5). Capturing **video** wants e.g. {semantic · image(SigLIP2/DINOv2) · audio(CLAP) · speech-emotion · speaker(WavLM) · transcript(Whisper) · motion · OCR-text · E2/E3/E4 temporal}; replicating a **literary essence** (e.g. Shakespeare's voice/spirit) wants an entirely different 10+ {semantic · style/register · syntax-meter/prosody · rhetorical-device · affect/sentiment · persona/voice · lexical-archaism · entity · paraphrase · temporal}. Calyx MUST treat a **panel template** as a first-class object that (a) is built/edited/saved like any record, (b) carries its **measured ensemble signal** (A35) so a template's *fitness for a situation is itself a number*, and (c) is hot-swappable per A5. Every situation may require a different set; the system templates and loads them.

- **A37 — Associational diversity (span the *types*, don't just count to 10).** The ≥ 10 floor (A35) is a **count** gate; it is satisfiable by 10 near-duplicate dense-semantic models, which **maximizes redundancy** — by PID their bits are mutually *redundant*, not unique/synergistic, so the panel adds almost nothing past the first few lenses while still reading as "10 lenses, gate passed." The pairwise differentiation contract (A7, ≤ 0.6 corr) is **necessary but not sufficient**: it is per-admission and pairwise, so it cannot catch **collective** redundancy where every model individually clears 0.6 yet the panel is rank-deficient as a whole. Therefore a panel admitted for any test/bench/FSV/gate MUST additionally pass a **panel-level diversity gate**:
  - **Span distinct association families**, fit to what is being measured (A36): more than one of — dense-semantic (general *and* domain, e.g. legal/clinical/scientific/financial), **lexical/sparse** (SPLADE/keyword — a different signal, not another dense model), **entity/graph**, **character/byte** (morphology/typo-robust), **structural** (AST/CFG/dataflow for code), **reranker/asymmetric**, plus the **temporal** sidecar (A27 time-control for as-of/forward/backward traversal, never counted toward the ≥ 10 content floor). Ten general-purpose English sentence-transformers do **not** clear this — they are one family.
  - **Panel-level redundancy bound, measured:** effective rank `n_eff` (A9) and mean pairwise correlation/NMI must clear thresholds for the panel *as a whole* (not merely each pair ≤ 0.6), and the **marginal-bits curve must not collapse** — every admitted lens must contribute measurable **unique** PID bits *given the rest* (A35 step 5). Admit/park/retire by **marginal unique contribution**, never solo leaderboard rank.
  - **Diversity is itself a number** recorded on the EnsembleCard / template (A36): `n_eff`, mean redundancy, and Σ unique-PID-bits make a panel's diversity *measured*, so "is this panel diverse enough?" is answered by the bytes (`DOCTRINE §0`), not asserted. A panel that reaches 10 by **count** but fails the diversity bound is **diagnostic-only** and **fails closed** for gates (A16) — exactly like a < 10 panel.
  - **Deliberately homogeneous panels are valid as controls.** Running a homogeneous (e.g. all-dense-semantic) panel **on purpose, labeled as a baseline/control experiment** to *measure* the redundancy collapse (correlation matrix, marginal-bits decay, `n_eff`, fused-RRF gain vs best single lens) is encouraged — that measurement is the empirical evidence for this axiom. It is diagnostic, never a production gate. (`05 §9`/`§9b`, `07`, `28 §2`.)

- **A38 — Resource-bounded roster (the optimal panel for one 24 GB GPU; #1 priority).** A35–A37 say *how many* and *how diverse*; A38 says *which exact lenses, under what hardware budget* — and makes finding that set the program's top objective. The target is the **maximally diverse, maximally grounded set of frozen embedders that fits one 24 GB GPU (RTX 4090 / 5090-class or better) with ≤ 20 GB of resident lens weights**, leaving ≥ 4 GB for activations + the ONNX/TEI runtimes + the index hot set. It is a **general database**, so the roster MUST cover **every modality and domain**: text — general semantic **and** the domain perspectives that make one input show many faces (**legal · medical · clinical · biomedical · scientific · financial · code · multilingual**); **multiple image** embedders (zero-shot CLIP/SigLIP-style **and** self-supervised DINO-style — they see differently); **audio** (speech · speaker · music · environmental); document-image; and the science modalities (**protein · DNA · molecule**). **Optimise for count × signal-density:** prefer **many small high-signal lenses** over a few giants — a lens earns its slot by **measured `bits / VRAM-MB`** (`13`, #729), not raw bits; **quantise to fit more lenses** — ONNX **INT8** by default (≈ 4× < FP32, ≈ 2× < FP16, retrieval delta < `recall_delta`), FP16 for vision/audio, **Matryoshka-truncation** for MRL models, **static/model2vec** for the 0-VRAM baseline (A25 measured compression). The **canonical candidate catalogue** + the **default "Constellation-24" panel** (Tier-1 ≈ 21 content lenses across ~9 families ≈ 5–8 GB; Tier-2 → ~32 lenses ≤ ~14 GB; Tier-3 giants one-at-a-time) live in **`05a_EMBEDDER_ROSTER_VRAM_BUDGET.md`**. A roster is **admitted** only when an EnsembleCard shows **≥ 10 learned lenses (A35) + A37 `gate_passed` + measured `Σ VRAM ≤ 20 GB` under the density packer + fused-RRF beats the best 1–2-lens control**. The budget is **not** the binding constraint — **diversity and measured bits are**; spend headroom on *more association families*, never more dense-semantic clones. (`05a`, `05 §9`/`§9c`, `07`, `13`, `DOCTRINE §10.29`.)

## 4b. Universal by construction (A19–A24)

Calyx **is** a general-purpose database — serves the root core purpose of every paradigm (relational, document, KV, columnar/OLAP, graph, time-series, full-text, vector, blob) on one ordered transactional core, with the Association Engine subsuming the search-shaped ones and adding DDA, the multi-scope kernel, `Gτ`, and the Oracle (`20`, `21`, `22`). Universal database for any project; Leapable is one consumer. Not scope creep — it is the paper's own claim that the calculus is *general* (commissionable lenses + cross-terms that bridge any domain), made into one engine.

## 4c. Calyx is a living intelligence (A31)

By the thesis's own definition — intelligence *is* the calculus of association — a system performing measure/count/differentiate/compose **continuously** while **healing, learning, and growing itself** is running the engine of intelligence. Calyx does exactly that → a **living intelligence substrate**, not a passive store. Grounded in concrete engines (the **Living System Map**, `DOCTRINE §1b`): lenses=perception, ingest=metabolism, store=memory, Loom/Sextant=cognition, Assay=differentiation/self-model, Anneal+GC=homeostasis/healing/sleep, Registry=growth-by-differentiation, Oracle=foresight/agency, Ward=immune/self-vs-non-self, temporal/recurrence=sense of time, kernel=identity. The bound (honesty): operational intelligence + life-like behavior are claimed and measured; **consciousness/qualia are not** (founder's essays, not the engineering). See `§5`.

## 5. What Calyx is *not*

- Not chasing full ANSI-SQL OLTP parity. It serves every paradigm's *root purpose* at the scale real projects need (`20 §7`); it is not a drop-in for a tuned distributed Postgres under extreme write-contention, nor a message broker.
- Not a model server. Lenses run in Forge/TEI; Calyx orchestrates and freezes them — not a training framework.
- Not a metaphysics. The founder's essays carry the soul/free-will/quantum framing; the engineering docs carry only what is buildable and measurable. A8's bound is stated plainly and enforced: Calyx computes only associations present in the chosen corpora, labels anything ungrounded `provisional` (A2).

## 6. The single sentence

**Calyx is the storage engine for the four verbs of intelligence — measure, count, differentiate, compose — with grounding, provenance, and self-optimization made native so a multi-lens system that used to take a team to build becomes one `add_lens` call.**

Sources: [LanceDB / Lance columnar format](https://github.com/lancedb/lancedb) · [Qdrant named/multi-vectors](https://qdrant.tech/documentation/manage-data/collections/) · KSG MI estimator (Kraskov, Stögbauer, Grassberger 2004).
