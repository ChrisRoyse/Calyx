# Planning Docs Summary — `docs/dbprdplans/` + `docs/implementation/`

**Source files covered:**
- `docs/dbprdplans/00_INDEX.md`, `docs/dbprdplans/DOCTRINE.md`, and the 32 numbered PRD plans `docs/dbprdplans/01_*.md` … `docs/dbprdplans/31_*.md`
- `docs/implementation/00_README.md`, `docs/implementation/02_WORKING_AGREEMENT.md`, `docs/implementation/03_PHASE_MAP.md`, and the stage docs `10_STAGE0_*` … `30_STAGE20_*`, plus `01_AIWONDER_ENVIRONMENT.md`, `02_BUILD_PERFORMANCE.md`, `FSV_NOTES.md`, `STAGE1_5_EVIDENCE_MANIFEST.md`
- `docs2/` reference files

> **What this doc is.** A NAVIGATION/INTENT map of the *original design plans* for Calyx — the PRD (`docs/dbprdplans/`) and the build plan (`docs/implementation/`). These are markdown design documents, **not code**. They state design *intent*: what Calyx is meant to be and in what order it is meant to be built. Where the shipped code diverges from a plan, that divergence is documented in the per-subsystem reference docs `05_*.md`–`20_*.md` (cross-referenced by subsystem below). Per the spec's iron rule (`_SPEC.md §11`): document WHAT IS in those docs; this doc summarizes the PLANS. Do not read a plan statement as a claim that the code does it.

---

## 1. The two doc sets

| Set | Path | Role | Count |
|---|---|---|---|
| **PRD plans** | `docs/dbprdplans/` | *What* and *why* — the product requirements, vision, axioms, per-subsystem design | 32 numbered docs (`00`–`31`) + `DOCTRINE.md` (33 files total) |
| **Implementation plan** | `docs/implementation/` | *In what order, on what machine, proven how* — phases/stages mapped to crates with FSV exit gates | 21 stage docs (`10_STAGE0` … `30_STAGE20`) + `00_README`, `02_WORKING_AGREEMENT`, `03_PHASE_MAP`, `01_AIWONDER_ENVIRONMENT`, `02_BUILD_PERFORMANCE`, `FSV_NOTES`, `STAGE1_5_EVIDENCE_MANIFEST`, + 60+ per-phase `PHnn-*/` task-card subdirs |

`DOCTRINE.md` is the canonical binding charter — it overrides every other doc. The PRD's master index is `00_INDEX.md`. The implementation read order is: `DOCTRINE.md` → `00_README.md` → `01_AIWONDER_ENVIRONMENT.md` → `02_WORKING_AGREEMENT.md` → `03_PHASE_MAP.md` → the stage files in order.

---

## 2. DOCTRINE — the binding principles (verbatim where short)

`DOCTRINE.md` is "the canonical, binding charter" and "overrides every other doc in this project." Core principles, stated verbatim:

### 2.1 The cardinal rule (§0)
> **A return value is a claim. The source of truth is the bytes. Read the bytes.**

This is **FSV** (Full State Verification): "No FSV script/harness can satisfy FSV; a human or agent reads the bytes." Inherited from Leapable doctrine, non-negotiable.

### 2.2 The thesis (§1)
> **Calyx is the heart of the formula for intelligence, implemented as a database.**

Three-in-one: (1) the engine of the **Calculus of Association** — the four native verbs **measure · count · differentiate · compose**; (2) the **universal database** serving every paradigm's root purpose on one ordered transactional core; (3) the **AGI / Oracle / kernel substrate**.

### 2.3 Living intelligence (§1b, A31)
Every life-like property maps to a concrete engine (the Living System Map: perception=lenses, memory=store, cognition=Loom/Sextant, differentiation=Assay, homeostasis=Anneal, foresight=Oracle, immune boundary=Ward). The bound (binding, honesty): claims **operational intelligence + life-like behavior**, **never consciousness/sentience/qualia**.

### 2.4 The drive (§1c, A32)
> maximize the growth of grounded intelligence/understanding, as fast as safely possible.

Measurable objective `J`; fastest-first; "recode itself" means **online re-parameterization** (fusion weights, quant levels, `τ`, heads), **never engine-source rewriting**. Compression & retrieval are **facets, not trade-offs**.

