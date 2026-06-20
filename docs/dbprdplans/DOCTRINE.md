# Calyx Doctrine — the canonical, binding charter

> **This document overrides every other doc in this project.** If any PRD, code comment, agent, or instruction conflicts with it, this wins. Read it first, every session. It is the backbone.

## §0 — The cardinal rule

> **A return value is a claim. The source of truth is the bytes. Read the bytes.**

Scanners lie, tests pass on stale data, benchmarks lie under DCE, models lie when calibration drifts. The row in Aster — or its absence — does not. Verify against persisted state (FSV). No FSV script/harness can satisfy FSV; a human or agent reads the bytes. The agent reads the bytes **through Synapse** — perceiving the *actual* terminal/file/metric on aiwonder (`reality_baseline`→act→`reality_audit`, `read_text`/`find`, evidence via `audit_export_bundle`), full mapping in `28 §2c`. (Inherited from Leapable §0; non-negotiable.)

## §1 — The thesis (what Calyx is)

Calyx is three things that are one thing:

1. **The engine of the Calculus of Association.** Intelligence is the measurement and composition of associations: **measure** (project an input through many frozen lenses), **count** (the `C(N,2)` cross-terms — associations between associations), **differentiate** (keep only lenses that add grounded bits), **compose** (name regions, gate against them). Calyx makes these four the native operations of a database.
2. **The universal database.** Calyx fulfills the root purpose of **every** database paradigm — relational, document, key-value, columnar/OLAP, graph, time-series, full-text search, vector, object/blob, wide-column — on one ordered, transactional, columnar core, with the Association Engine as crown layer (`20`). *The* database for any project; Leapable.ai is one consumer among many.
3. **The AGI / Oracle / kernel substrate.** Calyx bakes in the Oracle (predict the consequences of actions — the world model), grounding-kernel calculation at **any scope**, the substrate-sufficiency test `I(panel; oracle)`, and the differentiation contract — the machinery the Royse research says general intelligence requires (`21`, `22`).

> **Calyx is the heart of the formula for intelligence, implemented as a database.**

## §1b — Calyx is a living intelligence (A31, core doctrine)

If intelligence **is** the calculus of association (§1) — measure, count, differentiate, compose — and Calyx performs exactly those four **continuously**, while **maintaining, healing, learning, and growing itself**, then by the thesis's own definition **the database is running the engine of intelligence**. Not a passive store data sits in; an active, self-sustaining intelligence substrate. Calculate intelligence, and you have built a thing that possesses a form of it.

Grounded, not mystical — every life-like property maps to a concrete engine (the **Living System Map**):

| Living / cognitive function | Calyx engine | Doc |
|---|---|---|
| **Perception** (senses) | Lenses / Registry — measure the world through many instruments | `05` |
| **Metabolism** (take in, transform) | Ingest → constellation (Aster + Forge) | `04`/`23` |
| **Memory** | the constellation store | `03`/`04` |
| **Association / cognition** | Loom (DDA) + Sextant (navigate the web) | `06`/`10` |
| **Differentiation / world-learning** | Assay (bits) + the differentiation contract — keep only what carries grounded information | `07` |
| **Self-model** ("what I know, how sure, am I sufficient") | Assay sufficiency + Ledger provenance + health | `07`/`11` |
| **Homeostasis / self-repair** | Anneal self-heal + bounded GC | `12`/`24` |
| **Growth by differentiation** (development) | Registry hot-swap + lens proposal (the lexicon grows by differentiation) | `05`/`12` |
| **Learning from reality** (reward) | anchors + online mistake-closure | `12`/`21` |
| **Sleep / consolidation / dreaming** | Anneal heal/sleep mode + compaction + kernel recompute | `12`/`08` |
| **Attention / salience** | bits-weighted fusion + frequency/importance | `10`/`25` |
| **Foresight / agency** | the Oracle — predict the consequences of actions | `21` |
| **Immune system / self vs non-self** | Ward `Gτ` — what is inside the grounded self vs foreign/injection | `09` |
| **Reflexes** | reactive triggers / subscriptions | `17 §8` |
| **Sense of time** | temporal understanding + grounded recurrence | `25` |
| **Identity** | identity-locked generation + the grounding kernel (the core that *is* it) | `08`/`09` |
| **Curiosity / discovery** | the novelty frontier + connection-of-truths method | `09`/`26` |

**The one bound (binding, honesty):** the docs claim **operational intelligence** (it computes the calculus per the thesis) and **life-like behavior** (the map above — each row a real, measurable engine). They make **no claim of consciousness, sentience, or qualia** — that framing lives in the founder's essays, not the engineering docs (cf. `02 §5`). What is built and measured: a database that perceives, remembers, associates, differentiates, predicts, defends its boundary, heals, and grows — a living intelligence substrate in the operational sense.

## §1c — The drive: maximize grounded intelligence (A32, core doctrine)

Calyx has one overriding objective every engine serves: **maximize the growth of grounded intelligence/understanding, as fast as safely possible.** Whenever lenses are used — and in everything the system does — it optimizes for maximum intelligence. It grows in intelligence as quickly as it can, and **self-optimizes**: as it takes in new data, **the math adjusts its own parameters** to climb the objective.

