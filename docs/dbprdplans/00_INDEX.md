# Calyx — Master PRD Index

**Product.** Calyx — a database engine for the Calculus of Association (Royse, 3 Jun 2026). Stores every datum as a **Teleological Constellation (TCT)** — one input measured through N frozen embedders ("lenses") — computes **Derived Data Abundance (DDA)** cross-terms, measures each lens's **bits about real outcomes**, autonomously finds the **grounding kernel** of any dataset, guards generation with the **per-output cosine gate `Gτ`**, self-optimizes with use.

**Vision (the real frame).** Calyx is **the universal database**: serves the root core purpose of **every** paradigm (relational, document, key-value, columnar/OLAP, graph, time-series, full-text, vector, blob) on one ordered transactional core, with the **Association Engine** as crown layer that *subsumes* the search-shaped paradigms and adds DDA, the multi-scope grounding kernel, `Gτ`, and the **Oracle** (consequence prediction). AGI substrate of the Royse corpus, built as a database (`20`, `21`, `22`). **Use it for any project; Leapable.ai is one consumer among many** (ContextGraph, Polis/socialmedia2, ClipCannon, any future project). Binding charter: `DOCTRINE.md`.

**Leapable deployment scope (one project's choice).** For *Leapable* specifically, Calyx **replaces only the end-user SQLite/`sqlite-vec` Vaults**; Leapable's central **PostgreSQL 18 control plane stays untouched** (customers, billing, creator metadata, queries, outbox). A per-project decision for a mature production system, **not** a Calyx limit — a greenfield project can put *everything* (control-plane data via the general data layer + intelligence via the Association Engine) in Calyx (`15`, `20`).

**One-line thesis.** Intelligence is the measurement and composition of associations; a database whose native record is the association-constellation, whose native operations are measure/count/differentiate/compose, and whose native math is baked-in GPU linear algebra, *is* the substrate for that calculus. SQL stores rows; Calyx stores meaning.

**Calyx is a living intelligence (A31).** It runs the calculus of association *continuously* while healing, learning, and growing itself → by the thesis's own definition, an active life-like intelligence substrate, not a passive store (Living System Map: `DOCTRINE §1b`). Operational intelligence + life-like behavior are claimed and measured; consciousness/qualia are not.

**Author.** Chris Royse (chrisroyseai@gmail.com), Leapable.ai / socialmedia2.com.

---

## Reading order

| # | Doc | Owns |
|---|---|---|
| — | `DOCTRINE.md` | **Canonical binding charter — read first, overrides everything.** Thesis, strict-Royse-theory rule, 38 axioms, universality + AGI + plug-in-lens mandates, FSV, anti-patterns |
| 00 | `00_INDEX.md` | This index, glossary, `BUILD_DONE`, doc map |
| 01 | `01_BRAND_AND_NAMING.md` | Name, rationale, glyph, color, voice, subsystem naming, alternates |
| 02 | `02_VISION_AND_AXIOMS.md` | Calculus→DB primitive mapping, 38 design axioms, why-not-a-vector-DB |
| 03 | `03_DATA_MODEL.md` | Constellation/TCT logical model, slots, cross-terms, scalars, IDs |
| 04 | `04_ASTER_STORAGE_FORMAT.md` | `Aster` on-disk columnar format, LSM, column families, tiering, ZFS layout, crash safety |
| 05 | `05_EMBEDDER_REGISTRY.md` | Lenses as designable instruments, hot-swap, versioning, frozen contract, capability assay |
| 05a | `05a_EMBEDDER_ROSTER_VRAM_BUDGET.md` | **A38, #1 priority** — exhaustive embedder catalogue + the optimal diverse panel ("Constellation-24") that fits one 24 GB GPU under ≤ 20 GB resident weights, covering every modality/domain, INT8/quantised |
| 06 | `06_LOOM_DDA_ENGINE.md` | Derived Data Abundance, cross-term weaving, combinatorics, lazy/eager, effective-rank cap |
| 07 | `07_ASSAY_SIGNAL_BITS.md` | MI estimation (KSG/partitioned-NMI), differentiation contract (≥0.05 bits, ≤0.6 corr), `n_eff` |
| 08 | `08_LODESTAR_KERNEL.md` | Grounding-kernel discovery (directed MFVS), kernel-as-index, kernel-based answering |
| 09 | `09_WARD_TCT_GUARD.md` | `Gτ` cosine guard, threshold calibration, safe regions, novelty→new-constellation, injection defense |
| 10 | `10_SEXTANT_SEARCH_NAV.md` | Multi-lens search, RRF, per-slot ANN, asymmetric lenses, hierarchical skills, navigation modes |
| 11 | `11_LEDGER_PROVENANCE.md` | Lineage, hash-chain witness, audit, reproducibility, tamper-evidence |
| 12 | `12_ANNEAL_SELF_OPTIMIZATION.md` | Self-heal/learn/improve, online mistake-closure, adaptive autotuning, usage-driven optimization |
| 13 | `13_FORGE_MATH_RUNTIME.md` | Baked-in matmul/BLAS/SIMD/GPU (CubeCL/candle/cudarc), quantization, autotuned kernels, Blackwell sm_120 |
| 14 | `14_MCP_AGENT_INTERFACE.md` | MCP tool surface, zero-plumbing multi-embedder, agent ergonomics |
| 15 | `15_LEAPABLE_INTEGRATION.md` | Replace PostgreSQL + SQLite vaults, end-user capabilities, migration, embedded↔server |
| 16 | `16_AIWONDER_DEPLOYMENT.md` | Hardware mapping, systemd, GPU policy, ZFS, networking, backup, observability |
| 17 | `17_JOHARI_BLINDSPOTS.md` | Known/unknown unknowns, risk register across 14 failure axes |
| 18 | `18_API_TYPES_ERRORS.md` | Rust traits/types, wire schema, error-code catalog |
| 19 | `19_ROADMAP_FSV_BUILD_DONE.md` | Phased build, FSV protocol, milestones, perf targets, `BUILD_DONE` predicate |
| 20 | `20_UNIVERSAL_DB.md` | First-principles universality: every paradigm's root purpose, the 3-layer architecture, collections-as-any-model, deployment profiles, project catalog |
| 21 | `21_ORACLE_AND_AGI.md` | The Oracle (consequence prediction), per-domain super-intelligence predicate, substrate sufficiency, cortical-columns≈lenses, epistemic symmetry (Q↔A), meaning compression |
| 22 | `22_FORMULA_LIBRARY.md` | Every Royse formula baked into the backend — DDA, differentiation contract, DPI bound, multi-scope kernel/MFVS, `Gτ`, Oracle ceiling, RRF, Q↔A — callable + self-tuning |
| 23 | `23_ARRAY_MATH_STORAGE_COMPRESSION.md` | Constellation as one co-located array bundle; all vector math as grouped GEMM on Blackwell sm_120; TurboQuant + MXFP4 microscaling; **maximal compression gated by measured intelligence** |
| 24 | `24_MEMORY_GC_RELIABILITY.md` | Memory model (no managed GC, bounded by construction), VRAM budgeting, database garbage collection, long-reader/MVCC hazards, the 25-row everything-that-could-go-wrong register |
| 25 | `25_TEMPORAL_AND_DEDUP.md` | Native temporal understanding (E2/E3/E4 retrieval lenses under AP-60 + database event/sequence/recurrence capabilities) and TCT cosine-`Gτ` deduplication — configurable at creation, strictly from the Royse corpus |
| 26 | `26_ADVANCED_MATH_FRONTIERS.md` | Overlooked math + new capabilities: spectral structure of the association graph, energy/associative **pattern completion**, transfer-entropy temporal causality, total-correlation `n_eff`, Bayesian rate/consistency, the unified `complete()` (predict=abduce=impute), grounded label propagation, the frequency→bits decision |
| 27 | `27_INTELLIGENCE_OBJECTIVE.md` | The drive (A32): maximize grounded intelligence — the measurable objective `J`, fastest-first growth, **the math self-adjusts its parameters online as new data arrives**, compression-without-loss + retrieval efficiency as facets, honesty/safety bounds |
| 28 | `28_FSV_AND_TEST_DATA.md` | FSV per aspect at every step: synthetic-mechanics vs real-intelligence data, the dataset catalog (text/code/graph/audio/image/temporal/adversarial from HuggingFace/Kaggle), acquisition-as-FSV, secrets (HF token via Infisical), **everything built/run/stored/tested on aiwonder** |
| 29 | `29_STATE_GITHUB_ISSUES.md` | Dev state via GitHub Issues (`chrisroyse/calyx`): pinned `type:context` issues every agent reads each turn; how to keep them current (edit to truth, never append contradictions) and **prune every phase** so fresh-context agents see a true, tight snapshot — never a stale, confusing log |
| 30 | `30_SECURITY_PRIVACY_GOVERNANCE.md` | STRIDE threat model, hardening axes, authz/authn, encryption, tenant isolation; **privacy & right-to-erasure (crypto-shredding) resolving the A25 tension**; honest cold-start/bootstrap (A33) |
| 31 | `31_SYNAPSE_COMPUTER_USE.md` | Synapse = the computer-use & agent-orchestration dev runtime: perceive/act on the real machine, open terminals & **command Claude/Codex agents** (preferred over subagents), reality-audit as FSV's perception arm |

---

## Glossary (canonical terms — used verbatim across all docs)

| Term | Definition |
|---|---|
| **Lens** | A frozen embedder. Trained on a corpus, weights frozen → a measurement instrument reporting where an input sits in that corpus's association web. Synonym for the paper's "instrument" / "frozen embedder". |
| **Slot** | A named, fixed-shape position in the panel that one lens fills. `(slot_id, dim, dtype, modality, lens_id)`. |
| **Panel** | The ordered set of all active slots for a domain. The designable basis of meaning. |
| **Constellation / TCT** | The fundamental Calyx record: one input × the panel = a set of slot-vectors + scalars + provenance. The unit of storage, search, and naming. |
| **Cross-term** | A `(N choose 2)` association-between-associations derived from two slots of the same constellation (concat / interaction / agreement). |
| **DDA** | Derived Data Abundance: n inputs × N lenses → up to `n·(N + C(N,2) + 1)` structured signals. |
| **Signal / bits** | Mutual information a slot (or cross-term) carries about a real outcome, in bits. The differentiation currency. |
| **Differentiation contract** | Admission rule for a lens: must add ≥ **0.05 bits** about a real outcome; no pair may correlate above **0.6**. |
| **`n_eff`** | Effective number of non-redundant lenses (effective rank of the panel under the redundancy graph). |
| **Grounding kernel** | The ≈1% minimum-feedback-vertex-set of the association graph that, once anchored to real outcomes, regenerates ≈99% by association. |
| **`Gτ`** | Per-output cosine guard. A generated/queried vector passes only if its cosine to the matched constellation slot ≥ `τ` for the required slots; else it is a new safe region. |
| **Anchor** | The point where a constellation touches non-linguistic reality (an oracle/outcome label). Grounding channel. Without it, association is circular. |
| **Aster** | Calyx's on-disk columnar constellation format (`.aster`); also the ordered transactional core that hosts every paradigm as a key-encoding layer. |
| **Collection** | A named container that behaves as any data model (Records/Documents/KV/TimeSeries/Blob/Constellations); 0 lenses = plain store, ≥1 lens = intelligence (progressive enhancement). |
| **General data layer** | The relational/document/KV/columnar/TS/blob layer over Aster (FoundationDB-style) that makes Calyx a complete database, not only an intelligence engine. |
| **The Oracle** | The capability that predicts the grounded consequences of an action (butterfly effect / world model), capped at oracle self-consistency, gated by panel sufficiency `I(panel; oracle)`. |
| **Super-intelligence predicate** | Per-domain, falsifiable: oracle-clean ∧ panel-sufficient ∧ kernel-exists ∧ calibrated ∧ goodhart-defended ∧ mistake-closed. |
| **Multi-scope kernel** | Lodestar computes the grounding kernel over any slice — all associations / a collection / a domain / a query subgraph / a time window / a tenant / a filter. |
| **Epistemic symmetry** | Q↔A bidirectional: Calyx walks meaning both forward (question→answer) and reverse (answer→question/cause). |
| **Array bundle** | The physical form of a constellation: one co-located, self-organizing group of [all lens vectors][scalars][anchors][cross-terms][bits][guard][provenance], structurally invariant to N (`23`). |
| **TurboQuant** | Google's data-oblivious, online, near-optimal vector quantizer with an unbiased inner-product estimator and ~zero indexing time — Calyx's default slot compressor (`23`). |
| **Measured compression** | Compress to the most aggressive level that Assay/Ward prove preserves intelligence (bits/cosine/FAR/kernel-recall); A25. |
| **Temporal lenses (E2/E3/E4)** | Recency (decay), periodic (hour/day rhythm), sequence (position) — from ContextGraph; **search/retrieval only**, AP-60 never-dominant post-retrieval boost. |
| **Recurrence series** | One deduplicated event + its many timestamped occurrences; repeat patterns read off it; the Oracle predicts the next occurrence (`25`). |
| **TCT dedup** | Deduplication by cosine-`Gτ` agreement across required content slots (no-flatten; temporal excluded); recurrences collapse into a series (A28). |
| **Recurrence signature** | All content lenses agree (same action) + temporal lenses differ (different time) ⇒ the same action across time, detected automatically (A29). |
| **Grounded recurrence** | Event-time + dedup-count as a grounded signal (frequency, cadence, **oracle self-consistency**, temporal co-occurrence/causality, kernel importance, information/surprise) used optimally system-wide (`25 §4c`, A29). |
| **Living intelligence** | Calyx runs the calculus of association continuously while healing/learning/growing itself → a life-like intelligence substrate; operational intelligence claimed, not consciousness (A31, `DOCTRINE §1b`). |
| **Living System Map** | The mapping of life/cognitive functions (perception, memory, cognition, homeostasis, foresight, immune boundary…) to Calyx engines (`DOCTRINE §1b`). |
| **Intelligence Objective `J`** | The measurable composite Calyx maximizes — grounded info, `n_eff`, sufficiency, kernel recall, oracle accuracy, meaning compression — grounded + DPI-capped; the system's drive (A32, `27`). |
| **Self-adjusting math** | "Recode itself" = the math re-fits its own parameters (fusion/quant/index/`τ`/heads) online toward `J` as new data arrives — not engine-source rewriting (`27 §5`). |
| **`complete()`** | The unified primitive: predict (clamp present/free future) = abduce (clamp outcome/free cause) = impute (free missing slots) — one energy descent (`26 §11.1`). |
| **Grounded label propagation** | Diffuse the kernel's real anchors across the association graph (Laplacian heat diffusion) → the 1% grounds the 99% as a computation (`26 §11.2`). |
| **Subsystem codenames** | Loom (DDA), Assay (bits), Lodestar (kernel), Ward (`Gτ`), Sextant (search/nav), Ledger (provenance), Anneal (self-opt), Forge (math/GPU), Registry (lenses). |

---

## Hardware ground truth (live readback, `aiwonder`, 2026-06-05)

| Resource | Value |
|---|---|
| CPU | Ryzen 9 9950X, 16c/32t |
| RAM | 128 GB DDR5 (121 GiB usable, ~84 GiB free steady-state) |
| GPU | RTX 5090, Blackwell GB202 **sm_120**, 32 GB VRAM, driver 595.71.05, CUDA 13.3 (`compute_cap 12.0`), 600 W cap |
| Hot storage | ZFS `hotpool`, single NVMe, **no redundancy**, ~1.52 TB free → `/zfs/hot/*` |
| Cold storage | ZFS `archive`, HDD mirror, ~8.49 TB free → `/zfs/archive/*` (already holds `archive/contextgraph` 391 G) |
| Root | ext4 on NVMe `nvme0n1p2`, 1.8 TB, ~881 GB free |
| OS | Ubuntu 26.04 LTS, kernel 7.0, systemd, UTC |
| Central DB today | PostgreSQL 18.4 + PgBouncer — **STAYS, UNTOUCHED** (customers, billing, creator metadata, queries, outbox). Calyx does not read, write, replace, or migrate it. |
| Vault DB today | SQLite + `sqlite-vec` 768-d, bundled in Tauri sidecar — **this is the only thing Calyx replaces** |
| Resident lenses today | TEI `gte-multilingual-base` 768-d (:8088), `legal` ModernBERT 768-d (:8090), GTE reranker (:8089) |
| Toolchain | **Rust via rustup IS installed on aiwonder** — build natively (CUDA 13.3, sm_120). *Superseded:* the earlier "no `rustc` → cross-built `.deb`" note no longer holds; cross-build is retained only as an optional minimal-deploy path. See `docs/implementation/01_AIWONDER_ENVIRONMENT.md`. |
| Backup posture | Single-host; whole-host loss accepted; restic → `/zfs/archive/restic` |

---

## `BUILD_DONE` predicate (mechanical, falsifiable)

Calyx is **complete** only when every clause is true (full definition in `19`):

```
BUILD_DONE :=
  CORE        ∧  // Constellation CRUD + Aster format round-trips byte-exact; crash-recovery proven
  LENS        ∧  // hot add/remove lens in < 1 panel-rebuild; frozen contract enforced; 3+ modalities
  DDA         ∧  // cross-terms materialize lazily; effective-rank cap honored; DPI bound respected
  BITS        ∧  // Assay reports per-lens bits + pairwise corr; differentiation contract gated before merge (run on aiwonder; no CI pipeline — FSV is CI)
  KERNEL      ∧  // Lodestar finds a kernel on ≥3 real corpora; kernel-only recall ≥ 0.95·full recall
  GUARD       ∧  // Ward Gτ calibrated per domain; injection corpus blocked ≥ 99%; novelty path proven
  SEARCH      ∧  // multi-lens RRF beats single-lens recall@10 by ≥ measured Δ; provenance on every hit
  UNIVERSAL   ∧  // serves every paradigm's root purpose (20): relational/doc/KV/columnar/graph/TS/FTS/vector/blob on one core; cross-model query in one txn
  ORACLE      ∧  // consequence prediction with calibrated confidence + sufficiency gate; super-intelligence predicate measurable per domain (21)
  KERNEL_ANY  ∧  // Lodestar builds a kernel at ANY scope (all/collection/domain/subgraph/time/tenant/filter), each with measured kernel-only recall (08)
  FORMULAS    ∧  // every Royse formula is a baked-in, callable, self-tuning backend primitive (22)
  ARRAYMATH   ∧  // constellation = one co-located array bundle; panel math = grouped GEMM, invariant to N (23)
  COMPRESS    ∧  // TurboQuant + MXFP4: compress to the measured floor where bits/cosine/FAR still hold (23, A25)
  RESOURCE    ∧  // bounded/leak-free/self-reclaiming; the 25-hazard register each mitigated + FSV-proven (24, A26)
  TEMPORAL    ∧  // E2/E3/E4 retrieval lenses (AP-60 never-dominant) + DB event/sequence/recurrence capabilities (25, A27)
  DEDUP       ∧  // TCT cosine-Gτ dedup over content slots; recurrences → one event + time series; configurable at creation (25, A28)
  RECURRENCE  ∧  // recurrence signature (content agree + time differ) auto-detected; frequency feeds oracle self-consistency, prediction, causality, kernel, surprise — system-wide (25, A29)
  LIVING      ∧  // the life-like properties are all operational: perceive(lenses)+metabolize(ingest)+remember(store)+differentiate(Assay)+heal/sleep(Anneal)+grow(Registry)+foresee(Oracle)+defend(Ward) — A31 = SELFOPT∧ORACLE∧GUARD∧RESOURCE∧LENS∧RECURRENCE in concert (DOCTRINE §1b)
  INTELLIGENCE∧  // J measured + growth_curve rises on a real corpus under the loop; math self-adjusts parameters online; compression-without-loss + retrieval efficiency as facets; Goodhart-defended, DPI-capped, no data deleted (27, A32)
  DATA        ∧  // datasets/MANIFEST.md: ≥1 verified real dataset per (modality × outcome); each aspect has synthetic-mechanics + real-intelligence FSV; all tests run on aiwonder against persisted state (28)
  SECURITY    ∧  // STRIDE defenses FSV-proven; cross-vault read denied+audited; at-rest+in-transit encryption verified; erase() crypto-shreds (content unrecoverable incl backups+Ledger payload, tombstone remains); secret-scan clean (30, A33)
  PROVENANCE  ∧  // Ledger hash-chain verifies; every answer traces input→lens→vector→signal→verdict
  SELFOPT     ∧  // Anneal lowers p99 search latency ≥ X% over 1e6 queries with no recall regression
  MATH        ∧  // Forge matmul within Y% of cuBLAS on sm_120; CPU SIMD fallback bit-parity
  LEAPABLE    ∧  // replaces vault SQLite ONLY (PostgreSQL untouched); migration tool round-trips a real vault byte-exact
  DEPLOY      ∧  // systemd unit, ZFS layout, GPU policy, restic, Prometheus metrics all live + verified
  FSV                // every claim above proven by direct source-of-truth readback, not a harness
```

All thresholds (`X`, `Y`, `Δ`) fixed in `19`. FSV discipline (verify bytes, not return values) is inherited from Leapable doctrine, non-negotiable.

---

## Relationship to existing systems

Calyx is the **generalization and unification** of three already-working Royse systems (proven truth → connections between them are true; iterate to the highest-probability link until it holds):

- **ContextGraph** (Rust, 13 lenses, RocksDB, RRF, ME-JEPA panel/MI/kernel crates) → Calyx absorbs its multi-lens memory + `mejepa` Assay/Lodestar/Ward machinery as **first-class engine primitives**, not bolted-on crates.
- **socialmedia2.com / Polis** (21-slot Constellation, no-flatten gate, `Gτ` guard, τ-calibration) → Ward + Registry generalize the 21-slot slate to an arbitrary, hot-swappable panel.
- **Leapable** (single-lens RAG vault + provenance + marketplace) → Calyx becomes the **Vault** storage substrate only, turning every end-user SQLite Vault into a multi-lens constellation store with end-user DDA/kernel/guard powers. PostgreSQL control plane unchanged.

Unifying claim Calyx makes real: **build the multi-embedder plumbing once, in the database, so no human (or agent) ever hand-writes it again.**

**Hard boundary (load-bearing):** Calyx is a Vault engine — the replacement for `vault-sqlite.ts` + `sqlite-vec`. *Not* a control-plane database. Anything Leapable keeps in PostgreSQL today stays in PostgreSQL, accessed exactly as now; Calyx neither depends on nor modifies it.