### 2.5 Zero-cost & self-built (§1d, A34)
> **Everything is free, and we build it ourselves in Rust.**

No paid services/SaaS/cloud DB/CI/scanners. Every engine hand-built in Rust. Only conceivable paid item: a great embedder (doubted to exist).

### 2.6 Strict source-of-theory (§2, A24)
Theory is **strictly and only** the Royse corpus (papers *Calculus of Association*, *The Oracle and the Kernel*, *The Symmetry of Knowing*; proven systems ContextGraph/Polis/ClipCannon/Leapable; the video transcripts). No external theory of intelligence imported. External technique is frozen to founder-requested/platform-reality scaffolding only (TurboQuant, grouped GEMM / Blackwell / CUDA / ZFS, FoundationDB-style key-encoding pattern).

### 2.7 Other binding rules
- **§3 Universality mandate** — serve the root purpose of every paradigm (relational/document/KV/columnar/graph/time-series/full-text/vector/blob) from first principles, not by bolting on engines. Full-text search = a sparse lens; a vector DB = a 1-lens Calyx.
- **§4 AGI/Oracle/kernel mandate** — bake in the Oracle, the kernel at any scope, automatic intelligence extraction.
- **§5 The backbone rule** — *plug-in lenses is THE key*: a new lens is one call (`add_lens`), its value one number (`bits`), the kernel one call at any scope. Reject any change that makes this harder.
- **§6 The 34 axioms (A1–A34)** — full text in `02_VISION_AND_AXIOMS.md`. Key ones: A1 record=constellation · A2 grounding mandatory · **A3 no-flatten** · A4 frozen lenses · A5 hot-swap · A7 differentiation contract (**≥0.05 bits, ≤0.6 corr**) · A8 DPI ceiling · A10 kernel at any scope · A12 `Gτ` everywhere generation touches the store · A13 Rust + baked-in GPU math · A15 source-of-truth verification · **A16 fail closed** · A19 universality · A20 the Oracle · A24 strict Royse theory · A25 measured compression · A33 security/privacy by construction · A34 zero-cost.
- **§7 Bake the formulas in** — every Royse formula is a callable, self-tuning backend primitive.
- **§8 Engineering rules** — **fail closed** (unknown lens, dim mismatch, ungrounded result, corrupt shard → structured `CALYX_*` error, never a silent fallback); provenance always (Ledger); FSV (harnesses banned); **code/test files ≤ 500 lines (HARD)**, docs unlimited; one change at a time.
- **§8b/§8c/§8d/§8e** — protocols (`AICodingAgentSuperPrompt.md`, `modulateprompt.md`, `compressionprompt.md`); **no CI — FSV is CI**; everything runs on **aiwonder** (WSL authors only); secrets via Infisical (`hf_hub_token`); dev state in GitHub Issues; **Synapse is the computer-use & orchestration runtime** (preferred over the subagent tool).
- **§9 Anti-patterns (refuse)** — flattening the panel; selling `C(N,2)` past the DPI ceiling; labeling ungrounded results "trusted"; mutating a frozen lens; substituting an external theory; making lens plug-in harder; a green-checkmark harness standing in for FSV; >500-line file without a tracking issue; bolting on a separate search/graph/vector DB.

The **`BUILD_DONE` predicate** (`00_INDEX.md`, full def in `19_ROADMAP_FSV_BUILD_DONE.md`) is a mechanical conjunction of ~30 clauses (CORE ∧ LENS ∧ DDA ∧ BITS ∧ KERNEL ∧ GUARD ∧ SEARCH ∧ UNIVERSAL ∧ ORACLE ∧ … ∧ FSV); Calyx is "complete" only when all are true.

---

## 3. PRD plan docs — `docs/dbprdplans/` (table, grouped by theme)

Filename → title → what it specifies (design intent). Cross-references point to the per-subsystem reference doc that documents the shipped code.

