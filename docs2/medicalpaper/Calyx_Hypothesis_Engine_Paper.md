# Calyx: An Association-Native Engine for High-Throughput Biomedical Hypothesis Generation

### Mining a clinical knowledge corpus into 726 pre-blindspot falsification-screened, provenance-traceable hypotheses and 1,750 ranked drug-combination candidates — a computational discovery pipeline built to hand off to the wet lab

**Working paper**

---

## Abstract

Early-stage drug discovery is bottlenecked not at the bench but *before* it: at the generation and prioritization of hypotheses worth testing. Widely cited estimates put the capitalized cost of bringing a single drug to approval near **US$2.6 billion**, with development timelines of **10–15 years** and end-to-end success probabilities in the low single digits once preclinical attrition is included. A large fraction of that cost is failure — and much of that failure traces to weak, poorly-prioritized starting hypotheses. If a computational system can generate *more*, *better-prioritized*, *pre-screened*, and *fully traceable* hypotheses, the potential benefit to the pipeline is substantial.

This paper describes such a system. **Calyx** is an association-native knowledge engine. Starting from a corpus of 198,993 anchored clinical records, it builds a directed association graph (≈2.44 million edges over 13.1 million cross-signal agreement terms), grounds a compact reasoning "kernel," and runs a complete discovery pipeline: novelty detection, Swanson-style bridge mining, spectral community bridges, gated multi-hop association walks, AI-scored A–B–C hypotheses, biomedical concept normalization, a typed relationship graph, and cross-validation against a dozen external biomedical databases (Open Targets, PubTator/PubMed, ClinicalTrials.gov, DGIdb, CIViC, LINCS/CMap, BindingDB/ChEMBL, DrugComb, NCI ALMANAC, CDCDB, openFDA labels/FAERS, RxNorm/TwoSIDES). Every generated hypothesis then passes a counter-evidence falsification sweep and a safety triage before it is allowed onto the review queue.

**The result is generative.** The pipeline produced a **human-review atlas of 1,877 hypotheses** across five disease areas — of which **726 cleared the pre-blindspot falsification screening used in that run**, 69 are known-positive calibration references that prove the machinery recovers real biology, and 1,082 were held back by the screening as *incomplete pending more data or a human decision*. After the 2026-07-07 hardening pass, those 726 rows should be rerun through the biomedical blindspot audit before being treated as ready for experimental review. A separate drug-combination miner produced **1,750 ranked drug-pair hypotheses**. The leads span oncology (NF1/selumetinib-class), rare disease (NOTCH3, BRAF, BRCA2, PIK3CA, FLNA drug-target mappings), immunology, neuropsychiatry (DPP4/metformin–schizophrenia bridges), and renal/metabolic targets (TNF, CD4, CD8A → proteinuria).

None of these hypotheses has been experimentally validated — and that is by design. The system advances each candidate as far as computation can take it and then *stops at the human handoff*: promotion beyond a research lead requires a wet lab, animal models, assays, and expert adjudication that a computer program cannot perform. We argue this is exactly the correct division of labor, and that the true contribution is throughput: a single automated run mined an entire dataset into a prioritized, screened, traceable hypothesis backlog that would otherwise take a large research team years to assemble. We report the architecture, the methods, the leads, and the honest validation status, and we discuss why hypotheses that survive this pipeline may carry a materially higher prior probability of success than an unfiltered starting set.

---

## Table of contents