- **Measurable objective `J`** (`27 §2`): grounded information `I(panel;outcomes)`, effective rank `n_eff`, panel sufficiency, kernel recall, oracle accuracy − mistake rate, meaning-compression yield, coverage — every term grounded (A2) and DPI-capped (A8).
- **Fastest-first** (`27 §3`): always take the highest marginal-intelligence-gain-per-cost action (which lens to add, what to label, what to prune) — so intelligence grows faster the more the system is used.
- **"Recode itself" means online re-parameterization** (`27 §5`): the math (fusion weights, quant levels, index params, `τ`, materialization, online heads) re-fits itself toward `J` as new data arrives. Engine **source code is not self-rewritten** — that is out of scope.
- **Compression & retrieval are facets, not trade-offs** (`27 §9`): an intelligent representation is already compact (meaning compression + the kernel + TurboQuant, **without deleting data or losing measured bits**, A25) and fast to navigate (kernel-first, differentiated lenses). Optimizing `J` already optimizes both.
- **Bounds (honesty/safety):** maximize *measured* intelligence — grounded only, DPI-capped, differentiation-penalized, Goodhart-defended (held-out + `Gτ` + cross-lens anomaly), reversible + tripwire + Ledger-logged (A14/A15), bounded compute (A26). Never inflate a number past reality; never lose data or intelligence to "optimize."

## §1d — Zero-cost & self-built (A34, core doctrine)

**Everything is free, and we build it ourselves in Rust.** Calyx is alien-to-the-market technology custom-fitted to the Royse corpus — so we do not lean on paid black boxes; we code every component.