### 3.1 Foundation & vision
| Doc | Title | What it specifies | Reference doc |
|---|---|---|---|
| `00_INDEX.md` | Master PRD Index | Product framing, reading order, the canonical glossary (Lens/Slot/Panel/Constellation/Cross-term/DDA/`Gτ`/kernel/Oracle), hardware ground truth, the `BUILD_DONE` predicate. | — |
| `01_BRAND_AND_NAMING.md` | Brand & naming | Establishes the name *Calyx* and the celestial-instrument subsystem codenames (Aster, Loom, Assay, Lodestar, Ward, Sextant, Ledger, Anneal, Forge, Registry); controlled vocabulary, one term per concept. | — |
| `02_VISION_AND_AXIOMS.md` | Vision & 34 axioms | The four verbs (measure/count/differentiate/compose) as first-class DB operations; "why not a vector DB"; the binding axioms A1–A34 every later doc is checked against. | `01_system_overview.md` |
| `03_DATA_MODEL.md` | Data model | The logical hierarchy Vault → Collection → Panel → Slot → Lens → Constellation/TCT; slot-vectors, cross-terms, scalars, anchors, provenance; content-addressed IDs for idempotency. | `05_core.md`, `04_storage_and_schema.md` |

### 3.2 Engine (storage, math, lenses, search)
| Doc | Title | What it specifies | Reference doc |
|---|---|---|---|
| `04_ASTER_STORAGE_FORMAT.md` | Aster on-disk format | A custom columnar transactional LSM (`.aster`): one ordered keyspace, association-native column families (base/slot/anchors/cross-term/kernel/bits/guard/ledger), tiering, ZFS layout, crash safety. | `06_aster_storage_engine.md`, `04_storage_and_schema.md` |
| `13_FORGE_MATH_RUNTIME.md` | Forge math runtime | Baked-in matmul/BLAS/SIMD/GPU (CUDA sm_120 + AVX-512 + ONNX), quantization, autotuned kernels, CPU↔GPU bit-parity contract; no external math service on the hot path. | `07_forge_math_runtime.md` |
| `05_EMBEDDER_REGISTRY.md` | Registry / lenses | Lenses as designable, frozen, content-addressed instruments; hot-swap add/retire with lazy backfill; the frozen contract; capability assay. | `08_registry_lenses.md` |
| `10_SEXTANT_SEARCH_NAV.md` | Sextant search/nav | Multi-lens search, RRF/WeightedRRF/SingleLens fusion, per-slot ANN, sparse lexical lens, query planner/explain, navigation modes; the universal query surface. | `09_sextant_search.md` |

### 3.3 Intelligence (associations, bits, kernel, guard, oracle, math)
| Doc | Title | What it specifies | Reference doc |
|---|---|---|---|
| `06_LOOM_DDA_ENGINE.md` | Loom / DDA | Derived Data Abundance: cross-term weaving (`C(N,2)` associations-between-associations), lazy/eager materialization, effective-rank cap, DPI bound. | `10_loom_associations.md` |
| `07_ASSAY_SIGNAL_BITS.md` | Assay / signal bits | MI estimation (KSG / partitioned-NMI), the differentiation contract (≥0.05 bits, ≤0.6 corr), `n_eff`, sufficiency, oracle self-consistency. | `11_assay_signal_bits.md` |
| `08_LODESTAR_KERNEL.md` | Lodestar / kernel | Grounding-kernel discovery via directed MFVS (~1% feedback-vertex-set), kernel-as-index + kernel-based answering, kernel at ANY scope. | `12_lodestar_kernel.md`, `17_graph_mincut_paths.md` |
| `09_WARD_TCT_GUARD.md` | Ward / `Gτ` guard | Per-output cosine guard `Gτ`, per-slot conformal `τ` calibration, novelty→new-region, injection defense, identity-locked generation. | `13_ward_guard.md` |
| `21_ORACLE_AND_AGI.md` | Oracle & AGI | Consequence prediction (world model / butterfly effect), oracle self-consistency ceiling, panel sufficiency `I(panel;oracle)`, falsifiable per-domain super-intelligence predicate, epistemic symmetry (Q↔A). | `16_oracle_prediction.md` |
| `22_FORMULA_LIBRARY.md` | Formula library | Every Royse formula baked in as a callable, self-tuning primitive (DDA, differentiation contract, DPI bound, MFVS, `Gτ`, oracle ceiling, RRF, Q↔A). | `16_oracle_prediction.md`, `11_assay_signal_bits.md` |
| `26_ADVANCED_MATH_FRONTIERS.md` | Advanced math | Spectral structure of the association graph, energy/associative pattern completion, transfer-entropy causality, total-correlation `n_eff`, the unified `complete()` (predict=abduce=impute), grounded label propagation. | `16_oracle_prediction.md` |
| `27_INTELLIGENCE_OBJECTIVE.md` | Intelligence objective `J` | The measurable composite `J` Calyx maximizes; fastest-first growth; the math self-adjusts parameters online; compression-without-loss + retrieval efficiency as facets; honesty/safety bounds. | `15_anneal_optimization.md` |

