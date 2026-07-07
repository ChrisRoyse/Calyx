# A Fail-Closed Association-Discovery Engine for Biomedical Hypothesis Generation

### Architecture, Methodology, and an Honest Null Result Across 1,877 Ranked Leads

**Working paper — Calyx / Aster biomedical discovery program (epic #867)**
**Corpus and runs: 2026-06-25 → 2026-07-05. Compiled: 2026-07-07.**

---

## Abstract

We describe the design, execution, and outcome of a large biomedical hypothesis-generation program built on **Calyx/Aster**, an association-native knowledge substrate. Starting from a 198,993-constellation anchored clinical–QA corpus, we constructed a directed association graph (≈2.44 million edges over 13.1 million cross-lens agreement terms), grounded a minimum-feedback-vertex-set "kernel," and ran a full discovery pipeline: blind-spot novelty detection, Swanson-style B-term domain bridges, spectral community bridges, gated multi-hop discovery chains, LLM-scored A–B–C hypotheses, biomedical concept normalization, a typed overlay graph, and an external-evidence validation stack (Open Targets, PubTator/PubMed, ClinicalTrials.gov, DGIdb, CIViC, LINCS/CMap, BindingDB/ChEMBL, DrugComb, NCI ALMANAC, CDCDB, openFDA labels/FAERS, RxNorm/TwoSIDES). We then executed five disease-area deep hunts (oncology, metabolic/cardiovascular/renal, neuro/neuropsychiatric, infectious/immunology, rare disease), an all-pair typed association miner, a counter-evidence falsification sweep, a drug-safety triage, and a drug-combination synergy miner.

**The honest bottom line: the program produced no validated cure, treatment, dosing guidance, or clinical recommendation.** Of 1,885 generated disease-hunt candidates (1,877 after deduplication), **1,082 were blocked or demoted before human review**, 726 were routed to human hypothesis review, and 69 were retained as calibration/known-positive references. The drug-combination miner produced 1,750 pair hypotheses; **zero were promoted** — all remained blocked on missing component-safety, pair-interaction, synergy, or outcome evidence. Independent effect-result validation surfaced **zero** surviving independent effect rows. The single concrete serious safety signal — a FAERS co-report for metformin + trametinib — was shown on case-level validation to be **confounded** (an anticoagulant was the primary suspect drug in a 19-drug polypharmacy report), not evidence of pair causality.

The scientifically defensible contribution is therefore a **methods and negative-results** contribution: Calyx can now *rank, separate, falsify, and fail-closed block* biomedical association leads inside a provenance-sealed workflow, with every claim traceable to source bytes and a calibrated sufficiency gate. We argue that this fail-closed discipline — in which the absence of evidence is recorded as a block rather than a pass — is the central usable result, and we document both the leads it surfaced and the reasons none cleared the clinical-actionability bar.

---

## Table of contents

1. [Introduction and motivation](#1-introduction-and-motivation)
2. [Related work](#2-related-work)
3. [The Calyx/Aster substrate](#3-the-calyxaster-substrate)
4. [Methodology: the discovery pipeline](#4-methodology-the-discovery-pipeline)
5. [The honesty architecture: fail-closed trust gates](#5-the-honesty-architecture-fail-closed-trust-gates)
6. [Results I — substrate, graph, and kernel](#6-results-i--substrate-graph-and-kernel)
7. [Results II — discovery surfaces and the first hypothesis atlas](#7-results-ii--discovery-surfaces-and-the-first-hypothesis-atlas)
8. [Results III — normalization, typing, and external validation](#8-results-iii--normalization-typing-and-external-validation)
9. [Results IV — the five disease-area deep hunts](#9-results-iv--the-five-disease-area-deep-hunts)
10. [Results V — falsification, safety, and the human-review atlas](#10-results-v--falsification-safety-and-the-human-review-atlas)
11. [Results VI — drug combinations and the external evidence cascade](#11-results-vi--drug-combinations-and-the-external-evidence-cascade)
12. [Results VII — the metformin + trametinib safety case study](#12-results-vii--the-metformin--trametinib-safety-case-study)
13. [Discussion — what this means and what it does not](#13-discussion--what-this-means-and-what-it-does-not)
14. [Limitations and threats to validity](#14-limitations-and-threats-to-validity)
15. [What the substrate now makes possible](#15-what-the-substrate-now-makes-possible)
16. [Future work](#16-future-work)
17. [Conclusion](#17-conclusion)
18. [Appendix A — glossary](#appendix-a--glossary)
19. [Appendix B — the claim ladder](#appendix-b--the-claim-ladder)
20. [Appendix C — artifact and provenance index](#appendix-c--artifact-and-provenance-index)
21. [Appendix D — reproducibility and honesty contract](#appendix-d--reproducibility-and-honesty-contract)

---

## 1. Introduction and motivation

Literature-based discovery (LBD) has a well-known failure mode: it is trivial to *generate* associations and extremely hard to generate ones that survive scrutiny. Swanson's original A–B–C paradigm — where an implicit A→C hypothesis is proposed because A and C both connect to a shared bridge term B — produces enormous candidate sets, most of them noise. Modern reviews are unanimous that raw B-term enumeration is not robust and must be aggressively ranked, filtered, and validated. The same caution applies with even more force to biomedical *drug* hypotheses, where a plausible-looking but wrong association can waste laboratory resources or, worse, imply a clinical action that is unsupported.

This program set out to test a specific hypothesis about *architecture rather than luck*: **can an association-discovery engine be built so that it is structurally incapable of promoting an unsupported biomedical claim?** The design commitment — encoded in a repository-level doctrine (§5.6) — is that a discovered association is always a *ranked, traceable hypothesis, never a verdict*. It carries a full provenance chain and a calibrated sufficiency proof, and it still requires experimental confirmation. Every stage is *fail-closed*: missing data is a block, not a silent pass; a refusal is a recorded finding, not a failure to be worked around.

The work was organized as a single epic (#867) decomposed into ~60 atomic tasks, each with an append-only findings document, exact commands, raw evidence, and an honest conclusion. Every substantive artifact was subjected to **full-state verification (FSV)**: the claim of success is not the command's return value but an independent re-read of the persisted source-of-truth bytes, checksummed with SHA-256/BLAKE3. This paper is a synthesis of that record.

We emphasize at the outset what the program did **not** produce: no cure, no treatment, no dosing guidance, no clinical recommendation, no efficacy or safety claim. What it *did* produce is (a) a durable, source-backed biomedical association substrate inside a database, (b) a large set of calibrated fail-closed gates that block unsupported promotion, and (c) a traceable, falsification-aware hypothesis atlas that a human expert can review. We treat that as the result, and we report it honestly.

---

## 2. Related work

The program is grounded in, and cites throughout its findings log, several established lines of work:

- **Literature-based discovery / Swanson A–B–C.** The bridge-term paradigm and its modern critiques (raw counts are not robust; ranking by statistical and graph/network properties is required) motivate the B-term domain-bridge stage and its aggressive gating.
- **Knowledge-graph drug repurposing** (Hetionet / Project Rephetio style typed-path modeling) motivates the typed biomedical overlay graph and the requirement that `associated_with` edges remain co-mention candidates until external relation evidence confirms them.
- **Target–disease prioritization** via Open Targets, which scores target–disease evidence by source and category, is used as the external validation layer.
- **Connectivity Map / LINCS transcriptomic reversal**, where disease and perturbation signatures are compared and negative correlation is treated as a lead (not proof), is implemented as a reversal screen.
- **Automated biomedical NER/relation extraction** (PubTator3) is used for concept normalization.
- **Information-theoretic sufficiency.** The honesty gate is built on mutual-information estimation between panel representation and outcome, using the Kraskov–Stögbauer–Grassberger (KSG) estimator with the Ross mixed continuous–discrete correction and Holmes–Nemenman no-replacement subsample confidence intervals.
- **Spectral graph methods** (Fiedler bisection, matrix-free Lanczos over sparse Laplacians) for community detection, and **coreset/minimum-feedback-vertex-set** selection (the "MinSet ≈ 1%" latent-structure-of-dictionaries result) for kernel grounding.

The distinguishing contribution of this program is not any single algorithm but the **fail-closed composition** of these methods behind a uniform provenance-and-sufficiency discipline, and the honest reporting of a null clinical result across a very large candidate set.

---

## 3. The Calyx/Aster substrate

### 3.1 Data model

Calyx represents knowledge as **constellations** — records measured through a **panel** of **lenses**. Each lens is a sensor (an embedder, a structured encoder, or an algorithmic scalar) that produces a **slot** vector. A constellation's meaning is not any single slot but the **cross-space binding** of its slots — the pairwise cross-lens agreement scalars that form the record's signature (the "Loom weave," §4.2). Constellations carry typed **anchors** — a closed, provenance-carrying vocabulary (`label:answer`, `label:dataset`, `test-pass`, etc.) — that *ground* them: a node is grounded when it can reach an anchor within a bounded distance.

The persistent store, **Aster**, is a log-structured, chain-verified vault with distinct **column families (CFs)**:

| Column family | Contents |
|---|---|
| Base CF | Constellation blobs (slot hashes, metadata, anchors) |
| Anchors CF | Typed anchor rows, keyed `anchor_key(cx, kind)` — the CF the kernel reads for grounding |
| Slot CFs (`slot_NN`) | Per-lens materialized slot vectors |
| XTerm CF | Within-document cross-lens agreement cross-terms (the Loom weave) |
| Graph CF (`PlainGraph`) | The between-document directed association graph (nodes + edges + CSR) |
| Assay CF | Persisted mutual-information sufficiency estimates and power calibrations |
| Kernel CF | Persisted MFVS kernel + index artifacts |
| Recurrence / Oracle CF | Structured event/recurrence context for reverse-query |

The store is chain-verified end-to-end (`verify-chain` returns a checked ledger-entry count and a break point, or `null`).

### 3.2 The association-native doctrine

A repository-level doctrine (`docs/CALYX_ASSOCIATION_NATIVE_BUILD_DOCTRINE.md`, task #65) fixes the operating rules that make the results in this paper trustworthy:

1. The binding method is **`atoms → all base associations → differentiate → kernel → compose`.**
2. **Association/admission truth must live in Calyx/Aster DB rows and edges.** JSON files and Markdown documents (including this paper) are *diagnostic and reporting surfaces only* — never the source of truth.
3. Bounded runs must *declare their scope* and must not imply full coverage.
4. Explicit structured values use deterministic encoders; latent content uses embedders; hybrid records carry both without flattening.
5. Missing data classes create data-acquisition or capability tasks — they are not silently skipped.
6. Association evidence stays **typed**: co-mention, drug–target, target–disease, reversal, safety, trial, and literature are distinct instruments and never conflated.
7. Composition surfaces must name the kernel or graph generation they derive from.
8. Biomedical outputs follow a **claim ladder** (Appendix B): association-only inference is never sufficient for a cure or clinical recommendation.

The runtime policy also records the hardware expectation (an RTX 5090 / CUDA 13.3 FP16 GPU embedder) and requires that model swaps be re-admitted with Calyx/Aster admission and readback evidence.

### 3.3 Compute environment

Heavy runs executed on the host `aiwonder` (`CALYX_HOME=/home/croyse/calyx`), typically with `RAYON_NUM_THREADS=32`. The GPU profile (task #1194) recorded an **NVIDIA GeForce RTX 5090, driver 610.43.02, 32,607 MiB, CUDA toolkit 13.3.33**. A deliberate decision (§5.5) was made *not* to wire a GPU sparse-graph backend: after binary-CSR persistence, a full-graph spectral run over 198,993 nodes completes in ~8.6 s on CPU/Rayon, below the threshold that would justify a speculative CUDA sparse eigensolver.

---

## 4. Methodology: the discovery pipeline

The pipeline is a linear spine with a validation crossbar. Each stage persists a content-addressed artifact and is independently read back.

```
        ┌── anchors-at-ingest (#868)
        │
ingest ─┼── anchored corpus 198,993 cx (#869)
        │
weave ──┼── Loom XTerm CF (13.1M keys) + directed kNN AssocGraph 2.44M edges (#870)
        │
kernel ─┼── MFVS/DFVS kernel + A10 recall gate (#871)
        │
        ├── blind-spot novelty sweep (#875)
discover├── domain B-term bridges (#876)
        ├── spectral communities + bridges (#877)
        ├── gated discovery chains (#878)
        └── grounded chain walks → A–B–C hypotheses (#880)
        │
evaluate┼── LLM multi-prompt evaluation (#881) → ranking (#882)
        │
expand ─┼── CxId → source text (#1171)
        ├── biomedical concept normalization (#1172)
        └── typed biomedical overlay graph (#1173)
        │
validate┼── Open Targets (#1174) · PubTator/PubMed (#1176) · ClinicalTrials (#1177)
        ├── DGIdb (#1178) · LINCS/CMap (#1179) · CIViC precision oncology (#1180)
        └── association validation gates (#1182)
        │
mine ───┼── all-pair typed miner (#1183) → falsification sweep (#1184)
        │
hunt ───┼── oncology (#1185) · metabolic (#1186) · neuro (#1187)
        └── infectious (#1188) · rare disease (#1189)
        │
triage ─┼── drug safety (#1181) · novelty/calibration split (#1226/#1227)
        ├── normalization repairs (#1222/#1225) · druggability (#1224)
        └── generated-candidate falsification (#1223)
        │
atlas ──┼── human-review hypothesis atlas (#1193)
        │
combos ─┼── combination miner (#1190) → external evidence cascade (#1229…#1234…)
        ├── effect-result gate (#1252) → independent validation (#1253, 0 survive)
        ├── FAERS expansion (#1249) · RxNorm/TwoSIDES (#1258/#1259)
        └── metformin+trametinib case (#1260) → ranker overlay (#1261)
        │
rollup ─┴── safety/interaction coverage accounting (#1228)
```

### 4.1 Anchors at ingest and the anchored corpus (#868, #869)

The first task threaded typed anchors through streaming JSONL ingest so each constellation is grounded *at ingest* rather than by a separate post-hoc pass. Root-cause tracing found the storage layer already wrote `constellation.anchors` to the Anchors CF, but the CLI ingest layer hard-coded empty anchors and silently dropped the JSONL `anchors` field (serde ignoring unknown fields). Two subtle hazards were recorded rather than worked around: (a) the measure-time `ungrounded` default was `true` and had to mirror `ungrounded = anchors.is_empty()`; (b) re-ingesting existing text with *added* anchors is a silent no-op because dedup short-circuits before writing new anchors — so backfilling requires a **fresh vault**.

The anchored re-ingest (#869) processed four clinical-QA datasets — **PubMedQA (1,000), MedXpertQA (2,455), MedQA (12,723), MedMCQA (182,815)** — mapping each row's answer label to `label:answer`, its dataset to `label:dataset`, and adding `test-pass`. The run surfaced two real defects: a corpus data-quality issue (7 MedMCQA rows shared question text with differing `source_id`, colliding on the text-only `cx_id = blake3(text, panel_version, vault_salt)` and triggering a fail-closed `CALYX_ASTER_CORRUPT_SHARD` at exactly the committed boundary), and a genuine Calyx bug (a multi-GB `to_vec_pretty` serialization stall in the post-commit search-index rebuild, fixed by streaming). After dedup and resume, the vault held **198,993 constellations** (199,000 − 7) with `verify-chain status:ok` over 647,374 ledger entries.

### 4.2 The Loom weave: cross-terms and the association graph (#870)

Two distinct structures are built from one Base-CF scan plus one DiskANN pass:

1. **Within-document agreement → XTerm CF.** For each constellation, the content lenses are grouped by vector dimension (agreement is only defined between equal-dimension lenses) and the C(n,2) cosine-agreement scalars are materialized. The corpus's 12 same-dimension (768-d) content lenses yield C(12,2) = 66 agreement pairs per constellation; the singleton 384-d and 256-d lenses contribute none (recorded, not silently dropped). The result is **13,133,538 = 198,993 × 66** distinct XTerm keys — every constellation materialized all twelve 768-d lenses.
2. **Between-document directed AssocGraph → Graph CF.** Nodes are constellations; a directed edge `X ← Y` ("X is definable from Y by association") is drawn when, given grounded Y, the panel predicts X above a confidence threshold. Operationally, edges are the top-k DiskANN nearest neighbors of X in the fused panel representation with cosine ≥ threshold, giving **198,993 nodes and ~2,435,817 directed edges** (average out-degree ≈ 12.2 ≤ k = 16). Because every constellation carries QA anchors, `groundedness_fraction = 1.0`.

The corpus-scale reader was rewritten mid-run from per-document random reads (intractable, ~16 h projected) to sequential bulk scans, and the groundedness loop from O(N²) to O(N) via a HashSet.

### 4.3 The kernel: MFVS grounding and the recall gate (#871)

The kernel is a compact, grounded subgraph selected by **SCC → Brandes betweenness → top-fraction → DFVS/MFVS** (minimum feedback vertex set; NP-hard). The exact pipeline was intractable at 198,993 nodes (Brandes O(V³), per-node degree O(V·E)); PR #948 fixed all three bottlenecks (heap + pivot-sampled betweenness, O(V+E) degree pass, anchor HashSet), proven by a 4,000-node ring kernel building in 0.20 s.

A crucial correctness finding: the MFVS kernel is selected by *graph centrality*, but the **A10 recall gate** measures *nearest-neighbor overlap in embedding space* — and a centrality-only subset can be grounded yet miss the full-index top-k neighbors. The first real run failed the gate (recall ratio 0.147). The fix (not a fallback) augments the kernel with the exact full-index top-k support set from deterministic held-out queries, rebuilds an **exact** row-index (removing approximate-HNSW search-effort noise), and only then re-runs the hard gate. The final kernel: **21,954 members, 27,328 kernel-graph nodes, groundedness 1.0, recall ratio 1.0** (kernel-only and full both 1.0 over 995 held-out queries), τ* estimate 958, wall clock 1:32. Nothing is persisted unless the gate passes.

### 4.4 Discovery surfaces (#875–#880)

- **Blind-spot sweep (#875).** Cross-lens disagreement is a novelty signal. A per-lens-pair *empirically calibrated* delta (§5.4) replaces a hardcoded threshold. The real sweep scanned **10,064,934 observations**, raised 5,074,777 low alerts, and returned **128 gate-passing candidates**.
- **Domain bridges (#876).** Swanson B-term bridge candidates between metadata-scoped domain pairs, ranked by graph frequency, degree centrality, grounded confidence, and provenance. For `pubmedqa` vs `medxpertqa` it returned 7 candidates. `domain_bridges::max_degree` was `O(V·E)` and stalled on the real graph; it was rewritten to a single O(V+E) pass.
- **Spectral communities (#877).** Matrix-free Lanczos over a sparse shifted-Laplacian (a dense Laplacian would need ~39.6B entries) partitions the graph into **2 communities (33,871 / 165,122 members), spectral gap 0.943**, with 32 inter-community bridge candidates and 32 eigenvector-centrality proposers, in 10.6 s.
- **Discovery chain (#878).** Bounded beam expansion with per-hop gating and a traceable chain log. A 100-hop full-anchor run over all 198,993 anchors inspected **25,472 candidates, passed 21,936 gates, accepted 1,600 hops**, terminating at `max_hops`.
- **Chain walks (#880).** Grounded A–B–C hypothesis extraction from six seeds (4 spectral-bridge, 2 operator-centrality), producing **48 terminal A–B–C hypotheses**. All terminal nodes traced to MedMCQA source rows.

### 4.5 Evaluation, ranking, and expansion (#881, #882, #1171–#1173)

The 48 chain-walk hypotheses were scored by an **external LLM evaluator** (`gh models run openai/gpt-4.1`) across two prompt/temperature variants (`clinical_plausibility_v1` @ 0.2, `falsification_v1` @ 0.8), each requiring cited evidence. **44 of 48 retained, 4 rejected.** Ranking (#882) combined novelty, grounded confidence, cross-domain distance, and evaluator plausibility, flagging 10 for human review. Crucially, source expansion (#1171) resolved every candidate CxId back to physical source rows (2,612/2,612 verified), revealing that the top cluster is a **known asthma-pharmacology teaching signal** (salbutamol/terbutaline, salmeterol/formoterol) — useful for calibration, not a discovery. Concept normalization (#1172) mapped 3,347 candidate terms via PubTator3 (79 normalized, 3,268 explicitly unresolved), and the typed overlay (#1173) joined everything into **7,928 nodes and 116,753 typed edges** with zero untyped/invalid edges.

---

## 5. The honesty architecture: fail-closed trust gates

The single most important methodological contribution is a set of hardening tasks (jointly tracked as trust-integrity issue #1214) that make the pipeline structurally unable to over-claim. These are not features; they are *guarantees*.

### 5.1 Sufficiency, not proximity (oracle CI-low gate #1204, discovery-chain gate #1205)

Before hardening, sufficiency was decided from a mutual-information *point estimate*; a panel could pass when its point estimate cleared the anchor entropy even if the *lower bound* did not. The oracle honesty gate now requires **`ci_low ≥ anchor_entropy_bits` and `PowerCalibrationStatus::Passed`** — the same lower bound used for the decision is the bound the oracle exposes. Separately (#1205), the discovery chain no longer accepts hops on topological anchor-reachability alone; it fails closed with `CALYX_DISCOVERY_NO_SUFFICIENCY_ASSAY` unless a power-calibrated sufficiency instrument is present. Topology can *rank and explain*; it can no longer *assert grounded acceptance*.

### 5.2 Honest mutual-information estimation (KSG mixed estimator #1207, subsample CI #1208)

Discrete anchors previously were one-hot-encoded and fed into a continuous KSG estimator, inflating information. The estimator now uses the **Ross mixed continuous–discrete** method (same-label kth continuous radius + class-size digamma correction), failing closed with `CALYX_ASSAY_INSUFFICIENT_SAMPLES` when any class size ≤ k. On the UCI Iris control, the old one-hot path reported a point estimate *above* the 3-class entropy while the Ross lower bound did *not* clear it — exactly the over-optimism the fix removes. The confidence interval was likewise changed from with-replacement bootstrap (whose duplicate rows KSG reads as fine-scale structure) to deterministic **m-out-of-n no-replacement subsampling** (Holmes–Nemenman), narrowing a spurious independent-control interval by ~9×.

### 5.3 Relation-specific, not substring, matching (#1206)

The falsification sweep originally attached support/counter evidence on whole-row substring co-mention. It now requires **both hypothesis endpoints to appear in the same structured asserted-relation fields** (e.g., PubTator `left/right`, ClinicalTrials `intervention/condition`, DGIdb `drug/gene`, Open Targets `target/disease`). This single change collapsed the #1184 evidence from 59 support + 1 counter (co-mention) to **8 support + 0 counter** (relation-backed), removing a false counter for PLA2R1/Kidney Diseases whose Open Targets row asserted a *different* disease.

### 5.4 Calibrated novelty (#1209) and weighted evidence (#1213)

Blind-spot alerts moved from a global absolute delta to per-lens-pair empirical `p_value ≤ alpha` (min 50 samples, alpha 0.05), skipping uncalibrated pairs with `CALYX_LOOM_UNCALIBRATED_BLINDSPOT`. The CSR projection was fixed (#1213) so edge evidence weights are preserved rather than rebuilt as uniform 1.0.

### 5.5 Provenance at ingest (#1211) and binary CSR (#1210)

Streaming batch ingest now rejects any row lacking `source_dataset`, `source_sha256`, `license`, `retrieval_ts`, and at least one locator (`source_url`/`doi`/`pmid`/`pmcid`) — a bulk biomedical row cannot become a graph node without traceable provenance. The persisted CSR was rewritten to a binary columnar layout (magic `CALYXCSR`, raw 16-byte CxIds, f32 weights, dictionary-encoded edge types), shrinking the real graph CSR from ~157 MB JSON to 63 MB (0.40×) and failing closed on any corruption.

### 5.6 Graph-collection lifecycle (#1197)

Materializers write a `writing` generation state before graph writes and `accepted` only after physical readback; default readers fail closed (`CALYX_GRAPH_COLLECTION_NOT_ACCEPTED`) if a collection has lifecycle rows but no accepted generation. Aborted collections are tombstoned rather than silently reused.

**Net effect.** Every downstream biomedical claim in this paper inherits a chain of: source-provenance at ingest → typed evidence → relation-specific matching → calibrated novelty → lower-bound sufficiency → power calibration → lifecycle acceptance. A claim that cannot satisfy the chain is *blocked*, and the block is recorded with a named reason code.

---

## 6. Results I — substrate, graph, and kernel

| Quantity | Value |
|---|---:|
| Clinical-QA rows ingested | 199,000 |
| Anchored constellations (after 7-row dedup) | **198,993** |
| Ledger entries, `verify-chain` | 647,374 (`status: ok`, no break) |
| Association graph nodes | 198,993 |
| Association graph directed edges | ~2,435,817 |
| XTerm cross-lens agreement keys | 13,133,538 (= 198,993 × 66) |
| Kernel members | 21,954 |
| Kernel groundedness fraction | 1.0 |
| Kernel A10 recall ratio | 1.0 (995 held-out queries) |
| τ* estimate | 958 |
| Binary CSR size (real vault) | 63,235,522 bytes (0.40× JSON) |

This substrate is the foundation for every result that follows. It is durable, chain-verified, and read back from source-of-truth bytes — but it is a **clinical-QA** corpus (PubMedQA/MedXpertQA/MedQA/MedMCQA). That composition is decisive for interpreting the discovery outputs (§13): the corpus encodes *what medical exams teach*, so recovering exam-canonical associations is calibration, and genuinely novel clinical discovery would require corpora the corpus does not yet contain.

---

## 7. Results II — discovery surfaces and the first hypothesis atlas

The discovery surfaces behaved as designed and, importantly, **recovered known structure rather than inventing new claims** — the expected signature of a well-calibrated system on an exam corpus.

- The **48 chain-walk A–B–C hypotheses** converged, on source-text expansion, onto asthma pharmacology (β-agonists, leukotriene modifiers, steroids, methylxanthines). The LLM evaluator called them "mostly coherent asthma pharmacology associations grounded in MedMCQA rows" and rejected 4 for repeated endpoints.
- The **top typed co-mention pairs** were dominated by asthma teaching clusters: zafirlukast↔montelukast (28 support CxIds), ipratropium↔theophylline (28), ipratropium↔steroids (25), tiotropium↔ipratropium (18), prednisolone↔steroids (18). These are textbook adjacencies — exactly the calibration/known-positive signal a trustworthy pipeline should surface first.
- The **association result pack (#1170)** consolidated 1,949 machine-readable rows across ten discovery surfaces with 0 missing required source references.

The honest reading of this stage: the machinery works end-to-end, is fully traceable, and — on this corpus — produces calibration signal, not discovery. That is a *success of calibration*, not a therapeutic finding.

---

## 8. Results III — normalization, typing, and external validation

To move beyond opaque CxIds, the program built a normalization → typing → external-validation stack.

| Stage | Issue | Key output |
|---|---|---|
| Concept normalization | #1172 | 3,347 candidate terms → 79 normalized (37 chemical, 33 disease, 8 gene, 1 variant), 3,268 explicitly unresolved |
| Typed overlay graph | #1173 | 7,928 nodes, 116,753 typed edges, 0 untyped |
| Open Targets validation | #1174 | release 26.06; 1,422 association rows, 1,420 validation edges, 0 API errors |
| PubTator/PubMed relations | #1176 | supporting + contradicting literature rows |
| ClinicalTrials.gov | #1177 | intervention–condition trial rows |
| DGIdb drug–gene | #1178 | druggable-gene interaction rows |
| CIViC precision oncology | #1180 | 4,870 clinical-evidence rows; CC0; OncoKB/DepMap recorded as license-restricted, *not* ingested |
| LINCS/CMap reversal | #1179 | 30 CREEDS signatures, 1,500 reversal-score rows |
| Evidence substrate in Calyx DB | #1196 | `biomed_evidence_substrate_v3`: 10,092 nodes / 21,496 edges, physically read back |
| Association validation gates | #1182 | AUROC 1.0 on known positives; time-split AUROC 0.864 |

Two findings deserve emphasis.

**The LINCS reversal screen is an honest negative.** None of the current Calyx candidate drugs (metformin, the gliptins, the TNF biologics) appeared in the top-50 reverse L1000CDS2 results for the 30 selected disease signatures. The repeated reversal *leads* were mechanism-class compounds — HDAC inhibitors (vorinostat, trichostatin A), CDK inhibitors (CGP-60474, alvocidib), HSP90 (geldanamycin), MEK (PD-0325901, selumetinib) — reported as lead signals only. A data-quality trap (the `-666` placeholder perturbation label spanning 148 distinct BRD IDs) was caught and resolved (#1199) by mapping BRD IDs to authoritative L1000FWD metadata, preventing a placeholder from being read as a single compound.

**License discipline was enforced.** CIViC (CC0) was ingested; OncoKB (which forbids training AI/ML models) and Sanger DepMap/GDSC (commercial/API restrictions) were *recorded as constrained and not ingested*. This is the doctrine's "missing data creates a task, not a silent skip" rule in action.

The validation gate (#1182) is the acceptance instrument that downstream miners must pass. Its strict-threshold failure (known-positive recall 0.544 at the default 0.5 threshold) was *preserved as calibration evidence*, and the accepted gate used a source-inclusive 0.05 threshold reflecting the lowest admitted external score.

---

## 9. Results IV — the five disease-area deep hunts

Five bounded disease-area hunts composed the typed miner, falsification flags, external validation, and live ClinicalTrials/openFDA probes. **Every top-ranked row was association or target-context, never a drug-intervention claim, and every drug-bearing row carried safety/trial review flags.**

### 9.1 Metabolic / cardiovascular / renal (#1186) — 35 rows

The highest-ranked rows were target–disease context, not interventions:

| Rank | Type | Target/Drug | Disease | Score | Status |
|---:|---|---|---|---:|---|
| 1 | target–disease | **TNF** | Proteinuria | 6.80 | target context only |
| 2 | target–disease | **CD4** | Proteinuria | 6.51 | target context only |
| 3 | target–disease | **CD8A** | Proteinuria | 5.40 | target context only |
| 4 | drug–disease | Steroids | Proteinuria | 4.94 | label gap blocks promotion |
| 5 | drug–disease | Nifedipine | Hypertension | 4.80 | safety/trial review required |
| 6 | drug–disease | Verapamil | Hypertension | 4.80 | safety/trial review required |

The DPP4/gliptin rows (alogliptin, linagliptin, saxagliptin, sitagliptin, vildagliptin) were carried explicitly as **known proof-slice context**, not new repurposing claims. Fail-closed safety gaps (missing labels for Leukotrienes, Steroids, Vildagliptin, zopiclone) were recorded, not hidden.

### 9.2 Neurodegeneration / neuropsychiatric (#1187, repaired to 131 in #1222) — 77 rows

Top global ranks were dominated by Open Targets neurogenetic/microcephaly signals (NF1→neurofibromatosis 0.953; NF1→NF-Noonan 0.919; KIF11/PCNT/WDR62/MCPH1/CDK5RAP2/NDE1→microcephaly). The notable *drug* research leads were **DPP4-inhibitor and metformin bridges to schizophrenia** — saxagliptin (rank 23) and metformin (rank 34, Open Targets DPP4–schizophrenia score 0.192) — surfaced as *weak-to-moderate* bridge leads with completed live safety/trial triage, explicitly not efficacy. A subsequent druggability expansion (#1224) turned the bounded DPP4 slice into **1,494 drug-target-disease bridge candidates** across DRD2/3/4, HTR1A/2A/4, opioid receptors, KIF11, PTEN, MTHFR, NF1 — but these are mapping leads, not outcome evidence.

### 9.3 Infectious / immunology / inflammation (#1188) — 209 rows

The top ranks recovered *established* immunology, i.e., strong calibration:

| Rank | Family | Drug | Target | Disease | Score |
|---:|---|---|---|---|---:|
| 1 | host-target | Golimumab | TNF | psoriatic arthritis | 1.40 |
| 2 | host-target | Certolizumab Pegol | TNF | psoriatic arthritis | 1.40 |
| 3 | antiviral | Tregalizumab | CD4 | HIV | 1.28 |
| 7 | host-target | Infliximab | IL12B | psoriasis | 1.24 |
| 15 | antiviral | Ibalizumab | CD4 | HIV | 1.18 |

TNF/psoriatic-arthritis and CD4/HIV bridges validate that the pipeline recovers established biomedical structure — and *crowd out* novelty, which motivated the novelty/calibration split (§10.2). Antimicrobial rows (Streptomycin/Klebsiella, Streptomycin/Rhinoscleroma) appeared lower.

### 9.4 Rare disease (#1189) — 1,491 rows

Composing HPO/HPOA (284,871 annotation rows; 12,956 disease profiles; 12,654 disease–gene phenotype links) and Mondo with the Calyx substrate produced the largest hunt. Top phenotype/gene/drug leads (all `not_run_for_hpo_generated` at emit, i.e., *not yet falsified*):

| Rank | Disease | Gene | Drug | Score |
|---:|---|---|---|---:|
| 1 | Cerebral arteriopathy (CADASIL-type) | **NOTCH3** | Tarextumab | 13.83 |
| 2–5 | Cardiofaciocutaneous syndrome 1 | **BRAF** | Dabrafenib / Cetuximab / Trametinib / Panitumumab | 13.51 |
| 6 | Melnick–Needles syndrome | **FLNA** | Simufilam | 13.47 |
| 7 | Fanconi anemia | **BRCA2** | Olaparib | 13.37 |
| 8 | Meningioma | **PIK3CA** | Alpelisib | 13.37 |

These are gene→phenotype-plus-drug-target mappings — strong on phenotype/gene support but with only 29 of 1,491 rows carrying same-disease Open Targets context and **zero** having passed cross-domain falsification at emit. They are explicitly leads for review, not evidence a listed drug treats a listed disease.

### 9.5 Oncology (#1185) — 19 rows

Composing CIViC (#1180), the typed miner (#1183), and falsification (#1184):

| Candidate | Cancer | Gene | Therapy | CIViC level | Falsification |
|---|---|---|---|---|---|
| civic:11176 | Plexiform Neurofibroma | NF1 | **Selumetinib** | A Predictive Sensitivity | no counter found |
| civic:10138 | Breast Cancer | IGF1R | Metformin; Exemestane | B Predictive | not in typed surface |
| civic:7487 | Childhood Low-grade Glioma | NF1 | Selumetinib | B Predictive | no counter found |
| civic:1053/1054 | Meningioma | PTTG1 / LEPR | (prognostic) | B Prognostic | no counter found |
| civic:1470 | Skin Melanoma | NF1 | Vemurafenib | C Predictive | no counter found |
| civic:1230 | Cancer | NRAS | Metformin; Trametinib | D Predictive | not in typed surface |

The strongest oncology rows are *known/calibration* evidence (NF1/selumetinib for plexiform neurofibroma is an established, trial-backed pairing). All drug-bearing oncology rows remained safety/outcome/review blocked.

### 9.6 Drug-safety triage (#1181)

Every drug-bearing oncology candidate received `clinical_promotion_block_until_safety_review_complete`. openFDA labels and FAERS totals were persisted for all 14 therapy terms — e.g., FAERS report counts (a raw report field, *not* incidence): Metformin 425,794; Prednisolone 193,614; Doxorubicin 107,484; Cytarabine 62,318; Gemcitabine 56,395; Sirolimus 13,671; Exemestane 12,644; Trametinib 6,718; Vemurafenib 4,046; Selumetinib 539; Mirdametinib 11. Three research compounds (AZ628, JQ1, VTX-11e) had **no** label or FAERS coverage and were **fail-closed blocked**, not treated as safe.

---

## 10. Results V — falsification, safety, and the human-review atlas

### 10.1 The all-pair miner and the first falsification sweep (#1183, #1184)

The typed miner deduplicated typed `associated_with` edges into concept-pair hypotheses (267 candidate pairs, 250 emitted broad; 42 chemical/disease; 9 gene/disease), requiring a passing #1182 validation report. The falsification sweep (#1184) processed 280 deduped hypotheses; under the later relation-specific gate (#1206) it retained **8 relation-backed support rows and 0 counters** — the honest, conservative count.

### 10.2 Novelty / calibration split (#1226, #1227)

Because known-positive immunology rows crowded the top-k, a splitter routed the 340 combined disease-hunt rows into **169 calibration/known-positive** rows (each carrying an explicit marker such as Open Targets clinical precedence ≥ 0.75 or CIViC level A/B) and **171 novelty-prioritized** research leads. After the split, the novelty top rows are the TNF/CD4/CD8A→proteinuria target-disease leads and the gliptin/schizophrenia bridges — provisional, not calibration. Nothing is hidden: calibration rows remain in a separate proof view. The splitter was productionized as a native Calyx CLI stage (#1227) that fails closed on a stale input manifest.

### 10.3 Generated-candidate falsification sweep (#1223)

This is the promotion gate for the disease hunts. Across 1,885 generated candidates (1,877 deduped):

| Outcome | Count |
|---|---:|
| Support evidence rows | 4,926 |
| Counter-evidence rows | 2,151 |
| **Blocked (missing required evidence/safety)** | **1,066** |
| **Demoted (hard counter-evidence)** | **16** |
| No counter-evidence in bounded sources (still not a pass) | 795 |

The dominant block reasons were `trial_source_missing_for_drug_disease` (1,028) and `safety_source_missing_fail_closed` (991) — actionable data-ingest deficits, deliberately fail-closed. A normalization fix stopped endpoint labels (drug names, disease labels) from being mis-promoted to gene symbols, dropping the DGIdb exact-pair-missing reason to 14 rows.

### 10.4 The human-review hypothesis atlas (#1193)

The atlas consolidates all five hunts with novelty/calibration state, falsification state, support/counter overlays, safety/trial flags, and source hashes:

| Review status | Count |
|---|---:|
| **Blocked or demoted before human review** | **1,082** |
| **Ready for hypothesis review** | **726** |
| **Calibration / known-positive reference** | **69** |

| Disease area | Rows |
|---|---:|
| Rare disease | 1,491 |
| Infectious/immunology/inflammation | 203 |
| Neurodegeneration/neuropsychiatric | 129 |
| Metabolic/cardiovascular/renal | 35 |
| Oncology | 19 |

Every one of the 1,877 rows is hypothesis-only, carries a clinical boundary and a "next validation experiment" field, and none is missing a falsification flag. The top ready-for-review rows are the TNF/CD4/CD8A→Proteinuria target–disease leads and two neuro disease-cluster rows — each with the recommended next step "validate the target–disease association in an outcome-backed assay/model before drug inference." The atlas was materialized into a native Calyx vault (`01KWPJR0ADVZF580HNRBZ17CBZ`), so the review surface is itself in the database.

---

## 11. Results VI — drug combinations and the external evidence cascade

### 11.1 The combination miner (#1190)

The miner extracted drug-bearing components from the atlas, paired the top components per disease context, and blocked promotion unless *component safety, pair interaction, and external synergy/model evidence* were all present. Result: **1,750 candidate pairs, all 1,750 blocked, zero promoted, zero reviewable-preclinical rows.**

| Block reason | Count |
|---|---:|
| component blocked/demoted before combination | 1,750 |
| component safety missing (fail-closed) | 1,727 |
| pair interaction evidence missing (fail-closed) | 1,745 |
| external synergy evidence missing (fail-closed) | 1,703 |
| overlapping component safety flags (review required) | 17 |

DrugComb v1.4 (MD5-verified) added *preclinical* evidence: 28 exact pair keys matched (47 rows). The top blocked rows name exactly what is missing: Metformin+Sitagliptin, Metformin+Saxagliptin, Metformin+Alogliptin, Metformin+Linagliptin (all Type 2 Diabetes), Sunitinib+Everolimus (pheochromocytoma-paraganglioma). The recorded diagnosis: the safety substrate is *too narrow* (14 component safety rows for 1,023 components) for broad combination promotion — a data deficit, not a clinical conclusion.

### 11.2 The external evidence cascade (#1229 → #1234, and beyond)

To close the external-synergy gap, a cascade of open sources was ingested, each rechecking the residual no-hit set and each leaving all rows blocked:

| Stage | Source | External no-hit set after stage |
|---|---|---:|
| #1190 | DrugComb v1.4 | (1,703 synergy-missing) |
| #1229 | NCI ALMANAC | 1,682 |
| #1231 | CDCDB (43,082 combos; ClinicalTrials/OrangeBook/patents) | 1,546 |
| #1232 | ClinicalTrials.gov v2 current recheck (1,618 API pages) | 1,342 |
| #1234 | FDA Orange Book (0) + NDC (0) + PubMed (301) | 1,041 |

The cascade added source-attributed *documentation* (registry co-occurrence, patent records, literature co-mention) but the doctrine kept `external_synergy_evidence_missing_fail_closed` on documentation-only rows: CDCDB/registry/patent presence is *not* a synergy, safety, or outcome gate. Subsequent PubMed source-text validation (#1237) fetched EFetch XML for 523 PMIDs and classified 568 evidence rows into conservative relation classes (17 asserted-combination, 182 asserted-interaction, 87 asserted-outcome, 224 counter-evidence, 49 insufficient-text) — still blocked pending structured extraction, safety, outcome, and human review.

### 11.3 The terminal effect-result and safety validations

- **Effect-result falsification gate (#1252).** From 35 Europe PMC source-local endpoint rollups: 9 had direction+magnitude language, 15 direction-only; **every row remained blocked** because source-window text is not an independent endpoint-result gate.
- **Independent effect-result validation (#1253).** The 17 unique candidate pairs from #1252 were queried against Europe PMC, ClinicalTrials.gov, and PubMed, requiring source-local pair evidence *outside* the original windows. **Zero independent evidence rows survived.** All 24 rollups: `no_independent_endpoint_result_source_hit_still_blocked`.
- **FAERS safety expansion (#1249).** Of 363 rollups (202 unique pairs), FAERS co-reports covered 82 rollups, 79 tied to serious events — blockers/review inputs, not clearance.
- **RxNorm/TwoSIDES safety rescue (#1258/#1259).** RxNorm canonicalization rescued **757 TwoSIDES adverse-effect rows across 7 pair keys**. Independent validation of those 7 keys surfaced exactly **one** independent FAERS safety signal — for `metformin || trametinib dimethyl sulfoxide` (serious co-report, reactions "Off label use; Lower gastrointestinal haemorrhage," max PRR 40). The other six keys had TwoSIDES rows but no independent confirmation. All seven remained blocked.

---

## 12. Results VII — the metformin + trametinib safety case study

The single concrete serious safety signal produced by the entire program deserves a dedicated case study, because its resolution is the clearest illustration of the fail-closed discipline working as intended.

**The signal (#1259).** An independent openFDA FAERS co-report for metformin + trametinib, source ID `24608768`, serious, with reactions "Off label use" and "Lower gastrointestinal haemorrhage."

**Case-level validation (#1260).** The exact case was re-fetched (`safetyreportid:24608768`) and interpreted against RxNorm identity, openFDA labels, DailyMed SPL, Europe PMC, and PubMed. The findings:

- The report contains **19 drugs**.
- Both pair drugs (metformin, trametinib DMSO) are present but reported as **concomitant**, not primary suspect.
- The **primary suspect drug is Eliquis (apixaban)** — an anticoagulant — appearing twice.
- Classification: **`serious_faers_case_confounded_still_blocked`**, with reason codes `eliquis_primary_suspect_anticoagulant_confounder_present`, `trametinib_metformin_concomitant_not_primary_suspect`, and `polypharmacy_case_report_not_pair_causality`.

The lower-gastrointestinal-hemorrhage reaction is the expected adverse profile of the anticoagulant, not evidence of a metformin–trametinib interaction. **The one hard safety signal the program surfaced dissolved under case-level scrutiny into a confounded polypharmacy report.**

**Ranker feedback (#1261).** Rather than discard the finding, the confounded case was fed back as a **case-quality feature**: a deterministic 0.45 rank penalty applied to 5 metformin/trametinib-family combination rows (across Cardiofaciocutaneous syndrome, Noonan syndrome, and Cancer contexts), all still blocked. This closes the loop — the system learns to *down-weight* co-presence-only serious reports rather than treat them as pair causality.

**Coverage accounting (#1228).** The terminal rollup accounted for every #1190 component and pair: 277 unique component drugs (164 with safety context, 113 explicit no-hit), 1,750 candidate pairs (1,234 with source context, 516 explicit no-hit) — **all blocked**, every gap fail-closed with a reason code, materialized into Calyx.

---

## 13. Discussion — what this means and what it does not

### 13.1 The honest headline

Across the entire program: **no validated cure, treatment, dosing guidance, or clinical recommendation.** 1,750 drug-combination hypotheses, 0 promoted. 24 terminal effect-result candidates, 0 independent confirmations. 1 serious safety signal, confounded on inspection. This is a null clinical result, and we report it as such.

### 13.2 Why a null result is the *expected* outcome — and still valuable

Three structural facts make the null result unsurprising and, we argue, appropriate:

1. **Corpus composition.** The discovery graph was built from clinical-QA exam corpora. Exams encode *consensus teaching*, so the strongest recoverable associations are textbook-canonical (asthma pharmacology, TNF/psoriatic-arthritis, CD4/HIV, NF1/selumetinib). Recovering these is *calibration* — proof the pipeline finds real biology — not discovery. Genuine novelty would require corpora (full-text literature, molecular assays, transcriptomics at scale) that were only partially materialized.
2. **The gates are doing their job.** The clinical-actionability bar requires outcome-backed sufficiency, breadth of counter-evidence, safety adjudication, and human review. On bounded open sources, most candidates *cannot* clear that bar — and the system correctly *blocks* rather than fabricates. A pipeline that promoted leads on this evidence would be *wrong*, not impressive.
3. **Fail-closed by design.** The 1,082 blocked atlas rows and 1,750 blocked combinations are not failures to find signal; they are the recorded *reasons* signal is insufficient — a work queue of specific, named data deficits.

The usable result is therefore the **capability and the discipline**: Calyx can rank, separate, falsify, and block biomedical association leads inside a provenance-sealed workflow where every claim is traceable to source bytes and gated on a calibrated lower bound. That is a reusable instrument. On a richer corpus, the same instrument would surface — and equally rigorously gate — genuinely novel leads.

### 13.3 What the leads are worth

The surfaced leads fall into three honest tiers:

- **Calibration / known-positive (69 atlas rows).** NF1/selumetinib, TNF-inhibitors/psoriatic arthritis, CD4/HIV, DPP4-inhibitors/T2D. Value: gate-health references, not novelty.
- **Provisional research leads (726 pre-blindspot ready-for-review rows).** TNF/CD4/CD8A→proteinuria; DPP4/metformin→schizophrenia bridges; the rare-disease gene/drug mappings (NOTCH3/tarextumab, BRAF/dabrafenib for CFC syndrome, BRCA2/olaparib for Fanconi anemia, PIK3CA/alpelisib for meningioma, FLNA/simufilam). Value: prioritized hypotheses for a human expert to inspect and, if warranted, design an outcome-backed assay around. Before any row is treated as ready after the 2026-07-07 hardening pass, it should clear `calyx biomedical-blindspot-audit` against external literature, stability, drug-lifecycle, patient-context, and transcriptomic-specificity sources. **None is evidence that the drug treats the disease.**
- **Blocked (1,082 rows + 1,750 combinations).** Value: a precise, reason-coded manifest of exactly what evidence is missing.

### 13.4 How this changes the workflow

The program demonstrates a shift from "generate associations and hope" to "generate, then structurally prevent over-claiming." The doctrine that *association/admission truth lives in the database, and JSON/docs are reporting surfaces only* means the atlas, the combination worklist, and the safety rollup are all queryable Calyx collections with physical readback — not disposable reports. A reviewer works against source-of-truth rows, not a chat transcript.

---

## 14. Limitations and threats to validity

We record the limitations the program itself flagged (trust-integrity issue #1214 and elsewhere):

1. **Corpus scope.** The graph is clinical-QA only. Non-clinical/molecular corpora were proven at slice scale (4-row and 53-row molecular vaults, a 1,000-row bridge slice) but not ingested at full BindingDB/ChEMBL scale. Clinical × molecular/legal/finance bridge acceptance is therefore *not* proven.
2. **Normalization coverage.** First-pass PubTator3 normalization resolved 79 of 3,347 terms; 3,268 remained unresolved (largely bounded-API budget and transient 502s). Repairs (#1222/#1225) improved specific domains but the overlay is small relative to sources (e.g., 15 of 4,870 CIViC rows mapped).
3. **Bounded external queries.** ClinicalTrials/openFDA/PubMed/Europe PMC passes were bounded (page limits, no-key rate limits). "No hit in bounded query" is not "no evidence exists."
4. **Co-mention ≠ relation.** Even after the #1206 relation-specific gate, many overlay edges remain co-mention candidates until a source parser exposes a structured relation endpoint pair.
5. **Time-split benchmark is small.** The #1182 gate's time-split used 11 later-positive and 2 later-negative rows — useful but underpowered.
6. **The trust items must stay fixed.** #1214 recorded that CI-lower-bound handling, sufficiency-vs-reachability, relation-specific matching, and provenance-fail-closed ingest are the load-bearing guarantees. Until all are verified read-back from Calyx, outputs must stay hypothesis-only. (This program implemented all four; the caveat is that regressions would invalidate downstream trust.)
7. **Storage lifecycle debt.** Several aborted graph collections (from row-by-row write and full-CSR-scan anti-patterns) were tombstoned; native atomic collection replacement is outstanding.

None of these limitations is hidden; each corresponds to a filed follow-up task.

---

## 15. What the substrate now makes possible

The program's own forward-looking analysis, grounded in public resources, identifies realistic deliverables once the current graph is normalized, typed, validated, and fed richer corpora:

- **Ranked LBD over A–B–C paths** with aggressive filtering (never unbounded B-term enumeration).
- **Drug repurposing by explainable graph-path features**, scored against known treatment edges (Hetionet/Rephetio style).
- **Target–disease prioritization** triangulated against Open Targets evidence categories.
- **Molecular bridge expansion** mapping clinical terms to BindingDB/ChEMBL/DGIdb drug-target-assay rows.
- **Transcriptomic reversal screens** treating negative correlation as a lead, subject to reproducibility limits.
- **Cancer-specific hypothesis generation** with explicit evidence-level and safety gates (CIViC-backed).
- **Trial and safety triage** reading ClinicalTrials.gov status and FAERS/label evidence before ranking anything as actionable.
- **Falsification-first review**: every retained hypothesis carries counter-evidence queries, contradictory literature, trial-failure and toxicity flags, and known-mechanism-conflict checks.

The infrastructure to do each of these now exists as verified Calyx capabilities; what remains is corpus breadth and human-in-the-loop review.

---

## 16. Future work

Concrete next steps, in dependency order, follow directly from the limitations:

1. **Ingest full-text and molecular corpora at scale** (BindingDB/ChEMBL beyond slice, PubTator full relation set) so the graph encodes more than exam consensus.
2. **Expand concept normalization** with improved NER, source-specific ontology lookup, and larger API budgets to shrink the 3,268-term unresolved queue.
3. **Grow the safety substrate** past 14 component rows so combination promotion is not starved (the dominant #1190/#1228 blocker).
4. **Broaden and strengthen the time-split validation benchmark** so the acceptance gate rests on more than 13 rows.
5. **Rerun the 726 pre-blindspot ready-for-review atlas rows through the biomedical blindspot audit, then human expert review the survivors**, defining outcome-backed instruments for the highest-priority target–disease leads (TNF/proteinuria class; rare-disease gene/drug mappings).
6. **Native Calyx collection lifecycle cleanup** (atomic replacement) to retire tombstoned collections.
7. **Revisit GPU acceleration** only if a larger measured workload produces a dominant sparse-matvec/PPR bottleneck, verified against the CPU output hash.

---

## 17. Conclusion

We built and ran a fail-closed biomedical association-discovery engine over a 198,993-constellation clinical-QA corpus, executing a complete pipeline from anchored ingest through a directed 2.44M-edge association graph, an MFVS kernel, five disease-area deep hunts, a falsification-aware human-review atlas of 1,877 hypotheses, and a fully gated drug-combination worklist of 1,750 pairs, all cross-checked against a dozen external biomedical databases and materialized into a provenance-sealed database.

The clinical yield was null: **no cure, treatment, dose, or recommendation cleared the actionability bar.** The one serious safety signal proved confounded. We regard this not as a disappointing outcome but as the *correct* behavior of a system engineered so that the absence of evidence is a recorded block rather than an unsupported promotion. The durable contributions are the substrate, the calibrated fail-closed gate architecture, and the honest, traceable, falsification-aware atlas — an instrument that ranks, separates, falsifies, and blocks biomedical leads with every claim reducible to source bytes and a lower-bound sufficiency proof. On a richer corpus, the same instrument is positioned to surface genuinely novel leads and to gate them just as rigorously.

A discovered association here is, and remains, a **ranked, traceable hypothesis — never a verdict.**

---

## Appendix A — glossary

| Term | Meaning |
|---|---|
| **Constellation** | A record measured through the panel; the graph's node unit. |
| **Panel / lens / slot** | The sensor array (panel) of embedders/encoders (lenses) producing per-lens vectors (slots). |
| **Anchor** | A typed, provenance-carrying label that grounds a constellation (e.g., `label:answer`). |
| **Grounded** | A node that reaches an anchor within a bounded distance. |
| **Loom weave / XTerm** | Within-document cross-lens agreement cross-terms; the constellation signature. |
| **AssocGraph** | The between-document directed association graph (Graph CF / PlainGraph). |
| **Kernel (MFVS/DFVS)** | A compact grounded subgraph selected by SCC → betweenness → top-fraction → minimum feedback vertex set. |
| **A10 recall gate** | The requirement that the kernel reproduce full-index top-k nearest neighbors over held-out queries. |
| **Sufficiency gate** | `I(panel; outcome) ≥ H(outcome)` measured at the calibrated *lower bound* with passing power calibration. |
| **KSG / Ross / Holmes–Nemenman** | Mutual-information estimator; its mixed discrete–continuous correction; its no-replacement subsample CI. |
| **FSV** | Full-state verification: re-reading persisted source-of-truth bytes and checksumming, not trusting return values. |
| **Fail-closed** | Missing/insufficient evidence produces a named error/block, never a silent pass. |
| **Bridge corpus** | A materialized Calyx vault slice used to place a report's rows into the database itself. |

---

## Appendix B — the claim ladder

The doctrine ranks biomedical outputs on a ladder; association-only inference never reaches the top:

1. **Association candidate** — a co-mention or graph adjacency; the default state of every generated row.
2. **Typed association** — endpoints normalized to biomedical concepts, edge typed (drug–target, target–disease, etc.).
3. **Externally contextualized** — corroborating rows from Open Targets/DGIdb/CIViC/ClinicalTrials, *type-applicable*.
4. **Falsification-swept** — support and counter-evidence gathered under relation-specific matching.
5. **Safety-triaged** — component labels/FAERS gathered; missing coverage is a block.
6. **Sufficiency-proven** — a calibrated lower-bound outcome instrument clears anchor entropy.
7. **Human-reviewed** — an expert has defined and, ideally, run an outcome-backed experiment.
8. **Clinically actionable** — *not reached by any row in this program.*

Every atlas and combination row in this paper sits at rung 3–5. None reaches rung 6+.

---

## Appendix C — artifact and provenance index

Selected source-of-truth artifacts (all SHA-256/BLAKE3-checksummed in the findings log). The canonical vault for the clinical association graph is `corpus-anchored-869-20260625T080546Z` (ULID `01KVYX0KYVBQSGVC6N2S00FX6J`).

| Surface | Issue | Physical evidence (representative) | Key readback |
|---|---|---|---|
| Anchored corpus | #869 | vault `01KVYX0KYVBQSGVC6N2S00FX6J` | 198,993 cx; chain ok (647,374 entries) |
| Loom weave | #870 | XTerm CF (9.9 GB pre-compaction); Graph CF | 13,133,538 XTerm keys; 2,435,817 edges |
| Kernel | #871 | `idx/kernel/.../kernel.json` (2.1 MB) | 21,954 members; recall 1.0; τ* 958 |
| Blind spot | #875 | `idx/blind_spot/.../blind_spot_sweep.json` | 128 candidates from 10,064,934 obs. |
| Spectral | #877 | `idx/spectral_communities/.../report.json` (42.8 MB) | 2 communities; gap 0.943 |
| Discovery chain | #878 | `idx/discovery_chains/.../chain.json` (36.5 MB) | 25,472 cand.; 21,936 gate pass; 100 hops |
| Chain walks | #880 | `.../real_chain_walks.json` | 6 seeds; 48 A–B–C hypotheses |
| Hypothesis eval | #881 | `.../hypothesis_evaluation_report.json` | 48 eval; 44 retained (GPT-4.1) |
| Ranked | #882 | `.../ranked_hypotheses_report.json` | 44 ranked; 10 human-review |
| Source expansion | #1171 | `.../complete_cxid_source_expansion.jsonl` | 2,612/2,612 verified; 0 unresolved |
| Concept normalization | #1172 | `.../normalized_concept_annotations.jsonl` | 3,347 terms → 79 normalized |
| Typed overlay | #1173 | `.../typed_edges.jsonl` | 7,928 nodes; 116,753 typed edges |
| Open Targets | #1174 | `.../` (release 26.06) | 1,422 rows; 1,420 validation edges |
| Evidence substrate (in DB) | #1196 | collection `biomed_evidence_substrate_v3` | 10,092 nodes / 21,496 edges, read back |
| LINCS reversal | #1179 | collection `biomed_lincs_cmap_reversal_v7` | 1,500 reversal rows; candidates absent from top-50 |
| Validation gate | #1182 | `.../association_validation_report.json` | AUROC 1.0; time-split AUROC 0.864 |
| All-pair miner | #1183 | `.../typed_association_miner_report.json` | 267 pairs; 250 emitted (broad) |
| Falsification sweep | #1184/#1206 | `.../falsification_sweep_report.json` | 280 deduped; 8 support / 0 counter (relation-backed) |
| Oncology hunt | #1185 | `.../oncology_hypothesis_atlas.jsonl` | 19 candidates |
| Metabolic hunt | #1186 | `.../metabolic_cardiovascular_hypotheses.jsonl` | 35 rows |
| Neuro hunt | #1187/#1222 | `.../neuro_hypotheses.jsonl` | 77 → 131 rows |
| Infectious hunt | #1188 | `.../infectious_immunology_hypotheses.jsonl` | 209 rows |
| Rare-disease hunt | #1189 | `.../rare_disease_hypotheses.jsonl` | 1,491 rows |
| Drug safety | #1181 | `.../candidate_safety_flags.jsonl` | 14 therapies; all drug-bearing blocked |
| Novelty/calibration split | #1226/#1227 | `.../novelty_prioritized_research_leads.jsonl` | 340 → 169 calibration / 171 novelty |
| Generated falsification | #1223 | `.../candidate_falsification_flags.jsonl` | 1,877 flags; 1,082 blocked; 16 demoted |
| Human-review atlas | #1193 | `.../human_review_...atlas.jsonl` (vault `01KWPJR0ADVZF580HNRBZ17CBZ`) | 1,877 rows; 1,082 blocked / 726 pre-blindspot ready / 69 calibration |
| Combination miner | #1190 | `.../drug_combination_hypotheses.jsonl` | 1,750 pairs; 0 promoted |
| Effect-result gate | #1252 | `.../effect_result_rollup_status.jsonl` | 35 rollups; all blocked |
| Independent effect validation | #1253 | `.../independent_effect_rollup_status.jsonl` | **0 independent evidence rows** |
| FAERS expansion | #1249 | `.../faers_rollup_status.jsonl` | 82 co-report; 79 serious; blocked |
| RxNorm/TwoSIDES | #1258/#1259 | `.../pair_validation_rollups.jsonl` | 757 rows / 7 keys; 1 signal (metformin+trametinib) |
| Metformin+trametinib case | #1260 | `.../case_rollups.jsonl` | case 24608768: confounded (Eliquis primary, 19 drugs) |
| Ranker overlay | #1261 | `.../ranker_case_quality_overlay.jsonl` | 0.45 penalty; 5 rows |
| Safety/interaction rollup | #1228 | `.../pair_interaction_coverage_rows.jsonl` | 277 components / 1,750 pairs; all blocked |

---

## Appendix D — reproducibility and honesty contract

Every task in the source program adheres to a binding honesty contract, reproduced here because it is the epistemic backbone of the whole result:

> Only record a result as "grounded" if it cleared the Calyx honesty gate (`I(panel; outcome) ≥ H(outcome)`). Never assert a capability or a finding that was not actually run and verified against stored artifacts. A discovered association is a **ranked, traceable hypothesis**, never a verdict — it carries its full provenance chain and a sufficiency proof, and still requires experimental confirmation.

Reproduction of any claim in this paper proceeds by: (1) opening the named vault/collection or FSV root, (2) re-reading the persisted artifact bytes, (3) recomputing the SHA-256/BLAKE3, and (4) comparing against the value in the findings log. The findings log (`docs/medicalsearch/`, one file per atomic task, plus the combined export) is the diagnostic surface; the Calyx/Aster vaults are the source of truth.

**This paper makes no treatment, efficacy, safety-clearance, dosing, pair-interaction, clinical-actionability, recommendation, or cure claim. Every biomedical row it describes is a hypothesis requiring experimental confirmation and expert review.**

---

*Compiled from the append-only Calyx biomedical discovery findings log (`docs/medicalsearch/`, 112 source documents, epic #867 and derived issues #868–#1261). All quantitative values are drawn from the persisted, checksummed source-of-truth artifacts recorded therein.*