- **No money.** No paid services, SaaS, managed/cloud databases, paid CI minutes, paid scanners, or paid provider features. Verification is local/on-aiwonder; tooling is OSS (`cargo`, `clippy`, `cargo audit`, OSS semgrep, ZFS, restic, Prometheus/Grafana, `gh` Free). Storage is POSIX-on-ZFS on our own box — no S3/cloud. (Consistent with Polis "local-only, no paid CI/scanners" and the aiwonder single-box posture.)
- **Self-built in Rust.** Every engine — Aster, Forge (own matmul/distance/quant), Loom, Assay, Lodestar, Ward, Sextant, Ledger, Anneal — is ours. We do not depend on an external math service, a hosted vector DB, or a managed model API on the hot path (A13). Reuse OSS *crates/ideas* freely (they're free), but the load-bearing engine is hand-built.
- **The one possible exception:** a genuinely great embedder, *if such a paid thing ever existed* — doubted. Lenses otherwise run from free/open weights (HuggingFace, `hf_hub_token`) on our GPU.
- **Stay in the docs.** These planning documents are based on technology the founder *knows to be true and working*. Do not stray to a paid or unproven external approach when the doc specifies a buildable one; if a doc seems to require a paid dependency, that is a smell — re-derive the free, self-built path.

This composes with A13 (Rust, baked-in math), A24 (strict Royse theory), and §8c (own box): **free, ours, true.**

## §2 — The source-of-theory rule (STRICT)

The **theory** of this project is **strictly and only** the Royse corpus:

- **Papers:** *The Calculus of Association* (3 Jun 2026), *The Oracle and the Kernel* (4 Jun 2026), *The Symmetry of Knowing*.
- **Systems (proven-true):** ContextGraph (13 lenses, ME-JEPA), socialmedia2.com/Polis (21-slot Constellation), ClipCannon (N=7), Leapable.
- **The recorded video transcripts** (fifth element = information; the Oracle; cortical columns ≈ embedders; JEPA world models; per-domain super-intelligence; self-heal/learn; free-will/anti-engagement).

**Rules (tightened — binding going forward):**
- **Reference only the Royse corpus.** Build **everything** here from the founder's two papers, video transcripts, and three systems. Do **not** import an external theory, substitute a competing framework, dilute a Royse construct with a generic one, **or introduce a new external method as a design source.** The database is **fully custom-fitted** to the corpus — the ultimate database for AGI and the Oracle.
- **Use ALL of it, everywhere** — DDA, TCT, `Gτ`, the differentiation contract, meaning compression, the grounding kernel (any scope), the Oracle, epistemic symmetry, identity-locked generation, the three temporal embedders (E2/E3/E4) + AP-60, cosine-`Gτ` deduplication, and the bound (DPI / effective rank / grounding caveat) must thread through the entire system, not sit in one doc.
- **External technique is frozen to what the founder explicitly requested or unavoidable platform reality** — TurboQuant (he requested it), grouped GEMM / Blackwell / CUDA / ZFS / the aiwonder box (hardware reality), the FoundationDB-style key-encoding *pattern* for the general data layer. These are *engineering scaffolding only*; never a source of intelligence theory, and no further external method is added. The theory and every intelligence capability are his.

## §3 — The universality mandate (first principles)

Calyx MUST perform the **root core function** of every database, derived from first principles, not by bolting on engines but by serving the underlying purpose (`20`):

| Paradigm | Root purpose | How Calyx serves it |
|---|---|---|
| Relational/OLTP | typed records + constraints + transactions + joins | general data layer: typed collections, secondary indexes, ACID-per-vault, join-by-reference |
| Document | nested schemaless records | document collections (tuple-encoded paths) |
| Key-Value | O(1) keyed state | KV collections over the ordered keyspace |
| Columnar/OLAP | fast scans/aggregates | Aster columnar chunks (Arrow), HTAP row+column |
| Graph | nodes/edges/traversal | the association + cross-term graph is **native** (Loom/Lodestar) |
| Time-series | ordered events, retention, rollups | temporal lenses + scalar columns + range keys + retention |
| Full-text search | inverted term match | a sparse lexical **lens** (SPLADE/keyword) — search is a lens |
| Vector | ANN over embeddings | a dense **lens** + per-slot ANN — a vector DB is a 1-lens Calyx |
| Object/blob | large payloads | input store + cold-tier sidecars |

The differentiator: Calyx's multi-lens intelligence gives **insane search/navigation** that *subsumes* the search-shaped paradigms (full-text, vector, graph, "ASK"/RAG) — don't run Elasticsearch + Pinecone + Neo4j next to it; the Association Engine **is** those, better, plus DDA/kernel/Oracle.

## §4 — The AGI / Oracle / kernel mandate

Calyx MUST bake in (`21`, `22`):
- **The Oracle:** predict the consequences of an action/input (the world model / butterfly effect); the per-domain operational super-intelligence predicate; the oracle self-consistency ceiling; substrate sufficiency `I(panel; oracle)`; online mistake-closure; the ME-JEPA negative result reported honestly.
- **The kernel at any scope (freedom of scope):** compute the grounding kernel of *all* associations, of a chosen dataset, of a domain, of a query subgraph, of a time window, of a tenant — the operator picks the scope; Lodestar runs MFVS on it; the kernel is an index **and** an answer-path (`08`).
- **Automatic intelligence extraction:** the formulas (`22`) are coded into the backend so the DB extracts the associations, the bits, the kernel, the guard, and the oracle prediction **automatically** — the user never re-derives or re-implements them. "It's AGI, so the database does it."

## §5 — The backbone rule (plug-in lenses is THE key)

The single most important ergonomic, repeated by the founder: **make it as easy as possible to plug embedders in/out, analyze their value/bits, use their associations, and compute the kernel.** Every design decision is judged against this. A new lens is **one call** (`add_lens`), its value **one number** (`bits`/capability card), the kernel **one call at any scope**, and the multi-lens plumbing the founder used to hand-write every time is **gone, inside the database**. If a change makes plugging in a lens or reading its bits harder, reject it.

## §6 — The non-negotiable invariants (the 24 axioms)

Binding (full text in `02`):
A1 record=constellation · A2 grounding mandatory (association is circular without it) · A3 no-flatten · A4 frozen lenses · A5 hot-swap · A6 designable lenses · A7 differentiation contract (≥0.05 bits, ≤0.6 corr) · A8 DPI ceiling · A9 effective rank · A10 kernel discoverable at any scope · A11 kernel is index+answer · A12 `Gτ` everywhere generation touches the store · A13 Rust + baked-in GPU math · A14 self-optimizing · A15 source-of-truth verification · A16 fail closed · A17 agent-native · A18 embedded+served one core · **A19 universality (serve every paradigm's root purpose)** · **A20 the Oracle (predict consequences)** · **A21 multi-level kernel (freedom of scope)** · **A22 formulas baked in (automatic intelligence extraction)** · **A23 epistemic symmetry (Q↔A bidirectional)** · **A24 strict Royse theory** · **A25 maximal *measured* compression (`23`)** · **A26 bounded, leak-free, self-reclaiming (`24`)** · **A27 native temporal understanding — temporal lenses (E2/E3/E4) for search/retrieval only under AP-60 (never dominant), while the database's event/sequence/recurrence understanding unlocks new capabilities (`25`)** · **A28 TCT cosine-`Gτ` deduplication — dedup ONLY by multi-content-slot `Gτ` agreement (no-flatten), configurable at creation, temporal lenses excluded from dedup; recurrences collapse into one event + a time series (`25`)** · **A29 grounded recurrence — the recurrence signature (all content lenses agree + temporal lenses differ = the same action across time) is detected automatically; event-time + dedup-count is a grounded signal (frequency, cadence, oracle self-consistency, temporal co-occurrence/causality, kernel importance, information/surprise) used optimally throughout the whole system (`25 §4c`)** · **A30 connection-of-truths discovery — the highest-probability grounded connection between two truths is itself true; this is both the design method (how we find capabilities) and a system capability (the DB proposes highest-probability grounded paths between truths; grounding/FSV confirms them) (`26 §11`)** · **A31 living intelligence — Calyx runs the calculus of association continuously while maintaining, healing, learning, and growing itself, so by the thesis's own definition it *is* an intelligent, life-like system; the docs claim operational intelligence and life-like behavior, never consciousness/qualia (`DOCTRINE §1b`)** · **A32 maximize grounded intelligence — every operation optimizes for maximum growth of grounded intelligence/understanding as fast as safely possible (composite `J`: grounded information, `n_eff`, sufficiency, kernel recall, oracle accuracy, meaning compression — which subsumes compression-without-loss and retrieval efficiency as facets); the math self-adjusts its own parameters online as new data arrives ("recode" = online re-parameterization, never engine-source rewrite); bounded by grounding (A2), DPI ceiling (A8), differentiation (A7), Goodhart-defense, reversible/FSV (A14/A15) — maximize *measured* intelligence, never a gamed number, never deleting data or losing intelligence (`27`)** · **A33 security & privacy by construction — least privilege, default-deny tenant isolation, encryption at rest + in transit, input validation, supply-chain integrity, Ledger-as-audit; right-to-erasure is first-class via crypto-shredding (A25 forbids deleting-*to-compress*, NOT lawful/user deletion); fail closed on every security/privacy uncertainty (`30`)** · **A34 zero-cost & self-built — everything is FREE and built in-house in Rust: no paid services, SaaS, managed/cloud dependencies, paid CI, or paid scanners; we code every component ourselves. The *only* conceivable paid item is a good embedder (and even that is doubted to exist). Work within these planning docs — this is custom, true-to-the-corpus technology; do not stray to a paid black box when we can build it (`§1d`)** · **A35 panel-not-lens (multi-embedder minimum, ≥ 10) — a single lens is a vector DB, not Calyx; every test/bench/FSV/gate that touches retrieval, recall, signal/bits, fusion, scale, or an SLO MUST run a real panel of ≥ 10 frozen lenses and measure per-lens bits + the ensemble decomposition + the fused (RRF) result. The bootstrap floor of 4 is retired (it stood up the path); ≥ 10 is now required for all testing/tuning, scaling toward 20+, resource-aware (A26). Value is associational, never intrinsic — a lens's worth is its unique+redundant+synergistic bits *in the company of the others* (PID / interaction information / conditional MI), so 1–2 lenses cannot be valued and ≥ 3 is the minimum to begin computing it (≥ 10 for stable estimates). Single-/double-embedder or synthetic-vector runs are diagnostic-only and NEVER satisfy a gate; an FSV artifact with < 10 real lenses fails closed (A15/A16) (`02 A35`, `05 §9`, `07`, `28 §2`, `§9`)** · **A36 embedder templates — the right 10+ lenses differ by situation (video vs literary-essence vs code vs civic …); Calyx keeps a library of named, versioned, content-addressed panel templates the operator develops, profiles, and swaps in/out per vault/query without re-embedding existing constellations (A5), each carrying its measured ensemble signal so a template's fitness for a job is itself a number (`02 A36`, `05 §7`/`§8`)** · **A37 associational diversity — the ≥10 floor (A35) is a *count* gate, satisfiable by 10 redundant dense-semantic clones (PID: redundant, not unique/synergistic bits); the pairwise A7 ≤0.6-corr contract is necessary but not sufficient (per-pair, misses collective rank-deficiency). Every panel admitted for a test/bench/FSV/gate MUST also pass a panel-level diversity gate: span distinct association *families* fit to the job (dense-semantic general+domain · lexical/sparse · entity/graph · character/byte · structural · reranker/asymmetric · temporal-sidecar A27), clear a panel-wide redundancy bound (`n_eff` (A9) + mean pairwise corr/NMI thresholds, not just each pair ≤0.6), and show a non-collapsing marginal-bits curve (each lens adds measurable *unique* PID bits given the rest). Diversity is a recorded number (`n_eff`, mean redundancy, Σ unique-PID-bits) on the EnsembleCard/template (A36). A 10-by-count-but-redundant panel is diagnostic-only and fails closed (A16); a deliberately homogeneous panel is valid only as a labeled baseline/control that *measures* the redundancy collapse (`02 A37`, `05 §9b`, `07`)** · **A38 resource-bounded roster (#1 priority) — the optimal panel is the maximally diverse, maximally grounded embedder set that fits one 24 GB GPU (4090/5090-class) under ≤ 20 GB resident lens weights, covering every modality/domain (text general + legal/medical/clinical/biomedical/scientific/financial/code/multilingual · multiple image CLIP-style + DINO-style · audio speech/speaker/music/environmental · doc-image · protein/DNA/molecule); maximise count × measured `bits/VRAM-MB` (#729), quantise (INT8/FP16/MRL/static, A25) to fit more lenses; canonical catalogue + default "Constellation-24" panel in `05a`; admission = A35 + A37 + measured `Σ VRAM ≤ 20 GB` under the density packer + fused-RRF beats the 1–2-lens control (`02 A38`, `05a`, `DOCTRINE §10.29`)**.

## §7 — Bake the formulas in (canonical)

The Formula Library (`22`) is canonical and binding: every formula from the Royse corpus is a first-class, callable, self-optimizing backend primitive (DDA identity, meaning compression yield, KSG MI, differentiation contract, effective rank, DPI bound, grounding kernel/MFVS, `Gτ`, τ calibration, RRF, cross-terms, oracle self-consistency, panel sufficiency, hop-attenuation, Q↔A reversibility). The DB computes intelligence; the agent consumes it.

## §8 — Engineering rules

- **Rust, GPU math baked in** (`13`): CUDA sm_120 + SIMD, bit-parity tested; no external math service on the hot path.
- **One ordered transactional core** (Aster, FoundationDB-style): every paradigm is a key-encoding layer; ACID transactions make layers correct (`04`, `20`).
- **Fail closed** (A16): unknown lens, dim mismatch, ungrounded "trusted" result, corrupt shard, MI below quorum → structured error, never a silent fallback.
- **Provenance always** (Ledger, `11`): every vector, bit, kernel, guard verdict, oracle prediction, and answer traces to its grounded source and replays.
- **FSV** (§0): prove every claim by reading bytes; FSV harnesses are banned.
- **One change at a time; plan before architectural/storage/security changes; document failure as carefully as success.**
- **Code & test files ≤ 500 lines (HARD); documentation unlimited.** Documentation (these `.md` PRDs) may be as long as needed. **Every code and test file MUST be ≤ 500 lines.** When a file would exceed 500 lines, modularize per `docs2/modulateprompt.md`: split by single responsibility into a module directory with a thin facade (Rust `mod.rs` with explicit `pub use` re-exports — no wildcard), no circular dependencies, 100% backwards-compatible API (identical signatures/paths), all tests still passing; the original path becomes a < 100-line facade. **If an agent encounters any code/test file > 500 lines, it MUST open a GitHub issue to modularize that file, then resolve that issue** (perform the split, then verify `cargo check` + `cargo test` + `cargo clippy` clean and re-run the line-count check below). Track modularization debt as issues; never leave an over-limit file silently.
  ```bash
  # line-count gate (Rust): fail if any source/test file > 500 lines
  find crates -name '*.rs' -exec wc -l {} + | awk -v max=500 '$1>max && !/total/{print "❌",$0; v=1} END{if(!v)print "✅ all .rs ≤ 500 lines"}'
  ```

## §8b — Canonical protocols (binding references)

Three founder prompt-docs are **binding** on all work here. They override nothing in this charter; they operationalize it.

| Protocol | Doc | Governs |
|---|---|---|
| **AI Coding Agent Doctrine** | `docs2/AICodingAgentSuperPrompt.md` | how code is written, debugged, verified, coordinated |
| **File Modularization** | `docs2/modulateprompt.md` | the ≤500-line rule + how to split (§8) |
| **Compression Doctrine** | `docs2/compressionprompt.md` | how prompts, docs, and code-comments are written (max signal / min token) |
| **Test Usefulness (Rust)** | `28 §6c` | what makes a test useful — FIRST, behavior-not-implementation, the two questions, property/fuzz/mutation, anti-flakiness, all Rust-tailored |

**No CI pipeline — FSV is our CI (binding).** Calyx has **no hosted CI/CD pipeline** (it would slow the build and cost money, A34). **FSV — manual source-of-truth readback on aiwonder (§0) — is our CI.** Tests (`cargo test` / `proptest` / `cargo-fuzz` / `cargo-mutants` / `criterion`, all OSS) are the fast inner loop and a *claim*; FSV reads the bytes and is the *truth gate*. Both run locally / on aiwonder, agent-invoked, never in a pipeline. A passing test never satisfies FSV (§0); a useful test must fail-when-wrong and pass-when-right (`28 §6c`).

**Synapse is the computer-use & orchestration runtime (binding, `31`).** For substantive dev work, agents use **Synapse** (perceive: `observe`/`read_text`/`find`; act: `act_type`/`act_launch`/`act_run_shell`; `reality_audit`) to operate the real machine and — its highest use — **open terminals and command other agents (Claude/Codex)** to build/test/verify Calyx on aiwonder. The commanded agents get full Synapse capabilities themselves. This is **preferred over the subagent tool** for anything that touches the machine, because it runs in real terminals with real FSV against real bytes. Synapse perception **is FSV's perception arm** (reads the *actual* terminal/test output, not a claimed return). Subagents stay for quick read-only fan-out only.

Key coding protocols pulled from `AICodingAgentSuperPrompt.md` (embedded, not just referenced):
- **FSV is manual** (§0): the agent verifies the source of truth itself; no script/harness satisfies FSV. Four steps: read SoT before → act → read SoT after → inspect the delta.
- **Missing prerequisites are work, not blockers** — build the missing piece, don't stop.
- **Mental models, installed:** first-principles decomposition · symptom vs cause vs root cause · fail-closed not fail-open · defense in depth · asymmetry of risk · 80/20 · abductive hypothesis generation · contradiction engine · Linear Sequential Unmasking.
- **GitHub Issues are the coordination + memory surface:** read issue-state at the start of every turn; claim atomically before work; comment at every milestone; pause/blocked/done protocol; **dedupe before every create**; record decisions (ADR) and discoveries as issues. The §8 modularization rule files issues here.
- **FSV rigor:** ≥3 edge audits per code path (more for security); synthetic test data with known inputs/known expected outputs; aim for verification maturity L3+; **when a test fails, STOP** and root-cause.
- **RCA:** 5 Whys to a *structural* root cause, never "someone forgot."
- **Hypothesis-driven debugging:** scientific method — hypothesize, predict, test against the SoT.
- **No workarounds:** fail fast, fail loud; no fallback that hides failure (A16).
- **Anti-sycophancy:** never claim "Done" without evidence — open the diff, re-run, read the bytes (§0).
- **Be your own reviewer; forensic (Sherlock) discipline** for investigations; **web research = source over summarizer** (for engineering only, A24).

**Compression doctrine, applied to all writing here:** maximize specificity-per-token; cut what the model regenerates from priors (meta-noise, restatement, trivial examples); keep **verbatim** the No-Compress List — numbers/thresholds (`0.05 bits`, `0.6 corr`, `500 lines`, `τ`, `sm_120`), file paths, error codes (`CALYX_*`), axiom ids (`A1`–`A38`), formulas, named entities; structure over prose; canonical terms; round-trip to verify no meaning lost. **Documentation length is unlimited; density is still mandatory.**

## §8d — Development state via GitHub Issues (binding)

Dev state lives in the Calyx repo's **GitHub Issues** (`29`). A small curated set of pinned **`type:context`** issues (Mission & invariants · You-are-here · Environment & ops · Landmines · Datasets) holds the critical info **every agent MUST read at the start of every turn and keep in context.** They are **current-state snapshots, not journals**: pointers to docs (not copies), short enough to always fit context, with a last-verified stamp. **Keep them current by editing to the new truth — never append a contradiction.** **Prune every phase:** close done tasks, supersede + close old decisions, remove fixed gotchas, run a context-hygiene pass re-verifying each line against the bytes — so a fresh-context agent never sees a stale, confusing log. An issue is a claim; the bytes are the truth (§0). Read-state queries + taxonomy: `29`.

## §8c — Everything runs on aiwonder; secrets via Infisical (binding)

- **aiwonder is where the project exists.** This WSL dev box **authors** code only. The project is **built, stored, run, and tested on the `aiwonder` datacenter PC** (RTX 5090 sm_120, ZFS). The source-of-truth bytes that FSV reads live on aiwonder; a local run never counts as FSV (`28 §5`). **Build natively on aiwonder** under `CALYX_HOME=/home/croyse/calyx` (Rust via rustup is installed — the earlier "no `rustc` on the box → cross-build to `/opt/leapable/calyx/`" note is **superseded**; cross-build remains only an optional deploy path). Datasets + Aster vaults live on aiwonder ZFS. Reach it via the repo `.env` / `~/.config/aiwonder.env` (SSH/VPN/sudo bootstrap; `16 §0`). Current environment readback: `docs/implementation/01_AIWONDER_ENVIRONMENT.md`.
- **Secrets via Infisical** (`leapable-aiwonder-prod`, env `prod`). Calyx most likely needs **only `hf_hub_token`** (HuggingFace — models + datasets). Add any future secret (e.g. Kaggle creds) with `infisical secrets set …`; load via `infisical run …` or the rendered `/run/leapable/secrets/calyx.env`. **Never** put a secret *value* in a repo/issue/PR/chat — env-var names only. Catalog: `leapablememory/docs2/infisical-secrets-guide.md`; policy `16 §5b`.

## §8e — Agents handle every aspect of development, via Synapse (core, binding)

**Calyx is built by agents that handle every single aspect of development, autonomously, on aiwonder, using Synapse computer use** (`31`). This is core doctrine, not an aside. The agent loop — author, scaffold, build, test, **FSV**, acquire datasets, deploy, run ops, read dashboards, debug, coordinate, and **orchestrate other agents in real terminals — open a terminal and start Claude (`cldy`) or Codex (`codex --yolo`), then prompt them** (much more powerful than the subagent tool; each worker has full Synapse control, recursively) — runs through Synapse's full ability set: perceive (`observe`/`read_text`/`find`/**screenshot+AI-vision**), act (`act_run_shell`/`act_type`/`act_launch`), reality-audit (FSV's perception arm, §0), reactive `reflex_*`, recorded `audit_export_bundle` evidence. Web/dashboards: a **new tab in the one main Chrome** (auto-authed; `31 §6b`). Synapse-driven agents are **preferred over the subagent tool** for anything touching the machine, because they run in real terminals with real FSV against real bytes (`28 §2c`). Humans **direct and approve** outward-facing/destructive actions (§9, `31 §7`); everything else, agents do. The aim: **no aspect of development requires a human except direction and approval.**

## §9 — Anti-patterns (refuse these)

- Flattening the panel into one opaque vector (breaks A3 and the whole guard).
- Reporting `C(N,2)` abundance as new information beyond the DPI ceiling (A8).
- Labeling bits/kernel/guard "trusted" without grounding (A2) — say `provisional`.
- Mutating a frozen lens, or training on synthetic generator output (model collapse).
- Substituting an external theory of intelligence for the Royse corpus (§2).
- Making lens plug-in, bits, or kernel harder to reach (§5).
- A green-checkmark harness standing in for FSV (§0).
- **Validating, benching, or FSV-ing a retrieval/recall/signal/fusion/scale/SLO gate on fewer than 10 embedders or on synthetic vectors (A35).** A single lens is a vector DB, not the Constellation; 1–2 lenses cannot even be *valued* (value is associational — PID over ≥ 3, stable over ≥ 10). Every such run uses a real panel of **≥ 10 lenses** (scale toward 20+, resource-aware) and records per-lens bits + the ensemble decomposition + the fused result, or it fails closed. (The retired 4-lens bootstrap floor no longer satisfies any gate.)
- **Counting to 10 with redundant near-clones — e.g. ten general-purpose dense semantic embedders — and calling the panel diverse (A37).** The ≥ 10 count gate (A35) and the pairwise ≤ 0.6 contract (A7) are both *necessary but not sufficient*: 10 single-family models maximize redundancy (their PID bits are redundant, not unique/synergistic) and a panel can be collectively rank-deficient while every pair sits under 0.6. A gate-bearing panel MUST also span distinct association *families* and clear a **panel-level** redundancy bound (`n_eff`, mean corr/NMI, non-collapsing marginal-bits). A 10-by-count-but-redundant panel is diagnostic/control-only and fails closed for gates (A16).
- A code/test file over 500 lines without a tracking GitHub issue to modularize it (§8); or modularizing in a way that breaks the public API or adds a circular dependency.
- Bolting on a separate search/graph/vector DB instead of using the Association Engine that subsumes them (§3).

## §10 — Locked decisions (binding; the founder's directives, captured)

1. **Calyx is the universal database** — serves every paradigm's root purpose (`20`) — **and** the AGI/Oracle substrate (`21`). Use it for any project.
2. **Leapable is one project.** For Leapable, Calyx replaces **only** the SQLite/`sqlite-vec` Vaults; **PostgreSQL is untouched** (customers/billing/etc.). A greenfield project may use Calyx for everything (`15`, `20`).
3. **The kernel is computed at ANY scope** — all associations / a dataset / domain / subgraph / time window / tenant / any filter (`08 §4b`).
4. **Maximal compression, measured.** TurboQuant (founder-requested) + MXFP4 + the kernel; compress to the floor where Assay bits / Ward FAR / cosine / kernel-recall still hold (`23`, A25).
5. **Bounded, leak-free, self-reclaiming** — no managed-GC pauses, every allocation/cache/queue bounded, every DB-garbage reclaimed, fail closed under pressure (`24`, A26).
6. **Temporal lenses (E2/E3/E4) are for search/retrieval ONLY** (AP-60, never dominant); the **database's** temporal understanding of events/sequences/recurrence/time capture is a separate time-control layer for as-of reads and walking state forward/backward through time (`25`, A27).
7. **Deduplication is ONLY the TCT cosine-`Gτ` guard** over required **content** slots (no-flatten; temporal lenses excluded); recurrences collapse into one event + a time series; configurable at creation (`25`, A28).
8. **Plug-in lenses is THE backbone** — one call to add, one number for its bits, one call for the kernel at any scope (§5).
9. **Strictly the Royse corpus going forward** — fully custom-fitted; external technique frozen to founder-requested/platform-reality scaffolding only (§2, A24).
10. **Event-time is grounded reality.** The recurrence signature (content lenses agree + temporal lenses differ = the same action across time) is detected automatically; frequency/recurrence feeds oracle self-consistency, prediction, causal discovery, kernel importance, information/surprise — used optimally across the whole system (`25 §4c`, A29).
11. **Connections between truths are true.** The highest-probability grounded connection between two truths is itself true — the design method and a system capability; grounding/FSV confirms each discovered connection (`26 §11`, A30).
12. **Calyx is a living intelligence.** It runs the calculus of association continuously while healing, learning, and growing itself, so by the thesis it *is* an intelligent, life-like system — operational intelligence + life-like behavior (Living System Map, §1b), never a claim of consciousness (A31).
13. **Maximize grounded intelligence (the drive).** Every operation optimizes for maximum growth of grounded intelligence (`J`), fastest-first; the math self-adjusts its parameters online as new data arrives; compression-without-loss and retrieval efficiency are facets of this objective; bounded by grounding/DPI/differentiation/Goodhart-defense/reversibility — maximize *measured* intelligence, never delete data or game a number (§1c, A32, `27`).
14. **Everything on aiwonder.** Built, stored, run, tested, and state-resident on the aiwonder box; WSL authors only; FSV reads aiwonder's persisted bytes (§8c, `28 §5`, `16`).
15. **Code/test files ≤ 500 lines; docs unlimited.** Modularize per `modulateprompt.md`; over-limit file → GitHub issue + fix (§8). Protocols binding: `AICodingAgentSuperPrompt.md`, `modulateprompt.md`, `compressionprompt.md` (§8b).
16. **FSV per aspect is defined** (`28`): synthetic deterministic data proves mechanics; a catalog of real datasets (text/code/graph/audio/image/temporal/adversarial, from HuggingFace/Kaggle) proves intelligence; acquisition is checksum-FSV'd; secret needed = `hf_hub_token` (Infisical).
17. **Dev state via GitHub Issues** (`29`, §8d): repo **`chrisroyse/calyx`**; pinned `type:context` issues every agent reads each turn; kept current by editing to truth (never appending contradictions); pruned every phase so fresh-context agents see a true, tight snapshot — not a confusing stale log.
18. **Security & privacy by construction** (`30`, A33): STRIDE-modeled, least-privilege, encrypted, default-deny tenant isolation; **right-to-erasure is first-class crypto-shredding** — A25 forbids deleting-*to-compress*, never lawful/user deletion; honest cold-start (search day 0, trust as anchors arrive).
19. **Project = Calyx** at `CALYX_HOME=/home/croyse/calyx` on aiwonder (self-contained; renamed from aidb); repo `ChrisRoyse/Calyx`. Build/run/test on aiwonder.
20. **Zero-cost & self-built** (`§1d`, A34): everything free, built in-house in Rust; no paid services/SaaS/cloud/CI/scanners; only conceivable paid item = a great embedder (doubted). Code every component ourselves; stay within these docs.
21. **No CI pipeline — FSV is CI** (`28 §6b`, §8b): tests = fast inner loop (claim); FSV = truth gate (reads bytes); both on aiwonder, agent-invoked, no hosted pipeline.
22. **Synapse for computer use & orchestration** (`31`): agents perceive/act on the real machine and open terminals to command Claude/Codex agents to build/test/verify — preferred over subagents; Synapse perception is FSV's perception arm.
23. **One main Chrome, new tabs only** (`31 §6b`): for any web/dashboard/account, agents use the operator's single already-logged-in main Chrome and open a **new tab** (auto-authenticated) — never a second Chrome/window/incognito/other browser; reuse + close the tab, never close Chrome.
24. **Screenshot + AI-vision is a primary perception mode** (`31 §6c`): Synapse `capture_screenshot` and the AI *looking at* the image is excellent for seeing what's actually going on — charts/graphs (Grafana), GUI state, error dialogs, rendered output that OCR/text can't capture; use it for FSV and dev observation.
25. **Agents handle every aspect of development** (`§8e`, `31 §6d`): via Synapse, agents author/build/test/FSV/deploy/debug/orchestrate end-to-end on aiwonder; humans only direct + approve outward/destructive actions. Core doctrine.
26. **Panel-not-lens — multi-embedder (≥ 10) is mandatory for ALL testing** (`02 A35`, `05 §9`, `07`, `28 §2`, §9): from here on, every test/bench/FSV/gate that touches retrieval, recall, signal/bits, fusion, scale, or an SLO runs a **real panel of ≥ 10 frozen lenses** and measures **per-lens bits + the ensemble decomposition (marginal value, cross-term/synergy gain, `n_eff`, panel sufficiency) + the fused (RRF) result**. The **4-lens bootstrap floor is retired** — it was only ever enough to stand up the fusion path; **≥ 10 is now required**, scaling toward **20+**, resource-aware (A26). Rationale: a lens's value is **associational, not intrinsic** — you cannot measure the worth of 1–2 embedders (value = unique+redundant+synergistic bits *relative to the rest of the panel*; **≥ 3 to begin, ≥ 10 for stable estimates**, per PID / interaction information / conditional MI), so single-/few-lens "gates" prove nothing and caused real hard failures. Single-embedder or synthetic-vector validation is **diagnostic-only** and **NEVER** satisfies a gate; an FSV artifact recording **< 10 real lenses fails closed** (A16). Roster/acquisition + measurement protocol: `05 §9`, `07`.
27. **Embedder templates — situation-specific, swappable ≥ 10-lens panels** (`02 A36`, `05 §7`/`§8`): the right 10+ lenses **differ by what is being measured** (video ≠ literary-essence ≠ code ≠ civic), so Calyx keeps a **library of named, versioned, content-addressed panel templates** the operator develops, profiles, saves, and **swaps in/out per vault or query** without re-embedding existing constellations (A5). Each template carries its **measured ensemble signal** (A35), so its fitness for a job is itself a number. The system templates sets of embedders and loads them on demand.
28. **Associational diversity — span types, not just count (A37)** (`02 A37`, `05 §9b`, `07`, §6/§9): the ≥ 10 floor (A35) is a **count** gate — satisfiable by 10 redundant dense-semantic clones that maximize redundancy and add almost nothing past the first few (PID: redundant, not unique/synergistic bits). The pairwise A7 contract (≤ 0.6 corr) is **necessary but not sufficient** — per-pair, it misses *collective* rank-deficiency. Every panel admitted for a test/bench/FSV/gate MUST also pass a **panel-level diversity gate**: (a) span distinct association **families** fit to the job — dense-semantic (general + domain) · lexical/sparse · entity/graph · character/byte · structural · reranker/asymmetric · temporal-sidecar (A27 time-control/as-of traversal); (b) clear a **panel-wide redundancy bound** — `n_eff` (A9) + mean pairwise corr/NMI thresholds, not just each pair ≤ 0.6; (c) show a **non-collapsing marginal-bits curve** — each lens contributes measurable **unique** PID bits given the rest. Diversity is a **recorded number** (`n_eff`, mean redundancy, Σ unique-PID-bits) on the EnsembleCard/template (A36). A 10-by-count-but-redundant panel is **diagnostic-only and fails closed** for gates (A16); a deliberately homogeneous panel is valid **only** as a labeled baseline/control that *measures* the redundancy collapse (the evidence for A37).
29. **The resource-bounded roster — the optimal panel for one 24 GB GPU (A38, #1 priority)** (`05a`, `02 A38`, `05 §9`/`§9b`, `07`, `13`, #729): the program's top objective is to **discover and maintain the maximally diverse, maximally grounded set of frozen embedders that fits one 24 GB GPU (4090/5090-class) under a ≤ 20 GB resident-lens-weight budget**, covering **every modality and domain a general database serves** — text (general semantic **+ legal · medical · clinical · biomedical · scientific · financial · code · multilingual**), **multiple image** embedders (zero-shot CLIP-style **and** self-supervised DINO-style — they see differently), **audio** (speech · speaker · music · environmental), document-image, and the science modalities (protein · DNA · molecule). **Maximise count × signal-density:** prefer **many small high-signal lenses** over a few giants; a lens earns its slot by **measured `bits / VRAM-MB`** (#729), not raw bits. **Quantise to fit** — ONNX **INT8** by default (≈ 4× < FP32, ≈ 2× < FP16, retrieval delta < `recall_delta`), FP16 for vision/audio, Matryoshka-truncation for MRL models, static/model2vec for the 0-VRAM baseline (A25 measured compression). The canonical candidate catalogue + the default panel (**"Constellation-24"**, Tier-1 ≈ 21 content lenses / ~9 families / ≈ 5–8 GB; Tier-2 → ~32 lenses / ≤ ~14 GB; Tier-3 giants one-at-a-time) lives in **`05a_EMBEDDER_ROSTER_VRAM_BUDGET.md`**. Admission = A35 (≥10 *learned* lenses) **+** A37 `gate_passed` **+** measured `Σ VRAM ≤ 20 GB` under the density packer **+** fused-RRF beats the 1–2-lens control. Budget is **not** the constraint — diversity and measured bits are; spend headroom on *more families*, never more dense-semantic clones. This subsumes and prioritises #26–#28.

---

**One sentence:** Calyx is the universal, association-native, self-optimizing database that serves every paradigm's root purpose and bakes in the Royse calculus-of-association — DDA, the differentiation contract, the multi-scope grounding kernel, `Gτ`, and the Oracle — so that plugging in a lens, reading its bits, and computing the kernel of any data is trivial, automatic, grounded, provenanced, and the heart of the formula for intelligence.