### 3.4 Self-optimization, provenance, temporal, resources
| Doc | Title | What it specifies | Reference doc |
|---|---|---|---|
| `11_LEDGER_PROVENANCE.md` | Ledger / provenance | Append-only hash-chain witness, Merkle checkpoints, audit, reproducibility, tamper-evidence; stores hashes not secrets. | `14_ledger_provenance.md` |
| `12_ANNEAL_SELF_OPTIMIZATION.md` | Anneal / self-opt | Three background loops (self-heal, self-learn via mistake-closure, self-optimize math/index params); shadow-tested, reversible, Ledger-logged changes. | `15_anneal_optimization.md` |
| `23_ARRAY_MATH_STORAGE_COMPRESSION.md` | Array math & compression | Constellation as one co-located array bundle; all vector math as grouped GEMM on sm_120; TurboQuant + MXFP4 microscaling; **maximal compression gated by measured intelligence**. | `06_aster_storage_engine.md`, `07_forge_math_runtime.md` |
| `24_MEMORY_GC_RELIABILITY.md` | Memory / GC / reliability | No managed GC (RAII + arenas, bounded by construction), VRAM budgeting, six categories of garbage reclaimers, long-reader/MVCC hazards, the 25-row hazard register. | `18_hazard_soak_and_testkit.md` |
| `25_TEMPORAL_AND_DEDUP.md` | Temporal & dedup | Three frozen temporal lenses E2/E3/E4 (recency/periodic/sequence, retrieval-only under AP-60 never-dominant); recurrence series; TCT cosine-`Gτ` dedup; oracle predicts next occurrence. | `06_aster_storage_engine.md`, `08_registry_lenses.md` |

### 3.5 Universality, interface, deployment, ops/governance
| Doc | Title | What it specifies | Reference doc |
|---|---|---|---|
| `20_UNIVERSAL_DB.md` | Universal DB | First-principles universality: every paradigm's root purpose, the 3-layer architecture (ordered core + general data layer + Association Engine), collections-as-any-model, deployment profiles. | `01_system_overview.md`, `04_storage_and_schema.md` |
| `14_MCP_AGENT_INTERFACE.md` | MCP agent interface | ~30 typed, self-describing MCP tools so agents compose multi-embedder systems with zero plumbing; vault/lens/ingest/search/guard/provenance operations. | `19_mcp_api_tools_reference.md` |
| `18_API_TYPES_ERRORS.md` | API types & errors | The Rust crate layout, core types (VaultId/LensId/SlotVector/Anchor), wire schema, the `CALYX_*` error-code catalog, the ≤500-line rule. | `05_core.md`, `03_configuration.md` |
| `15_LEAPABLE_INTEGRATION.md` | Leapable integration | Calyx replaces **only** the Leapable SQLite/`sqlite-vec` Vaults; PostgreSQL control plane untouched; preserves vault interface names; migration, embedded↔server. | `20_cli_and_daemon_reference.md` |
| `16_AIWONDER_DEPLOYMENT.md` | aiwonder deployment | Hardware mapping, systemd, GPU policy, ZFS, networking, restic backup, observability for `calyxd` as a self-contained build under `/home/croyse/calyx`. | `20_cli_and_daemon_reference.md` |
| `30_SECURITY_PRIVACY_GOVERNANCE.md` | Security/privacy/governance | STRIDE threat model, least-privilege, default-deny tenant isolation, encryption at rest+in transit, right-to-erasure via crypto-shredding (resolves the A25 tension), honest cold-start (A33). | `13_ward_guard.md`, `14_ledger_provenance.md` |
| `17_JOHARI_BLINDSPOTS.md` | Johari blindspots | Known/unknown unknowns risk register across 14 failure axes; mitigations mapped to FSV gates; hidden capabilities (anomaly detection, kernel summarization, reactive triggers). | `18_hazard_soak_and_testkit.md` |