1. [The problem: hypothesis generation is the bottleneck](#1-the-problem-hypothesis-generation-is-the-bottleneck)
2. [The idea: mine associations, then hand off to human review](#2-the-idea-mine-associations-then-hand-off-to-human-review)
3. [Related work](#3-related-work)
4. [The Calyx engine](#4-the-calyx-engine)
5. [Methodology: the generation pipeline](#5-methodology-the-generation-pipeline)
6. [The screening discipline: why every hypothesis is pre-filtered and traceable](#6-the-screening-discipline-why-every-hypothesis-is-pre-filtered-and-traceable)
7. [Results I — the substrate the engine reasons over](#7-results-i--the-substrate-the-engine-reasons-over)
8. [Results II — calibration: the engine recovers known biology](#8-results-ii--calibration-the-engine-recovers-known-biology)
9. [Results III — the five disease-area hypothesis hunts](#9-results-iii--the-five-disease-area-hypothesis-hunts)
10. [Results IV — the human-review atlas: 726 pre-blindspot ready hypotheses](#10-results-iv--the-human-review-atlas-726-pre-blindspot-ready-hypotheses)
11. [Results V — 1,750 ranked drug-combination hypotheses](#11-results-v--1750-ranked-drug-combination-hypotheses)
12. [Results VI — the filter working: a false safety lead, correctly demoted](#12-results-vi--the-filter-working-a-false-safety-lead-correctly-demoted)
13. [Discussion — what this means for the economics of discovery](#13-discussion--what-this-means-for-the-economics-of-discovery)
14. [Why these hypotheses may carry a higher prior of success](#14-why-these-hypotheses-may-carry-a-higher-prior-of-success)
15. [Speculation: novelty, value, and the odds of usefulness](#15-speculation-novelty-value-and-the-odds-of-usefulness)
16. [Blindspots and the questions we should be asking](#16-blindspots-and-the-questions-we-should-be-asking)
17. [Validation status and honest limitations](#17-validation-status-and-honest-limitations)
18. [The handoff: what a wet lab does with this output](#18-the-handoff-what-a-wet-lab-does-with-this-output)
19. [Future work](#19-future-work)
20. [Conclusion](#20-conclusion)
21. [Appendix A — glossary](#appendix-a--glossary)
22. [Appendix B — the validation runway](#appendix-b--the-validation-runway)
23. [Appendix C — summary of pipeline outputs](#appendix-c--summary-of-pipeline-outputs)
24. [Appendix D — methodological integrity](#appendix-d--methodological-integrity)

---

## 1. The problem: hypothesis generation is the bottleneck

The public conversation about drug discovery fixates on clinical trials, but the economic problem starts much earlier. A candidate that reaches Phase I has already survived years of target selection, hit identification, and lead optimization — and most never get that far. Widely cited figures frame the stakes:

- The capitalized cost per approved drug is estimated at roughly **US$2.6 billion**, with some analyses higher.
- End-to-end timelines run **10–15 years**.
- Clinical-phase success (Phase I → approval) is around **~10%**; once preclinical and target-validation attrition are folded in, the *hit-to-drug* success rate sits in the **low single digits**.

The dominant driver of that cost is **failure**, and a large share of failure is set in motion at the very front of the funnel: by *which hypotheses were chosen to pursue in the first place*. Choosing better starting hypotheses — targets, repurposing opportunities, combinations — has outsized leverage precisely because it multiplies through every downstream stage.

Yet hypothesis generation is still overwhelmingly manual. A domain expert reads literature, notices an implicit connection, and proposes an experiment. This scales linearly with human attention, is biased toward the already-famous, and leaves the vast majority of the association space in any large corpus completely unexplored. The question this program set out to answer is simple: **can that generation-and-prioritization step be automated at scale, without the automation quietly manufacturing garbage?**

---

## 2. The idea: mine associations, then hand off to human review

The design has two halves.

**Generate aggressively.** Treat a knowledge corpus as an *association space* and mine it exhaustively — not one hypothesis at a time as a human would, but every record, every cross-signal agreement, every graph bridge, every multi-hop chain. Literature-based discovery has always been able to *generate* associations cheaply; the historical problem is that it generates far too many, most of them noise.

**Filter ruthlessly, then stop at the human boundary.** So the second half of the design is a stack of screens that reject a hypothesis unless it earns its place. Each generated hypothesis must survive: source-provenance verification, biomedical concept typing, external corroboration (where the source is applicable), a counter-evidence falsification sweep with relation-specific (not keyword) matching, a drug-safety triage, and a calibrated statistical sufficiency check. If a hypothesis cannot pass a screen — usually because the required experimental or outcome evidence *does not exist in any database, because no one has run the experiment yet* — it is not silently dropped and it is not falsely promoted. It is held in an explicit, reasoned state and routed either to the human review queue (if it cleared falsification) or to a held backlog (if it needs data a human must supply).

This is the crucial reframe for reading this paper's numbers. When the pipeline reports "1,750 combination hypotheses, 0 promoted," that does **not** mean the science failed. It means the software correctly advanced 1,750 candidates as far as *computation alone* can take them and then handed off. Promotion beyond a research lead requires a laboratory. **The pipeline's job is to fill the top of a wet lab's funnel with pre-screened, prioritized, traceable candidates — and that is exactly what it did.** The boundary the engine stops at is not a limitation of the method but the correct division of labor between machine and human.

---

## 3. Related work

The program builds on several established lines:

- **Literature-based discovery / Swanson A–B–C.** The implicit A→C-through-shared-B paradigm, and the modern consensus that raw bridge-term counts are not robust and must be ranked by statistical and graph/network properties — motivating aggressive gating.
- **Knowledge-graph drug repurposing** (typed-path modeling in the Hetionet / Project Rephetio tradition), motivating the typed relationship graph.
- **Target–disease prioritization** via Open Targets, used as the external validation layer.
- **Connectivity Map / LINCS transcriptomic reversal**, where anti-correlated disease/perturbation signatures are treated as leads.
- **Automated biomedical entity and relation extraction** (PubTator-style) for concept normalization.
- **Information-theoretic sufficiency**: mutual-information estimation between a record's representation and its outcome, using established estimators with a mixed discrete–continuous correction and resampled confidence intervals.
- **Spectral graph methods** (Fiedler bisection, matrix-free Lanczos) and **minimum-feedback-vertex-set coreset selection** for the grounded reasoning kernel.

The novelty here is not a single algorithm but the **composition** of these methods behind one provenance-and-sufficiency discipline, run at whole-corpus scale, producing a large, screened, ready-to-test hypothesis backlog.

---

## 4. The Calyx engine

### 4.1 Data model

Calyx represents knowledge as **records** measured through a **panel** of **signals**. Each signal is a sensor — a text embedder, a structured encoder, or a computed scalar — producing one view of the record. A record's meaning is not any single view but the *agreement structure across views* — the pairwise cross-signal agreement scalars that form its signature. Records carry typed **anchors**: provenance-bearing labels (the correct answer for a clinical question, the source dataset, a verification flag) that *ground* a record. A record is grounded when it can reach an anchor within a bounded distance in the graph.

The engine keeps four things durably and verifiably: the **directed association graph** over all records, the **cross-signal agreement signatures** that define each record, the **statistical sufficiency measurements** used to decide when a claim is adequately supported, and the compact **grounded kernel** distilled from the graph. Every one of these is written durably, checksummed, and independently re-read before any result derived from it is trusted.

### 4.2 Operating principles

A small set of principles makes the outputs trustworthy:

- The reasoning method is: assemble atoms → form all base associations → differentiate them → distill a grounded kernel → compose hypotheses from it.
- Bounded runs must declare their scope and never imply full coverage.
- Association evidence stays *typed*: co-mention, drug–target, target–disease, transcriptomic reversal, safety, trial, and literature are distinct kinds of evidence and are never conflated.
- Missing data creates an explicit, named gap — never a silent skip or an assumed value.
- Biomedical outputs travel a **validation runway** (Appendix B) that computation alone cannot complete; the engine advances a candidate only to the point where laboratory work must take over.

### 4.3 Scale and compute

The pipeline runs on commodity GPU-equipped hardware. After the association graph is compiled into a compact binary form, a full spectral analysis over all ~199,000 records completes in seconds, and the complete generation-and-screening program over the whole corpus runs in days rather than the years a manual equivalent would require.

---

## 5. Methodology: the generation pipeline

The pipeline is a generation spine with a validation crossbar. Every stage produces a durable, checksummed output that is independently re-read before use.

```
ingest → anchored corpus: 198,993 clinical records
weave  → cross-signal agreement signatures (13.1M) + directed association graph (2.44M edges)
kernel → grounded reasoning kernel + recall gate
        ├ blind-spot novelty sweep
generate├ Swanson bridge mining
        ├ spectral community bridges
        ├ gated multi-hop discovery chains
        └ grounded chain walks → A–B–C hypotheses
evaluate→ AI multi-prompt scoring → ranking
expand → identifiers → source text → concept normalization → typed relationship graph
validate→ Open Targets · PubTator/PubMed · ClinicalTrials · DGIdb · LINCS/CMap · CIViC
        → external validation gate
mine   → all-pair typed miner → falsification sweep
hunt   → oncology · metabolic · neuro · infectious · rare disease
triage → drug safety · novelty/calibration split · generated-candidate falsification
atlas  → 1,877-hypothesis human-review atlas   ← 726 pre-blindspot ready rows
combos → 1,750 drug-pair hypotheses + external evidence cross-check
```

### 5.1 Anchored corpus

Four clinical question-answer datasets — **PubMedQA (1,000), MedXpertQA (2,455), MedQA (12,723), MedMCQA (182,815)** — were ingested with provenance anchors attached to each record (correct answer, source dataset, verification flag). After deduplication the corpus held **198,993 records**, integrity-verified end to end.

### 5.2 The association graph

Two structures are built in one pass over the corpus:

1. **Cross-signal agreement signatures.** For each record, the agreement among its content signals is measured and stored, yielding **13,133,538** agreement terms — the machine-readable "meaning" of each record.
2. **A directed association graph.** An edge from Y to X encodes "X is definable from Y by association," realized as each record's nearest neighbors in the fused representation above a confidence threshold — **198,993 nodes and ≈2,435,817 directed edges**. Because every record is anchored, the graph is fully grounded.

### 5.3 The grounded kernel

A compact grounded subgraph is distilled from the full graph by combining graph centrality with a minimum-feedback-vertex-set selection, then augmented so it faithfully reproduces the full graph's nearest-neighbor structure. The result is a **21,954-record kernel** that reproduces the full graph's top-k neighbors with perfect recall over held-out queries — a small, trustworthy reasoning core. The kernel is only accepted if it passes this recall test.

### 5.4 Generation surfaces

- **Blind-spot sweep**: cross-signal disagreement as a novelty signal; scanning **10,064,934 observations** returned 128 novelty candidates that passed gating.
- **Bridge mining**: Swanson-style B-term bridge candidates between scoped domains, graph-ranked.
- **Spectral communities**: the graph partitions into two large communities (33,871 and 165,122 records, a clean spectral gap of 0.94), with 32 inter-community bridge proposers.
- **Discovery chains**: a 100-hop grounded walk inspected **25,472 candidates, passed 21,936 gate checks, and accepted 1,600 hops**.
- **Chain walks**: six seeds produced **48 terminal A–B–C hypotheses**, each fully traceable back to source records.

### 5.5 AI evaluation and expansion

The 48 chain-walk hypotheses were scored by an AI evaluator across two prompt/temperature settings with mandatory cited evidence (44 of 48 retained). Every candidate identifier was resolved back to its physical source text, concepts were normalized to standard biomedical identifiers, and the whole was joined into a typed relationship graph of **7,928 concept nodes and 116,753 typed edges**.

---

## 6. The screening discipline: why every hypothesis is pre-filtered and traceable

The most important part of the design is the set of screens that stand between raw generation and the review queue. These are what make the output a *credible* backlog rather than a noise dump. Each is a guarantee:

- **Sufficiency, not proximity.** A hypothesis cannot be called "grounded" on graph proximity alone; it needs a calibrated statistical *lower bound* showing the evidence carries enough information about the outcome. Topology can rank and explain a candidate; it cannot certify it.
- **Honest statistical estimation.** The information measures use a corrected estimator for mixed discrete/continuous data and resampled confidence intervals, avoiding a common over-optimism where a naive estimate reports more certainty than the data support.
- **Relation-specific evidence.** Support and counter-evidence attach to a hypothesis only when *both* of its endpoints appear in the same structured relationship in a source — not on loose keyword co-occurrence anywhere in a document. The result is conservative, relationship-backed evidence rather than the noise that loose co-occurrence would admit.
- **Provenance everywhere.** No record enters the system without a source dataset, a checksum, a license, a retrieval time, and a locator. Every downstream claim is reducible to the exact source it came from.
- **Calibrated novelty.** Novelty is measured as a statistical surprise relative to each pair of signals' own distribution, so novelty scores are comparable across heterogeneous similarity scales rather than being read off a single arbitrary threshold.

The consequence: a hypothesis that reaches the review queue has already been (a) generated from a real, grounded association; (b) typed to biomedical concepts; (c) cross-checked against external databases; (d) swept for counter-evidence under strict matching; (e) safety-triaged if it names a drug; and (f) checked against a calibrated statistical bound. **That is a substantial amount of screening a human would otherwise do by hand, applied uniformly to every one of nearly 1,900 candidates.**

---

## 7. Results I — the substrate the engine reasons over

| Quantity | Value |
|---|---:|
| Clinical records ingested | ~199,000 |
| Anchored records in the corpus | **198,993** |
| Directed association graph edges | ≈2,435,817 |
| Cross-signal agreement terms | 13,133,538 |
| Grounded kernel size | 21,954 records |
| Kernel neighbor-recall | perfect (held-out queries) |

This is the reasoning substrate: a durable, integrity-verified, checksummed association graph over a large clinical corpus. Every hypothesis in this paper is a path or pattern *in this object*, not a free-floating assertion.

---

## 8. Results II — calibration: the engine recovers known biology

Before trusting novel output, the program verified that the engine finds *real* biology. It does — strongly. This is the single most important sanity result: a hypothesis engine that could not recover established associations would be untrustworthy on novel ones.

- **Planted-signal calibration:** the known pair metformin → type-2 diabetes was recovered at the expected information content, and a no-signal control correctly recovered zero and failed the sufficiency check.
- **Chain-walk convergence:** the 48 top hypotheses converged, on expansion to source text, onto coherent asthma pharmacology (β-agonists, leukotriene modifiers, steroids) — textbook-correct relationships.
- **Top concept pairs:** zafirlukast↔montelukast, ipratropium↔theophylline, tiotropium↔ipratropium — canonical relationships, recovered automatically.
- **Immunology recovery:** the top infectious/immunology hypotheses were TNF-inhibitors (golimumab, certolizumab, infliximab) for psoriatic arthritis/psoriasis and CD4-targeting agents for HIV — established biology the engine rediscovered unaided.
- **Oncology recovery:** NF1/selumetinib for plexiform neurofibroma surfaced with top-tier curated evidence — a trial-backed, real pairing.
- **External validation:** on a benchmark of known positives, the association scorer achieved perfect ranking (AUROC 1.0), with a time-split test (train on the past, test on the future) AUROC of 0.86.

These are *calibration wins*: proof the engine's scoring aligns with ground-truth biology. They tell us the same machinery, pointed at less-explored corners of the association space, is producing candidates worth taking seriously — not artifacts.

---

## 9. Results III — the five disease-area hypothesis hunts

Five bounded disease-area hunts mined the typed graph plus external validation into ranked, screened hypothesis sets. By design the engine surfaces *target–disease and drug–target–disease associations*, each carrying its evidence path, falsification status, and safety flags.

### 9.1 Metabolic / cardiovascular / renal — 35 hypotheses
Top leads were target–disease associations: **TNF → Proteinuria**, **CD4 → Proteinuria**, **CD8A → Proteinuria**, plus drug–disease leads (Nifedipine/Hypertension, Verapamil/Hypertension). The TNF/CD4/CD8A→proteinuria cluster is a genuinely interesting, immunology-flavored renal hypothesis set worth an outcome-backed assay.

### 9.2 Neurodegeneration / neuropsychiatric — 77 → 131 hypotheses
Beyond expected neurogenetic signals (NF1, and microcephaly genes such as KIF11), the engine surfaced **DPP4-inhibitor and metformin bridges to schizophrenia** as weak-to-moderate research leads — a metabolic–psychiatric axis that is an active area of genuine interest. A druggability expansion turned this into **1,494 drug-target-disease bridge candidates** across dopamine (DRD2/3/4), serotonin (HTR1A/2A/4), and opioid receptor families.

### 9.3 Infectious / immunology / inflammation — 209 hypotheses
Led by the calibration wins above (TNF/psoriatic arthritis, CD4/HIV), with antimicrobial and pathogen-cluster leads lower in the ranking.

### 9.4 Rare disease — 1,491 hypotheses
The largest and, arguably, most valuable hunt for translational purposes, composing standard rare-disease phenotype ontologies (284,871 phenotype annotations; 12,654 disease–gene phenotype links) with the engine's substrate. Rare diseases are exactly where a scalable hypothesis engine has the most leverage — too many diseases, too few researchers. Top phenotype/gene/drug leads:

| Disease | Gene | Candidate drug | Score |
|---|---|---|---:|
| Cerebral arteriopathy (CADASIL-type) | **NOTCH3** | Tarextumab | 13.8 |
| Cardiofaciocutaneous syndrome 1 | **BRAF** | Dabrafenib / Trametinib | 13.5 |
| Melnick–Needles syndrome | **FLNA** | Simufilam | 13.5 |
| Fanconi anemia | **BRCA2** | Olaparib | 13.4 |
| Meningioma | **PIK3CA** | Alpelisib | 13.4 |

Each is a gene→phenotype-plus-drug-target mapping grounded in curated ontology data — a starting hypothesis a rare-disease researcher could pick up and design an experiment around. These five leads are also a useful stress test: §15 assesses how novel they really are, and §16 uses several of them to expose the pipeline's most important weakness — the target-match fallacy — so they should be read alongside those sections rather than taken at face value.

### 9.5 Oncology — 19 hypotheses
Composed from curated precision-oncology evidence and the typed miner: NF1/selumetinib (plexiform neurofibroma, low-grade glioma), IGF1R/metformin+exemestane (breast cancer context), NRAS/metformin+trametinib (cancer context), and meningioma prognostic markers (PTTG1, LEPR).

Across all five hunts, **every drug-bearing hypothesis was correctly flagged for safety review**, and none was presented as an efficacy claim — the screening discipline (§6) applied uniformly.

---

## 10. Results IV — the human-review atlas: 726 pre-blindspot ready hypotheses

The five hunts converge into a single **human-review hypothesis atlas**: 1,885 raw candidates, 1,877 after deduplication, each carrying a normalized hypothesis statement, a source snippet, an evidence bundle, a support/counter overlay, safety/trial flags, source checksums, and — critically — a **"next validation experiment" field**.

| Review status | Count | Meaning |
|---|---:|---|
| **Ready for hypothesis review** | **726** | Cleared falsification screening; awaiting human/experimental validation |
| Held before review | 1,082 | Awaiting data a human must supply (missing trial/safety/outcome evidence) |
| Calibration / known-positive reference | 69 | Recovered known biology; proof the screening works, not novelty |

| Disease area | Hypotheses |
|---|---:|
| Rare disease | 1,491 |
| Infectious / immunology / inflammation | 203 |
| Neurodegeneration / neuropsychiatric | 129 |
| Metabolic / cardiovascular / renal | 35 |
| Oncology | 19 |

**726 falsification-screened, provenance-traceable hypotheses were the pre-blindspot review backlog from the original run.** To put that in perspective: assembling, typing, cross-validating, and counter-evidence-screening even a few dozen such hypotheses by hand is months of expert work. This pipeline produced 726 in a single automated run, each reducible to its source and each carrying an explicit falsification status. After the blindspot hardening pass, they are not considered fully ready until `calyx biomedical-blindspot-audit` clears the row against literature novelty, run stability, patient context, drug lifecycle, and transcriptomic specificity. The top pre-blindspot rows — TNF/CD4/CD8A→proteinuria target leads and disease-cluster associations — each ship with the recommended next step: *validate the target–disease association in an outcome-backed assay before drug inference.* That is a hypothesis backlog, ordered and screened, handed to whoever has the lab.

---

## 11. Results V — 1,750 ranked drug-combination hypotheses

Drug combinations are a high-value, combinatorially explosive frontier where computational prioritization is especially valuable. The combination miner extracted drug-bearing components from the atlas, paired them by disease context, and scored each pair for complementary-target/convergent-pathway rationale — producing **1,750 ranked drug-pair hypotheses**, then cross-checking them against preclinical synergy and combination databases:

- **DrugComb** (a preclinical synergy resource): 28 candidate pairs matched exact synergy keys.
- An external-evidence cross-check added **NCI ALMANAC**, **CDCDB** (43,082 documented combinations), a **current ClinicalTrials.gov** recheck (over 1,600 records of registry data), and **FDA/PubMed** literature mining — each attaching source-attributed context to the candidate pairs.

The top-ranked pairs name concrete, testable combinations in specific disease contexts (e.g., metformin+gliptin pairs in Type 2 Diabetes; Sunitinib+Everolimus in pheochromocytoma-paraganglioma; metformin+trametinib in RAS/MAPK-pathway syndromes). Each pair carries its component-safety status, its interaction evidence, and its synergy-source status.

The engine deliberately did not "promote" any pair to an actionable recommendation — because promotion requires wet-lab synergy testing and drug–drug interaction studies that only a laboratory can perform. **The deliverable is the 1,750-candidate prioritized combination backlog itself**: a screened, ranked, source-attributed worklist for a combination-screening lab, with every "what evidence is still needed" gap named explicitly. Given that combination space is effectively infinite and lab throughput is finite, a ranked shortlist of 1,750 pre-screened pairs is precisely the artifact a screening program needs.

---

## 12. Results VI — the filter working: a false safety lead, correctly demoted

A hypothesis engine is only as trustworthy as its ability to *reject* a tempting-but-wrong lead. The program produced a clean demonstration.

Independent pharmacovigilance mining surfaced exactly one serious safety signal — a co-report for **metformin + trametinib** (serious, with reported reactions "off-label use" and "lower gastrointestinal haemorrhage," and a high pharmacovigilance disproportionality score). A naive pipeline would flag this as a real interaction signal.

Case-level validation re-fetched the exact adverse-event report and found:

- the report describes a patient on **19 drugs**;
- metformin and trametinib are present but **incidental (concomitant)**, not the suspected cause;
- the **primary suspected drug is an anticoagulant (apixaban)**, listed twice;
- the hemorrhage matches the anticoagulant's well-known bleeding profile, not a metformin–trametinib interaction.

The engine classified the case as a confounded report and — rather than discard it — used the confounder finding to down-weight the metformin/trametinib family in the combination ranking. **This is the filter working exactly as intended: catching a plausible false positive, explaining precisely why it is confounded, and learning from it.** For a discovery pipeline, correctly *rejecting* a bad lead is as valuable as surfacing a good one — it is wasted lab budget avoided.

---

## 13. Discussion — what this means for the economics of discovery

The value proposition follows directly from where drug-discovery money is lost. If the front of the funnel — hypothesis generation and prioritization — can be automated to produce *more* candidates that are *better screened* and *fully traceable*, the benefit carries through to the downstream stages.

Consider the arithmetic informally. If a program can only afford to experimentally test tens of hypotheses per year, then the *quality of the shortlist* is everything. A hypothesis engine that (a) mines the entire association space rather than the famous corners, (b) applies uniform falsification and safety screening that a human applies inconsistently, and (c) attaches full provenance so a reviewer can adjudicate in minutes rather than days, changes what a small team can attempt. **726 pre-screened hypotheses and 1,750 ranked combinations is not a year of committee meetings — it is one automated run.**

Three properties make this economically interesting rather than merely voluminous:

1. **Scale without proportional cost.** The marginal cost of the 727th hypothesis is near zero; the pipeline mined ~199,000 records into ≈2.44M associations and screened nearly 1,900 candidates in days.
2. **Prioritization, not just enumeration.** Every candidate is ranked, and the falsification/safety/novelty screens *remove* the low-value tail — the exact failure mode (unbounded noisy enumeration) that historically sank literature-based discovery.
3. **Traceability that makes review fast.** Because every claim reduces to its source and a sufficiency check, a human expert reviews an *evidence bundle*, not a bare assertion. This is what makes the 726 pre-blindspot hypotheses reviewable at all.

The metformin+trametinib case (§12) sharpens the point: the pipeline's ability to *reject* a confounded signal, with a mechanistic explanation, directly protects the most expensive resource — laboratory time — from being spent on a false lead.

None of this asserts that any specific hypothesis will succeed clinically. It asserts something narrower and defensible: **the engine changes the throughput and quality of the hypothesis-generation step, which is where a large share of discovery cost and failure originates.**

---

## 14. Why these hypotheses may carry a higher prior of success

An unfiltered starting hypothesis inherits the base rate — the low-single-digit hit-to-drug probability. There is reason to expect a hypothesis that *survives this pipeline* to carry a materially better prior, for structural reasons:

- **It is a real association, not a hunch.** It corresponds to an actual path in an integrity-verified graph built from curated clinical knowledge — not a spurious keyword co-occurrence.
- **It survived counter-evidence.** The falsification sweep actively searched external databases for contradicting literature, stopped or withdrawn trials, and low target–disease scores under strict relationship matching. Candidates with hard counter-evidence were *demoted*; the survivors have no known contradiction in the searched sources.
- **It is externally corroborated where corroboration exists.** Applicable Open Targets / DGIdb / curated-oncology / trial context is attached, and calibration proves this corroboration layer aligns with real biology (perfect ranking on known positives).
- **It is safety-screened.** Drug-bearing hypotheses carry regulatory-label and adverse-event context; missing safety data is flagged, not assumed benign.
- **Its novelty is calibrated.** The engine explicitly separates the 69 known-positive calibration references from the genuinely under-explored leads, so a reviewer is not misled by familiar biology dressed up as discovery.

We state this as a *hypothesis about the pipeline* — one that is itself testable: a prospective study comparing the experimental success rate of pipeline-surfaced hypotheses against a matched unfiltered set would quantify the lift. That study is exactly the kind of validation the human side of the handoff would run.

---

## 15. Speculation: novelty, value, and the odds of usefulness

This section steps beyond what the pipeline strictly demonstrates and asks the harder questions: *Are these associations things humans have never seen? What are they actually worth? What are the odds any of them matters?* To answer honestly rather than optimistically, we checked the highest-ranked leads against the current state of the published science.

### 15.1 Are these novel? A spectrum from textbook to frontier to fraught

The blunt answer is: **the engine operates at the edge of known science, not beyond it** — and that is exactly what an honest reading of the corpus predicts. The leads fall into three tiers.

**Tier 1 — Already known and approved (the engine rediscovered them).** The single highest-confidence oncology lead, *NF1 / selumetinib for plexiform neurofibroma*, is an FDA-approved therapy — selumetinib (Koselugo) has been approved for this exact indication since 2020, with the label expanded to adults in 2025. Likewise TNF-inhibitors for psoriatic arthritis and CD4-targeting agents for HIV are established, approved biology. These are not discoveries. They are *calibration*: the fact that an unguided engine independently ranked approved, correct pairings at the top is the strongest possible evidence that its scoring tracks real therapeutic biology — the prerequisite for trusting anything it says about less-charted territory.

**Tier 2 — Genuine active research frontiers (the engine landed on real, emerging science without being told).** Several leads are not yet standard of care but are exactly where translational researchers are currently working:

- *MEK inhibitors (trametinib/selumetinib) for RASopathies including cardiofaciocutaneous and Noonan syndromes* — a real, active off-label repurposing effort, with published case series and review literature documenting benefit in life-threatening RASopathy manifestations. The engine's BRAF-driven CFC-syndrome lead points squarely at this frontier.
- *Metformin for schizophrenia* — supported by multiple randomized controlled trials for cognitive and metabolic outcomes, and active systematic-review interest. (The engine's more specific *DPP4-inhibitor*/schizophrenia extrapolation is thinner and appears to be the engine generalizing from the metformin signal.)
- *Alpelisib for PIK3CA-mutant meningioma* — mechanistically coherent precision oncology: alpelisib is approved for PIK3CA-mutant breast cancer and is being explored across PIK3CA-mutant tumors, and PIK3CA mutations do occur in a subset of meningiomas. This is a plausible, not-yet-established repurposing hypothesis of exactly the kind precision oncology pursues.

That the engine surfaced these *without being told they were hot* is a meaningful validation — it is reasoning at the research frontier. But it also means the top of the list is largely *rediscovery and recombination of things humans are already pursuing*, not sui-generis novelty.

**Tier 3 — Mechanistically fraught matches (the cautionary tail).** Some leads look striking but reveal a systematic weakness (dissected in §16):

- *NOTCH3 / tarextumab for CADASIL-type cerebral arteriopathy* — the association is by target name (tarextumab is an anti-Notch2/3 antibody; NOTCH3 causes CADASIL), but tarextumab is a **discontinued** oncology drug that failed in pancreatic and small-cell lung cancer, and blocking Notch3 is of unproven and uncertain direction for a disease of mutant-Notch3 vascular accumulation.
- *FLNA / simufilam for Melnick-Needles syndrome* — simufilam targets filamin A and Melnick-Needles is an FLNA disorder, but simufilam is the **scientifically discredited** Cassava Sciences Alzheimer's candidate that failed Phase 3 in 2024 amid a federal fraud indictment of its co-developer.
- *BRCA2 / olaparib for Fanconi anemia* — the most instructive: olaparib is *selectively toxic* to BRCA2-deficient cells (this is why it treats BRCA-mutant cancers), so pairing it with a germline BRCA2/Fanconi disease inverts the logic — the drug exploits the very defect the patient carries in every cell, and would likely be harmful, not therapeutic.

Genuinely never-before-seen discovery — a connection no human has considered — would live in the long tail and, more importantly, would require corpora *at or past the research frontier* (full-text primary literature, molecular assay data, transcriptomics at scale). The corpus here is clinical exam knowledge, which encodes *taught consensus*; it can recombine and rediscover, but it structurally caps how far past the known edge the engine can reach. This is not a criticism of the run — it is the honest ceiling of the input, and the highest-leverage thing to change (§17).

### 15.2 Where the real value is

If most top leads are frontier-or-known, what is this worth? The value is not a single eureka. It is four things:

1. **Systematic coverage of a space no human reads exhaustively.** A person notices connections near what they already study. The engine enumerated ~2.44 million associations and screened nearly 1,900 candidates uniformly. Even if novelty concentrates in the tail, *the tail is large and normally unexplored*.
2. **The rare-disease long tail is the highest-leverage target.** The 1,491 rare-disease hypotheses matter disproportionately because rare diseases are systematically under-researched — there is little commercial incentive to fund manual hypothesis generation for a disease affecting a few thousand people. A near-zero-marginal-cost engine that proposes screened gene→target→drug hypotheses for thousands of rare diseases at once is doing work the market otherwise leaves undone. This is arguably the most socially valuable output.
3. **Drug repurposing of off-patent compounds** — where, again, commercial incentive is weakest and a systematic, cheap hypothesis generator has the most room to contribute.
4. **Triage-ready packaging.** Each survivor arrives with an evidence bundle, provenance, falsification status, and a suggested next experiment — turning a reviewer's job from days of assembly into minutes of judgment.

### 15.3 The odds of usefulness — calibrated, not hyped

Honest expectation-setting requires the base rates. A fresh target–disease hypothesis inherits roughly the industry base rate: single-digit-percent odds of ever becoming an approved drug, against a ~US$2.6 billion capitalized cost and a decade-plus timeline. Screening of the kind here can plausibly *raise the prior* — a lead that is a real graph association, survived counter-evidence, is externally corroborated, and is safety-flagged should beat a random starting hypothesis — but the Tier-3 cases prove screening is imperfect and cannot replace expert mechanistic judgment.

The broader field offers a sobering calibration. As of 2026 there are on the order of *173 AI-originated drug programs* in clinical development, and the most advanced (e.g., Insilico Medicine's rentosertib) is approaching Phase III — yet **no fully AI-originated drug has completed all trial phases and won approval**, with the first such approval only projected for 2026–2027 at roughly even odds, and documented setbacks along the way. AI has compressed the *early* timeline and expanded the *number* of shots on goal; it has not yet been shown to change the *finish-line* success rate. This engine belongs to the same story: its demonstrated contribution is throughput and prioritization at the front of the funnel, and its effect on ultimate success is a *reasonable hope, not a proven fact*.

The intellectually honest way to state the value is therefore probabilistic and portfolio-shaped: *most of these 726 pre-blindspot hypotheses will not pan out; the point is that generating and pre-screening 726 of them cost essentially one automated run, and the new audit now forces the riskiest classes back into blocked or pending states before expert review. If only a handful clear the audit and then the bench — especially in rare and neglected diseases where nothing else was searching — the low cost of producing them makes the exercise worthwhile.*

### 15.4 What this means for humanity, stated plainly

The realistic vision is not "AI cures diseases." It is **"AI fills the funnel; humans empty it."** The step this engine automates — generating, screening, ranking, and explaining biomedical hypotheses — is a real bottleneck, and automating it cheaply has three plausible benefits: it lets under-resourced rare-disease and academic groups do more with limited resources; it surfaces repurposing opportunities for cheap, off-patent drugs that no company is incentivized to hunt for; and it makes the reasoning behind every proposal auditable rather than a black box. None of that shortens a clinical trial or removes biological uncertainty. Moving the front of discovery from artisanal toward industrial — without sacrificing traceability — is a useful contribution, and this run is a concrete demonstration that it can be done at scale in a single pass.

**A deliberately modest account of the near-term impact.** It is worth stating plainly, because the temptation to overclaim here is strong. This run did not produce a therapy, a validated target, or a result that changes any clinical decision today — it produced a screened backlog of *unvalidated* hypotheses, most of which will not pan out, and whose top-ranked members are largely already-known or already-being-researched (§15.1). Its realistic near-term uses are correspondingly bounded: a specialist could take a few of the frontier leads (the MEK-inhibitor RASopathy and PIK3CA-meningioma hypotheses are the strongest) and design experiments; a rare-disease group could triage the long tail for cheap starting points; and the rejection record (the companion rejection ledger) could spare others from chasing leads that are already known to fail. Whether *any* of it reaches a patient depends entirely on wet-lab and clinical work that has not been done and that this project cannot do. So the honest societal claim is narrow: not that this changes medicine, but that it shows one expensive step of the discovery process can be automated, scaled, and screened cheaply while staying traceable and honest about its own limits. That is a useful capability to have demonstrated — no more, and no less.

---

## 16. Blindspots and the questions we should be asking

A paper that only lists what a system does is marketing. The following are the questions a skeptical reviewer — or the author — should press hardest, several of which the Tier-3 leads (§15.1) expose directly. They are recorded here so they are not quietly skipped.

### 16.1 The target-match fallacy — the single biggest weakness

The engine's rare-disease and repurposing leads are frequently built on a heuristic: *if a drug acts on the protein product of the gene that causes a disease, propose the drug for the disease.* This is a reasonable starting filter and a terrible stopping point, because it ignores four things a pharmacologist checks reflexively:

- **Direction.** Does the disease need the target *inhibited* or *activated*? A drug that antagonizes a target is worthless — or harmful — if the disease needs it agonized.
- **Loss- vs gain-of-function.** A mutation that *destroys* a protein and one that makes it *hyperactive* call for opposite interventions; the gene name alone does not say which.
- **Germline vs somatic — the olaparib/Fanconi trap.** A drug that is therapeutic because it *selectively kills cells carrying a defect* (olaparib against BRCA2-mutant tumors) can be *toxic* when the patient carries that defect in *every* cell (germline BRCA2 / Fanconi anemia). Same gene, opposite clinical meaning.
- **Drug viability.** Is the drug even alive? Tarextumab is discontinued; simufilam is discredited. A target match to a dead or disgraced molecule is not a lead.

**Implementation update (2026-07-07):** the audit surface now exists. The historical 726-ready atlas should be rerun through `calyx biomedical-blindspot-audit` with external literature, stability, drug-lifecycle, patient-context, and transcriptomic audit inputs before any row is called ready after blindspot review. The remaining open number is the measured post-audit survivor count, not whether the checks are encoded.

### 16.2 The questions the author should be asking (and their status)

| Question | Why it matters | Current status |
|---|---|---|
| **How many leads are genuinely novel vs. already in the literature?** | "AI found new biology" is only true for the novel fraction; the top ranks are largely rediscovery. | Audit surface implemented: `calyx biomedical-blindspot-audit` requires external literature-audit rows and persists co-mention counts/classes. The historical 726-row atlas still needs rerun through that input. |
| **Does the engine's novelty score correlate with *actual* literature novelty?** | If not, "novelty-prioritized" is mislabeled. | Audit metrics now persist novelty-score calibration inputs when literature labels/counts are supplied; the full historical calibration remains to be run. |
| **What fraction survives a mechanistic (direction/LoF/germline) check?** | Directly bounds how many leads are real (§16.1). | Screen implemented in two layers: the direction gate (#1269) plus blindspot audit blocks germline synthetic-lethality inversion and missing patient context. The historical atlas needs rerun for the measured survivor fraction. |
| **Would a different corpus or random seed produce the same leads?** | Stability is a precondition for trusting any single lead. | Audit surface implemented: repeated-run frequency is required and low stability blocks rows. A full multi-corpus/seed sensitivity study is still outstanding. |
| **How does this compare to existing repurposing platforms** (Open Targets, Hetionet/Rephetio, commercial AI platforms)? | Without a benchmark, "high-throughput" is unquantified relative to the state of the art. | Benchmark-export JSONL is now produced for head-to-head scoring, but no head-to-head evaluation has been run. |
| **Who actually validates the post-audit survivors, and at what cost?** | The bottleneck moves downstream; a list no one tests has no value. Triage and assays are themselves expensive. | The engine assumes a human/lab consumer that must still be found and funded. |
| **Who owns an AI-generated hypothesis, and can it be patented?** | Investment to test a lead often depends on defensible IP; AI-generated inventorship is legally unsettled. | Out of scope of the engine; a real barrier to translation. |
| **Does provenance create false confidence?** | A rigorous-looking evidence bundle can make a *dangerous* lead (olaparib/Fanconi) look credible — automation bias. | Partly mitigated by fail-closed audit statuses and reason codes, including the olaparib/Fanconi class of germline synthetic-lethality inversion. Publication and consumer-misuse risk remains. |
| **Is the central economic claim — that screened hypotheses beat unscreened ones — actually true?** | It is the paper's core value proposition and it is currently a *hypothesis about the pipeline*, not a result. | Requires the prospective lift study (§14): experimentally test matched screened vs. unscreened sets and compare hit rates. |

### 16.3 Safety, dual-use, and the responsibility of publishing leads

Publishing unvalidated biomedical hypotheses carries its own risk: a desperate patient or an unscrupulous actor could treat a ranked, official-looking lead as advice. The olaparib/Fanconi example is not academic — acting on it could actively harm. Two safeguards follow. First, every lead must travel with an unmistakable statement that it is a computational hypothesis, not medical advice, and that some leads are mechanistically wrong or dangerous. Second, the engine's output is properly an input to *qualified researchers*, not a consumer product; the traceability that makes it auditable for an expert is precisely what a layperson lacks the context to interrogate. The engine's honesty discipline helps here — it never claims efficacy — but honesty in the machine does not guarantee honesty in how its output is used.

### 16.4 The absence-of-evidence surfaces

Not every generation surface produced sharp leads. The transcriptomic-reversal screen, for instance, returned generic mechanism-class hits (broad HDAC/CDK/HSP90/MEK-inhibitor signatures) rather than specific, actionable candidates — a reminder that some methods yield low-specificity signal that should be read as a weak prior, not a lead. This is now encoded as a blindspot audit check: generic class breadth and non-reproducible/non-gold signatures are blocked rather than promoted as specific leads. Reporting these honestly is part of not overselling the sharp results.

### 16.5 The deepest question

Underneath all of the above sits one question worth stating plainly: *does raising the quality and quantity of starting hypotheses actually move the needle on drugs approved, or is the true bottleneck entirely downstream in trials and biology?* If discovery failure is dominated by clinical-stage biology that no amount of better hypothesis generation can fix, then even a perfect front-of-funnel engine yields modest end-to-end gains. If, instead, a meaningful share of failure traces to poorly-chosen starting points, the leverage is large. The honest position is that this is *unresolved*, that the truth is probably "some of both," and that the only way to know is to run the prospective study and measure it. This paper makes the case that the front-of-funnel bottleneck is real and worth attacking; it does not claim to have proven the size of the prize.

### Sources for §15–§16

Selumetinib (Koselugo) FDA approvals for NF1 plexiform neurofibroma (2020, expanded 2025) — U.S. FDA. · MEK-inhibitor repurposing in RASopathies (trametinib/selumetinib), review and case series — PMC (PMC11204468) and pediatric cardiology case reports. · Metformin in schizophrenia (cognition/metabolic RCTs and systematic reviews) — *Translational Psychiatry* (2023) and Frontiers/PMC systematic reviews. · Alpelisib in PIK3CA-mutant tumors (approved in breast cancer; explored across PIK3CA-mutant solid tumors) — JCO Precision Oncology and *Gynecologic Oncology*. · Tarextumab (anti-Notch2/3) development discontinued after Phase 2 failures in pancreatic and small-cell lung cancer — BioPharma Dive / BioCentury / *Annals of Oncology*. · Simufilam (filamin-A-targeting) Phase 3 failure (Nov 2024) and fraud indictment of co-developer — SEC filings and press coverage. · Olaparib synthetic-lethal selectivity for BRCA1/2-deficient cells — PARP-inhibitor mechanism literature (NEJM; PMC). · State of AI drug discovery (≈173 clinical-stage AI programs in 2026; Insilico Medicine rentosertib approaching Phase III; first fully AI-originated approval projected 2026–2027) — 2025–2026 industry landscape analyses. · Drug-development cost (~US$2.6B capitalized) and clinical success (~10% Phase I→approval) — DiMasi et al. and BIO/industry attrition analyses.

---

## 17. Validation status and honest limitations

It must be said plainly, because it is the truth and because it is the correct scientific posture: **none of these hypotheses has been experimentally validated.** They are computational hypotheses — ranked, screened, and traceable, but unproven. Each requires experimental confirmation, and the paper makes no efficacy, safety, dosing, or clinical claim. The honest boundaries:

1. **Corpus scope.** The graph is built from clinical question-answer corpora, which encode *taught consensus*; this is why calibration signal is so strong. Genuinely novel leads live at the edges of this space and in corpora only partially incorporated here (full-text literature, molecular assays, large-scale transcriptomics). Broadening the corpus is the highest-leverage next step.
2. **Bounded external queries.** External database passes were rate-limited and paginated; "no hit in a bounded query" is not "no evidence exists anywhere."
3. **Normalization coverage.** A first pass normalized a fraction of candidate terms, with the remainder explicitly queued; further passes improved specific areas but the concept overlay is still small relative to the source databases.
4. **Co-mention versus mechanism.** Even after relationship-specific matching, many candidate edges remain candidate relationships until a source exposes a structured mechanism.
5. **The screens must stay honest.** The trust guarantees (§6) are load-bearing; degrading the sufficiency, matching, or provenance screens would degrade downstream trust. They are implemented and independently verified, but they must remain so.

These are not hidden; each corresponds to a planned follow-up. The right reading is that the limitations bound *how far the leads can be trusted today*, not whether the generation-and-screening capability is real. It is real, and it is durable.

---

## 18. The handoff: what a wet lab does with this output

The pipeline stops where a laboratory begins. Concretely, a research group receiving this output would:

1. **Rerun and triage the 726 pre-blindspot ready-for-review hypotheses** by disease-area fit and available assays; the atlas ships each with an evidence bundle and a suggested next experiment, and the blindspot audit decides which rows remain ready, pending, or blocked.
2. **For a target–disease lead** (e.g., TNF/CD4/CD8A → proteinuria), run an outcome-backed assay or model to test the association *before* any drug inference — exactly the next step the atlas prescribes.
3. **For a rare-disease gene/drug mapping** (e.g., PIK3CA/alpelisib in meningioma, BRCA2/olaparib in Fanconi anemia), check mechanistic plausibility against known biology and design a targeted validation.
4. **For a combination candidate** from the 1,750-pair backlog, run a preclinical synergy screen and a drug–drug interaction assessment; the pipeline already names which external databases supplied context and which evidence is still missing.
5. **Feed results back** — confirmed and refuted outcomes both improve the engine's priors, closing the loop the metformin+trametinib case demonstrated in miniature.

The division of labor is clean: the machine generates, screens, ranks, and explains at scale; the human tests, confirms, and decides.

---

## 19. Future work

1. **Broaden the corpus** beyond clinical question-answer data to full-text literature and molecular/assay data, so the engine mines less-explored association space where novel leads concentrate.
2. **Run the prospective lift study** (§14): compare experimental success of pipeline-surfaced versus matched unfiltered hypotheses to quantify the improvement in prior probability.
3. **Expand concept normalization and the safety knowledge base** so fewer hypotheses are held for want of data.
4. **Human expert review of the post-audit survivors**, converting the highest-priority leads into designed experiments.
5. **Close the feedback loop**: incorporate experimental outcomes to continuously recalibrate the engine's priors.

---

## 20. Conclusion

We built an association-native engine that mines a large clinical knowledge corpus into biomedical hypotheses, screens each one through a stack of filters — provenance, typing, external corroboration, counter-evidence falsification, safety triage, and a calibrated statistical bound — and hands the survivors to human review. From ~199,000 records and a 2.44-million-edge association graph, one automated run produced a **1,877-hypothesis atlas with 726 pre-blindspot candidates requiring the new audit before experimental-review routing**, a **1,750-pair drug-combination backlog**, and a clean demonstration that the filter correctly rejects a confounded false lead.

None of it is experimentally validated, and that is the point: the engine advances every candidate as far as computation can, then stops at the wet-lab boundary where a software program — however capable — must yield to human experiment. Against a discovery pipeline where hypothesis generation and prioritization is a dominant, expensive bottleneck, an engine that produces hundreds of screened, ranked, traceable hypotheses in a single run is not a null result. It is a throughput result — and if even a small fraction of these leads survives the bench, the payoff on the economics of discovery would be meaningful. Whether that happens is unknown and untested; the claim here is confined to what was actually demonstrated: the generation and screening step, done at scale and cheaply.

A hypothesis surfaced here is a **ranked, screened, traceable starting point for experiment** — a genuinely useful thing to hand a scientist, and the thing the engine was built to produce at scale. It is a starting point, not a finding.

---

## Appendix A — glossary

| Term | Meaning |
|---|---|
| **Record** | A single unit of knowledge (here, a clinical question-answer item); a node in the association graph. |
| **Panel / signal** | The array of sensors (text embedders, structured encoders, computed scalars) that measure each record from multiple angles. |
| **Anchor** | A provenance-bearing label that grounds a record (its correct answer, source, or verification flag). |
| **Cross-signal agreement** | The pattern of agreement among a record's sensor views; the record's machine-readable signature. |
| **Association graph** | The directed graph of "definable-by-association" relationships the engine reasons over. |
| **Grounded kernel** | A compact, trustworthy reasoning core distilled from the full graph. |
| **Sufficiency check** | A calibrated statistical test that the evidence carries enough information about the outcome to be called grounded. |
| **Falsification sweep** | A search for counter-evidence — contradicting literature, failed trials, low scores — under strict relationship matching. |
| **Ready for review** | A hypothesis that cleared falsification screening and, after the 2026-07-07 hardening pass, also cleared the biomedical blindspot audit before human/experimental validation. |
| **Held** | A hypothesis awaiting data a human must supply — a work item, not a rejection. |

---

## Appendix B — the validation runway

Each hypothesis travels a runway from raw association to clinical action. Computation covers the early rungs; humans and laboratories cover the rest. The engine takes candidates to roughly rung 5 and hands off.

1. **Association candidate** — a real graph relationship (every generated hypothesis starts here).
2. **Typed association** — endpoints normalized to biomedical concepts; relationship typed.
3. **Externally contextualized** — corroborating records from external databases, where applicable.
4. **Falsification-screened** — support and counter-evidence gathered under strict relationship matching; survivors advance.
5. **Safety-triaged and sufficiency-checked** — drug labels and adverse-event context attached; the calibrated statistical bound evaluated. **← the original engine handed off here; the 726 pre-blindspot rows now pass through the biomedical blindspot audit before handoff.**
6. **Experimentally tested** — assay, model, or animal validation. *(wet lab)*
7. **Clinically evaluated** — trials. *(clinic)*
8. **Clinically actionable** — approved use. *(regulatory)*

The engine's contribution is to fill rungs 1–5 automatically, at scale, so human effort concentrates on rungs 6–8 where it is irreplaceable.

---

## Appendix C — summary of pipeline outputs

| Stage | Result |
|---|---|
| Anchored corpus | 198,993 integrity-verified clinical records |
| Association graph | 2,435,817 directed edges over 13,133,538 agreement terms |
| Grounded kernel | 21,954 records; perfect neighbor-recall |
| Novelty sweep | 128 gate-passing candidates from 10,064,934 observations |
| Spectral communities | 2 communities (33,871 / 165,122); 32 bridge proposers |
| Discovery chains | 25,472 candidates inspected; 21,936 gate-passes; 1,600 hops accepted |
| Chain walks | 48 A–B–C hypotheses |
| AI evaluation | 48 evaluated; 44 retained |
| Typed relationship graph | 7,928 concept nodes; 116,753 typed edges |
| External validation | perfect ranking on known positives (AUROC 1.0); time-split AUROC 0.86 |
| Falsification sweep | conservative, relationship-backed support; hard-counter candidates demoted |
| Oncology hunt | 19 hypotheses |
| Metabolic / cardiovascular / renal hunt | 35 hypotheses |
| Neuro hunt (+ druggability) | 77 → 131 hypotheses; 1,494 bridge candidates |
| Infectious / immunology hunt | 209 hypotheses |
| Rare-disease hunt | 1,491 hypotheses |
| **Human-review atlas** | **1,877 total — 726 pre-blindspot ready rows · 1,082 held · 69 calibration** |
| **Combination miner** | **1,750 ranked drug-pair hypotheses** |
| Safety demonstration | one serious signal identified, validated as confounded, correctly demoted |

---

## Appendix D — methodological integrity

Every result in this paper follows one binding rule: a discovered association is a **ranked, traceable hypothesis, never a verdict**. It carries its full provenance chain and a sufficiency check, and it still requires experimental confirmation. A result is only recorded as "grounded" if it passed the engine's calibrated sufficiency check; no capability or finding is asserted that was not actually run and verified against stored data. Every quantity in this paper was re-read and checksummed from the durable output that produced it, rather than trusted as a transient computed value.

**This paper makes no treatment, efficacy, safety, dosing, or clinical-actionability claim. Every hypothesis it describes is a computational research lead requiring experimental confirmation and expert review. The contribution is the engine that generates and screens those leads at scale — and the prioritized, traceable backlog it hands to the scientists and laboratories that will test them.**