### 3.6 Process / build governance (meta — about how Calyx is built)
| Doc | Title | What it specifies | Reference doc |
|---|---|---|---|
| `19_ROADMAP_FSV_BUILD_DONE.md` | Roadmap, FSV, BUILD_DONE | Phased build (P0–P12+), the FSV protocol, milestones, perf targets, the mechanical `BUILD_DONE` predicate; thresholds (`X`,`Y`,`Δ`) fixed here. | `21_test_suite.md`, `22_verification_report.md`, `24_roadmap.md` |
| `28_FSV_AND_TEST_DATA.md` | FSV & test data | FSV per aspect; synthetic-mechanics vs real-intelligence data; the dataset catalog; acquisition-as-FSV; secrets via Infisical; everything built/run/tested on aiwonder. | `21_test_suite.md`, `28_STAGE18_DATASETS_FSV.md` |
| `29_STATE_GITHUB_ISSUES.md` | Dev state via GitHub Issues | Pinned `type:context` issues every agent reads each turn; kept current by editing to truth (never appending contradictions); pruned every phase. | `24_roadmap.md` |
| `31_SYNAPSE_COMPUTER_USE.md` | Synapse computer use | Synapse = the computer-use & agent-orchestration dev runtime: perceive/act on the real machine, open terminals, command Claude/Codex agents (preferred over subagents); reality-audit as FSV's perception arm. | — (dev tooling) |

---

## 4. Implementation stage docs — `docs/implementation/`

The implementation plan reorganizes the PRD into **phases PH00–PH72** grouped into **stages S0–S20**, each mapped to crate(s) with a single byte-level FSV exit gate. Phase IDs are stable handles used in GitHub issues/commits. The dependency spine (critical path): `S0 → S1 Aster → S2 Forge → S3 Registry → S4 Sextant → S5 Loom/Assay`, with Lodestar/Ward/Ledger/universal/resource branching off.

### 4.1 Working-agreement rules (`02_WORKING_AGREEMENT.md`)
A phase is DONE only when **all** hold:
1. `cargo check` + `cargo clippy -D warnings` + `cargo test` green **on aiwonder**.
2. **≤500-line file-size gate** passes (`scripts/linecount.sh`); over-limit → `type:task` issue + modularize first.
3. **CPU↔GPU bit-parity** holds for Forge-touching code (≤1e-3 rel tol).
4. **FSV exit gate met** — proven by reading persisted bytes on aiwonder, not a return value, not a harness; evidence attached to the GitHub issue.
5. **Provenance + fail-closed** wired: Ledger entry on every mutation; every error path returns a structured `CALYX_*` code, never a silent fallback (A16).
6. **Context issues updated** (`[CONTEXT] You are here`).

Other binding rules carried into every phase:
- **FSV protocol (5 steps, via Synapse):** identify bytes → read before → execute → read after → inspect delta. **FSV harnesses are banned.**
- **Tests support FSV, don't replace it:** two questions (fails-when-wrong, passes-when-right); FIRST + properties; seed all RNG, inject the clock (never `SystemTime::now()` in logic); proptest/cargo-fuzz/cargo-mutants/criterion (all free OSS). Zero tolerance for flakiness.
- **No CI — FSV is our CI** (`scripts/check.sh` run on aiwonder, agent-invoked).
- **Code reuse:** lift proven ContextGraph (`mincut`/`paths`/`solver`/`witness`) + `mejepa` (Assay/kernel/guard) seeds by **copying source** into Calyx crates, never linking the live project.
- **Dev-state on GitHub Issues** (`chrisroyse/calyx-dev`): pinned `type:context` issues; edit to truth, never append contradictions; prune every phase.
- **Orchestration via Synapse:** open real terminals, command Claude (`cldy`)/Codex (`codex --yolo`) workers — preferred over the subagent tool. Humans direct + approve outward/destructive actions only.
- **Doctrine compliance checklist** before closing any phase: no panel flattening (A3); no `C(N,2)` past DPI (A8); nothing "trusted" without grounding (A2 → say `provisional`); no frozen-lens mutation; no external theory (A24); lens plug-in/bits/kernel never made harder; no harness for FSV; no >500-line file without an issue; no bolt-on search/graph/vector DB.

### 4.2 Stage docs (table)
Stage doc → stage name → what it covers → crate(s). (Status as of `03_PHASE_MAP.md` dated 2026-06-10: S0–S8 ✅ DONE/FSV-signed-off; S9 ▶ ACTIVE; S10–S20 pending.)

| Stage doc | Stage | What it covers (phases) | Crate(s) |
|---|---|---|---|
| `10_STAGE0_FOUNDATION.md` | S0 Foundation | aiwonder bootstrap, workspace + line-count gate, GitHub context issues, `calyx-core` (IDs, enums, `CALYX_*` errors, constellation structs, engine traits, injected `Clock`). PH00–PH04. | `calyx-core` |
| `11_STAGE1_ASTER.md` | S1 Aster | WAL + group-commit, memtable + LSM SSTable, column families + key codecs, MVCC snapshots, constellation CRUD + idempotent ingest, manifest + crash recovery, compaction + hot/cold tiering. PH05–PH11. | `calyx-aster` (+ `calyx-cli`, `calyx-testkit`) |
| `12_STAGE2_FORGE.md` | S2 Forge | CPU SIMD backend (gemm/cosine/l2/normalize/topk, AVX-512), CUDA sm_120 + bit-parity, TurboQuant, MXFP4/grouped GEMM, per-shape autotune cache. PH12–PH16. | `calyx-forge` |
| `13_STAGE3_REGISTRY.md` | S3 Registry | Uniform `Registry.measure` over algorithmic/TEI-HTTP/candle-local/ONNX runtimes, frozen contract + content-addressed `LensId`, hot-swap add/retire/park + lazy backfill, capability cards, default panels + temporal lenses E2/E3/E4. PH17–PH22. | `calyx-registry` |
| `14_STAGE4_SEXTANT.md` | S4 Sextant | Per-slot dense (HNSW) + sparse (inverted/BM25) indexes, RRF/WeightedRRF/SingleLens fusion with provenance, query planner/intent/explain/freshness. PH23–PH26. | `calyx-sextant` |
| `15_STAGE5_LOOM_ASSAY.md` | S5 Loom + Assay | Agreement graph + lazy cross-terms + abundance reports; KSG MI, partitioned NMI, bootstrap CI, logistic probe, differentiation contract, `n_eff`, sufficiency, attribution. PH27–PH30. | `calyx-loom`, `calyx-assay` |
| `16_STAGE6_LODESTAR.md` | S6 Lodestar | Graph primitives (SCC, betweenness, LP); kernel-graph + directed MFVS; kernel index + `kernel_answer` + grounding gaps; multi-scope kernel (≥4 scopes). PH31–PH34. | `calyx-paths`, `calyx-mincut`, `calyx-lodestar` |
| `17_STAGE7_LEDGER.md` | S7 Ledger | Hash-chain append-only CF in group-commit; Merkle checkpoints + `verify_chain` + `reproduce()`. PH35–PH36. | `calyx-ledger` |
| `18_STAGE8_WARD.md` | S8 Ward | `Gτ` guard math + GuardProfile; conformal `τ` calibration + novelty→new-region; identity-locked generation (speaker/style). PH37–PH39. | `calyx-ward` |
| `19_STAGE9_TEMPORAL_DEDUP.md` | S9 Temporal & Dedup | Temporal fusion + AP-60 post-retrieval boost (E2/E3/E4 never dominant); DedupPolicy TctCosine + recurrence series + signature; grounded recurrence wiring across engines. PH40–PH42. | `calyx-sextant`, `calyx-aster`, `calyx-loom` (cross) |
| `20_STAGE10_ANNEAL_J.md` | S10 Anneal + `J` | Tripwires + shadow-first + reversible rollback; self-heal; mistake-closure + online heads; autotune loops; `J` objective + growth curve + `intelligence_report`. PH43–PH48. | `calyx-anneal` |
| `21_STAGE11_ORACLE_AGI.md` | S11 Oracle & AGI | Consequence prediction + sufficiency gate; super-intelligence predicate + `reverse_query`; `complete()` unified primitive; advanced math (spectral/energy/transfer-entropy/TC/Bayesian). PH49–PH52. | `calyx-oracle` (+ `calyx-assay`) |
| `22_STAGE12_UNIVERSAL.md` | S12 Universal data layer | Collections-as-any-model (relational/doc/KV/TS/blob); secondary indexes (btree/inverted); cross-model transactions + universal query surface. PH53–PH55. | `calyx-aster` (layers), `calyx-sextant` |
| `23_STAGE13_RESOURCE_GC.md` | S13 Resource/GC | Bounded caches/queues/memtables + arenas/pools; VRAM budgeter + admission control; GC reclaimers + long-reader watchdog + janitor; 25-hazard register FSV + soak. PH56–PH59. | `calyx-aster`, `calyx-core`, `calyx-forge`, `calyx-anneal` (cross) |
| `24_STAGE14_SECURITY.md` | S14 Security & privacy | Encryption at rest/in transit + tenant isolation; crypto-shred erasure + STRIDE FSV + secret-scan. PH60–PH61. | `calyx-aster`, `calyxd` (cross) |
| `25_STAGE15_INTERFACES.md` | S15 Interfaces | `calyx-cli` (vault/lens/ingest/search/readback); `calyx-mcp` (stdio embedded tool surface); migration tool (sqlite→calyx). PH62–PH64. | `calyx-cli`, `calyx-mcp` |
| `26_STAGE16_SERVER_DEPLOY.md` | S16 Server & deploy | `calyxd` daemon (loopback, healthcheck); systemd + ZFS provisioning + Prometheus/Grafana; restic backup + DR drill. PH65–PH67. | `calyxd`, infra |
| `27_STAGE17_SCALE.md` | S17 Scale | DiskANN dense + SPANN sparse for 1e8–1e9 constellations within search SLO; disk-resident graphs. PH68. | `calyx-sextant` |
| `28_STAGE18_DATASETS_FSV.md` | S18 Datasets & intelligence FSV | Dataset acquisition + MANIFEST + checksum FSV (≥1 verified dataset per modality×outcome); intelligence validation (recall/bits/kernel/oracle/J) on real corpora. PH69–PH70. | (cross) |
| `29_STAGE19_LEAPABLE.md` | S19 Leapable vault swap | V0 shadow → V1 flip → V2 calyx-only migration of the Leapable Vault; PostgreSQL untouched (verified). PH71. | `calyx-cli`, `calyx-mcp` |
| `30_STAGE20_CRITICAL_CAPS.md` | S20 Critical capabilities | Streaming ingest + reactive triggers + time-travel/as-of + universal summarization. PH72. | (cross) |

### 4.3 Supporting implementation docs
| Doc | What it is |
|---|---|
| `00_README.md` | Master implementation README: framing facts (everything on aiwonder; self-contained; FSV is the gate), how the plan is organized, numbering, dependency spine, engine→crate→stage cheat sheet, current build status. |
| `01_AIWONDER_ENVIRONMENT.md` | The real aiwonder box (live readback): hardware (RTX 5090 sm_120, 128 GB RAM, ZFS), toolchain, resident TEI lenses, services, the self-contained Calyx layout, connect procedure, sudo constraint. |
| `02_BUILD_PERFORMANCE.md` | Build-optimization strategies (debug profile, linker, incremental cache) to keep disk bounded and compilation fast on the shared build host. |
| `03_PHASE_MAP.md` | The master table of every phase PH00–PH72: stage, deps, crate, PRD/axiom mapping, one-line FSV exit gate, current status, evidence roots, critical path, `BUILD_DONE` mapping. |
| `FSV_NOTES.md` | Convention that FSV tools print source-of-truth bytes without verdicts; documents the readback surfaces (bytes, vault trees, CF rows, WAL, SST). |
| `STAGE1_5_EVIDENCE_MANIFEST.md` | Stage 1–5 audit index: PH05–PH30 evidence roots, commands, artifact hashes, source-of-truth summaries, live deferral-owner issues. |
| `PHASE_TASKS_README.md` + `PHnn-*/` subdirs | Per-phase atomic task-card convention; one subdir per phase (PH05–PH74) holding a `README.md` + `Tnn-*.md` task cards. Stage 0 has no subdir (already built). |

---

## 5. `docs2/` — additional reference files

High-level only; these are founder prompt-docs, guides, and the source papers (the latter three PDFs are the Royse corpus theory — the canonical source per DOCTRINE §2).

| File | One-line |
|---|---|
| `AICodingAgentSuperPrompt.md` | Binding AI-coding-agent doctrine: aiwonder as the only FSV-valid environment, manual verification, GitHub-issue state management (DOCTRINE §8b). |
| `compressionprompt.md` | Binding compression doctrine: maximize specificity-per-token; preserve verbatim numbers/paths/error codes/axiom ids/formulas (DOCTRINE §8b). |
| `modulateprompt.md` | Binding file-modularization protocol: the ≤500-line rule and how to split into SRP module dirs with a thin facade, 100% API-compatible (DOCTRINE §8). |
| `infisical-secrets-guide.md` | Operational reference for the Leapable Infisical vault (key catalog, access from Windows, rotation/verification). |
| `releaseguide.md` | Operational checklist for deploying the socialmedia2.com static site via Cloudflare Pages (not Calyx-core; cross-project). |
| `latestyoutubevideosgodstuff.md` | YouTube video transcript/metadata fragment (part of the Royse video-transcript corpus). |
| `FormulaForAGI.pdf` | Source paper (Royse corpus) — the formula for AGI / intelligence-as-association. |
| `TheOracleandtheKernel.pdf` | Source paper (Royse corpus, 4 Jun 2026) — the Oracle (consequence prediction) and the grounding kernel. |
| `TheSymmetryOfKnowing_Revised.pdf` | Source paper (Royse corpus) — epistemic symmetry (Q↔A bidirectional knowing). |

---

## 6. How plan ↔ code divergence is captured

These docs are **design intent**, not a claim of shipped behavior. Per `_SPEC.md`, where the implemented code diverges from a plan, the divergence lives in the per-subsystem reference docs:

- Storage format / on-disk reality → [04_storage_and_schema.md](04_storage_and_schema.md), [06_aster_storage_engine.md](06_aster_storage_engine.md)
- Lenses / Registry → [08_registry_lenses.md](08_registry_lenses.md)
- Search → [09_sextant_search.md](09_sextant_search.md)
- DDA / bits → [10_loom_associations.md](10_loom_associations.md), [11_assay_signal_bits.md](11_assay_signal_bits.md)
- Kernel / graph → [12_lodestar_kernel.md](12_lodestar_kernel.md), [17_graph_mincut_paths.md](17_graph_mincut_paths.md)
- Guard → [13_ward_guard.md](13_ward_guard.md)
- Provenance → [14_ledger_provenance.md](14_ledger_provenance.md)
- Self-opt / Oracle → [15_anneal_optimization.md](15_anneal_optimization.md), [16_oracle_prediction.md](16_oracle_prediction.md)
- MCP / CLI / daemon → [19_mcp_api_tools_reference.md](19_mcp_api_tools_reference.md), [20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md)
- Counts/health snapshot → [22_verification_report.md](22_verification_report.md); remaining work → [24_roadmap.md](24_roadmap.md)

A concrete plan-vs-shipped example already visible in the plan's own status text (`16_STAGE6_LODESTAR.md` / `00_README.md`): the kernel's "≈1% compact-kernel" is a **raw target**, while signed acceptance is the **measured final/tuned recall** with explicit `raw_recall`/`tuned_recall`/`pass_mode` — documented in [12_lodestar_kernel.md](12_lodestar_kernel.md).

## Gaps / not covered
- This is a navigation/intent map; it does **not** restate the full content of any plan doc. Read the source doc for detail.
- The 60+ `PHnn-*/` per-phase task-card subdirs are summarized at the convention level only, not card-by-card.
- The PDFs (`docs2/*.pdf`) are summarized from filename + DOCTRINE §2 context, not from reading the PDF bodies.
- Plan docs describe intent; for shipped behavior and any divergence, see the per-subsystem docs in §6.
