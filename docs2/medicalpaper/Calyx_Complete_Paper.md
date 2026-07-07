**Chris Royse** &middot; [chrisroyseai@gmail.com](mailto:chrisroyseai@gmail.com)
<br>Projects: [github.com/calyx](https://github.com/calyx) &middot; [github.com/synapse](https://github.com/synapse)

---

# Abstract {: .no_toc}

Early-stage drug discovery is bottlenecked not at the bench but *before* it: at the generation and prioritization of hypotheses worth testing. Widely cited estimates put the capitalized cost of bringing a single drug to approval near **US$2.6 billion**, over **10–15 years**, with end-to-end success probabilities in the low single digits. Much of that cost is failure, and much of that failure traces to weak, poorly-prioritized starting hypotheses. If a computational system can generate *more*, *better-prioritized*, *pre-screened*, and *fully traceable* hypotheses — and, just as importantly, *reject* the wrong and dangerous ones before anyone acts on them — the potential benefit to the pipeline is substantial.

This report describes such a system and everything it produced. **Calyx** is an association-native knowledge engine. From a corpus of 198,993 anchored clinical records it builds a directed association graph (≈2.44 million edges over 13.1 million cross-signal agreement terms), grounds a compact reasoning kernel, and runs a complete discovery pipeline — novelty detection, Swanson-style bridge mining, spectral community bridges, gated multi-hop association walks, AI-scored A–B–C hypotheses, concept normalization, a typed relationship graph, and cross-validation against a dozen external biomedical databases. Every generated hypothesis then passes a counter-evidence falsification sweep and a safety triage before it reaches a review queue.

The pipeline produced a **human-review atlas of 1,877 hypotheses** across five disease areas — of which **726 cleared the pre-blindspot screening used in that run**, 69 are known-positive calibration references that prove the machinery recovers real biology, and 1,082 were held back as incomplete pending more data or a human decision — plus a separate **1,750 ranked drug-combination hypotheses.** After the 2026-07-07 hardening pass, those 726 rows should be rerun through the biomedical blindspot audit before being treated as ready for experimental review. None has been experimentally validated, and that is by design: the engine advances each candidate as far as computation can take it and then stops at the human handoff.

This document combines three works into one report. **Part I** presents the engine, the methods, the leads, the economics, and an honest account of validation status, speculation, and blindspots. **Part II** is a lead-by-lead catalog of every named hypothesis, what makes each distinctive, and a candid probability that pursuing it would prove useful. **Part III** is the rejection ledger: every hypothesis, drug pairing, and safety signal the engine ruled out — including a lead that would have harmed a patient — and why catching those is as important as generating the good ones.

# How to read this document {: .no_toc}

The three parts are self-contained but complementary, and are best read in order.

- **Part I — The Calyx Hypothesis Engine.** The main paper: what the system is, how it works, what it found, and what it all means. Start here.
- **Part II — The Hypothesis Catalog and the Odds of Usefulness.** Goes lead by lead through every named hypothesis, grading each on validity, novelty, and a subjective probability of usefulness. Read it to understand *which specific leads matter and why.*
- **Part III — The Rejection Ledger.** The safety story: every lead the engine refused, the mechanism behind each rejection, and the human stakes. Read it to understand *why an engine's "no" is what makes it safe to use.*

Two conventions apply throughout. First, section numbers restart within each part; a reference like "(§6)" means section 6 *of the part you are reading*. Second, and most importantly: **nothing in this report is medical advice.** Every hypothesis is a computational lead awaiting expert review, and several named entries — especially in Part III — are included specifically as examples of ideas that must be *rejected*. Numbers presented as probabilities are subjective priors, not experimental results.

# Contents {: .no_toc}

[TOC]

# Part I — The Calyx Hypothesis Engine {: .part}

*The main paper: architecture, methodology, results across five disease hunts, the economics of discovery, and an honest account of validation status, speculation, and open questions.*

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


# Part II — The Hypothesis Catalog and the Odds of Usefulness {: .part}

*A lead-by-lead companion. For every hypothesis the engine surfaced by name: what it is, what makes it special or unique, and a candid, subjective probability that pursuing it would produce something genuinely useful.*

## 1. How to read this catalog

The engine produced far more hypotheses than any human would read: 726 leads marked ready for review before the blindspot audit, 1,082 more held behind evidence gates, 69 calibration references, and a separate 1,750 ranked drug-combination pairs. Cataloging literally *every one* of several thousand rows individually would be neither honest nor useful — most of the large rare-disease and combination buckets are variations on a theme, and the source only ever enumerated the top-ranked members of each.

So this catalog does two things. For every hypothesis the engine surfaced **by name** — every lead that appears explicitly in the ranked outputs — it gives a full entry. For the large buckets where only the top rows were named, it details every named row and then characterizes the remainder statistically. Nothing here is invented; every lead below came out of the engine's own ranked tables.

Each hunt section has the same shape: a table covering all its named leads, followed by prose deep-dives on the handful that genuinely matter. The tables carry three judgment columns, defined next.

---

## 2. The probability model, stated honestly

Three columns appear throughout. They are **subjective priors, not experimental results.** They encode my best reading of each lead against the current published science and against drug-discovery base rates — nothing in this engine has been tested at a bench.

**Validity** — is the underlying association *real*? Does the gene actually relate to the disease, does the drug actually act on the target? Graded High / Medium / Low.

**Novelty** — is it *new*? Three states:

- *Known* — already established, often already approved. Zero discovery value; maximum calibration value (it proves the engine's scoring tracks real biology).
- *Frontier* — real, plausible, and actively being researched but not yet standard of care. This is where genuine value concentrates.
- *Fraught* — the association is a surface match (drug hits the disease gene's product) that a pharmacologist would reject on mechanism. Often *negative* value: pursuing it wastes resources or, in a few cases, would be actively harmful.

**P(useful)** — a single number: my estimate of the probability that pursuing this specific lead yields a **real, non-obvious, translationally useful result.** This deliberately scores already-approved pairings *low* (the biology is real but there is no new value) and scores mechanistically-broken matches *low* (they will not validate). It rewards only leads that are both plausibly real **and** not yet established. The anchor points:

| Band | Meaning | Typical members |
|---|---|---|
| **~1–3%** | Already known, or mechanistically broken — little/no new value | Approved pairings; dead-drug target matches; known disease-gene rediscoveries |
| **~5–10%** | Real but vague, or a known relationship dressed as a lead | Broad inflammation/co-mention associations |
| **~10–30%** | Genuine research frontier: plausible, active, not yet established | Repurposing leads with mechanistic logic and some external corroboration |
| **>30%** | Reserved — nothing here earns it; the engine works at the edge of known science, not past it | (none) |

For calibration against reality: a *randomly chosen* fresh target–disease hypothesis has low-single-digit-percent odds of ever becoming an approved therapy, against a ~US$2.6 billion capitalized cost and a decade-plus timeline. As of 2026 there are roughly 173 AI-originated drug programs in clinical development and **not one has yet completed all trials and won approval.** These P(useful) numbers are therefore small on purpose. A lead at 20% is, in this catalog, a *strong* lead.

---

## 3. Oncology hunt

The smallest and most rigorously-sourced hunt: 19 candidates, most drawn from curated precision-oncology evidence with formal evidence levels (A = strongest). This is the hunt where the engine's top pick is a fully approved therapy — the single best evidence that its ranking is anchored in real biology.

| Lead (gene → cancer → drug) | Evidence | What makes it notable | Validity | Novelty | P(useful) |
|---|---|---|---|---|---:|
| NF1 → plexiform neurofibroma → **selumetinib** | Level A | The engine's top oncology pick is an FDA-approved therapy for this exact indication | High | Known | ~2% |
| NF1 → childhood low-grade glioma → selumetinib | Level B | Same drug, active/emerging pediatric indication | High | Frontier | ~15% |
| IGF1R → breast cancer → **metformin + exemestane** | Level B | Real IGF-axis + metabolic oncology research direction | Medium | Frontier | ~12% |
| PTTG1 (overexpr.) → meningioma | Level B | Prognostic marker, not a therapy — biomarker lead | Medium | Frontier | ~8% |
| LEPR (underexpr.) → meningioma | Level B | Prognostic marker; leptin-receptor biology in meningioma is under-studied | Medium | Frontier | ~8% |
| NF1 → skin melanoma → vemurafenib | Level C | *Cautionary:* NF1 loss is a known driver of BRAF-inhibitor **resistance** | Medium | Fraught | ~4% |
| NRAS → cancer → metformin + trametinib | Level D | *Cautionary:* the exact pair the engine later flagged as confounded in a safety-signal check | Low | Fraught | ~3% |
| NF1 → skin melanoma → sirolimus + mirdametinib | Level D | mTOR + MEK vertical-pathway combination logic | Medium | Frontier | ~10% |
| NF1 (loss) → malignant peripheral nerve sheath tumor → JQ1 | Level D | BET-bromodomain inhibitor; real preclinical MPNST research line | Medium | Frontier | ~9% |

**The deep dives:**

**NF1 → plexiform neurofibroma → selumetinib (the calibration win).** This is not a discovery and should not be sold as one. Selumetinib (Koselugo) has been FDA-approved for NF1-associated plexiform neurofibroma since 2020, with the label expanded to adults in 2025. Its value is the opposite of novelty: an unguided engine, ranking by its own internal scoring over curated evidence, independently placed a *correct, approved* gene→drug pairing at the top of its highest-quality hunt. That is one of the more reassuring results in this body of work, because it suggests the scoring is not just producing noise — here it tracked the same biology a pharmacologist would. Everything more speculative in this catalog earns its credibility from the fact that the engine got this one right. P(useful) as a *discovery* is ~2% only because the discovery already exists.

**IGF1R → breast cancer → metformin + exemestane (a real frontier).** Metformin's role in breast-cancer biology — via the IGF-1/insulin axis and AMPK signaling — is a genuine, actively-studied translational question, and combining it with an aromatase inhibitor like exemestane in ER-positive disease is exactly the kind of metabolic-oncology strategy under investigation. The engine landed on a plausible, live hypothesis without being told it was interesting. It is not established, hence the frontier tag and ~12%.

**NF1 → melanoma → vemurafenib (the instructive mistake).** This lead looks reasonable — NF1 is a melanoma driver, vemurafenib is an approved melanoma drug — but the mechanism runs the wrong way: loss of NF1 is a well-documented route to *resistance* against BRAF inhibitors like vemurafenib. The engine's own evidence layer flagged "predictive resistance" rows for the NF1/vemurafenib relationship. This is the target-match fallacy in miniature: a real gene, a real drug, a real cancer, and a pairing a specialist would reject. It earns its place in the catalog precisely as a worked example of why the P(useful) column has to be skeptical.

---

## 4. Rare-disease hunt

The largest and, for translational leverage, the most valuable hunt: 1,491 candidates built by composing standard rare-disease phenotype ontologies with the engine's substrate. Rare diseases are systematically under-researched — too many diseases, too few researchers, little commercial incentive — so a cheap engine that proposes thousands of screened gene→target→drug hypotheses at once is doing work the market otherwise leaves undone. That is the good news. The bad news is that the top of this list is where the **target-match fallacy** shows up most starkly, because the ranking rewards a bare "drug hits the protein made by the disease gene" match.

| Lead (gene → disease → drug) | Score | What makes it notable | Validity | Novelty | P(useful) |
|---|---|---|---|---|---:|
| NOTCH3 → CADASIL cerebral arteriopathy → **tarextumab** | 13.83 | Ranked #1 overall — but the drug is discontinued and the direction is unproven | Low | Fraught | ~1% |
| BRAF → cardiofaciocutaneous syndrome → **trametinib** | 13.51 | The genuine gem: MEK inhibition for a RASopathy is a real, active repurposing frontier | Medium | Frontier | ~20% |
| BRAF → cardiofaciocutaneous syndrome → dabrafenib | 13.51 | *Cautionary:* BRAF inhibitors can paradoxically activate MAPK in non-V600 RAS/RAF states | Low | Fraught | ~4% |
| BRAF → cardiofaciocutaneous syndrome → cetuximab / panitumumab | 13.51 | *Cautionary:* anti-EGFR antibodies — weak mechanistic rationale for a germline RASopathy | Low | Fraught | ~2% |
| FLNA → Melnick–Needles syndrome → **simufilam** | 13.47 | *Cautionary:* the drug is scientifically discredited (failed Phase 3; fraud-tainted) | Low | Fraught | <1% |
| BRCA2 → Fanconi anemia → **olaparib** | 13.37 | *Dangerous:* olaparib is selectively **toxic** to BRCA2-deficient cells | Low | Fraught | ~1% |
| PIK3CA → meningioma → **alpelisib** | 13.37 | Coherent precision-oncology repurposing: alpelisib targets mutant PIK3CA | Medium | Frontier | ~15% |

**The deep dives:**

**BRAF → cardiofaciocutaneous syndrome → trametinib (the one genuinely worth chasing).** Cardiofaciocutaneous (CFC) syndrome is a RASopathy — a developmental disorder of over-active RAS/MAPK signaling, frequently BRAF-driven. Trametinib is a MEK inhibitor that damps exactly that pathway *downstream* of BRAF. Off-label MEK inhibition (trametinib, selumetinib) for severe, life-threatening RASopathy manifestations is a real and active clinical-research effort, with published case series describing benefit in otherwise untreatable presentations. The engine reached this frontier from phenotype ontology data alone. Of every lead in this catalog, this is among the most defensible — a real disease mechanism, a mechanistically-correct drug, and an existing research community already moving in the same direction. Hence the ~20%, the highest single number in the document. Note the contrast with its own siblings: the *same* disease paired with dabrafenib (a BRAF inhibitor, which can paradoxically *activate* the pathway in these states) or with anti-EGFR antibodies is far weaker — the engine found the right disease but ranked right and wrong drugs identically, because it scored on target-name overlap, not mechanism direction.

**NOTCH3 → CADASIL → tarextumab (ranked #1, and a trap).** CADASIL is caused by mutations in NOTCH3 that lead to pathological accumulation of the receptor in small cerebral vessels. Tarextumab is an anti-Notch2/3 antibody — so on a pure target-name basis, the match is perfect, and the engine ranked it first overall. Two things sink it. First, tarextumab is a *discontinued* oncology drug that failed in pancreatic and small-cell lung cancer; it is not an available therapy. Second, whether blocking Notch3 helps, harms, or does nothing in a disease of *mutant* Notch3 vascular deposition is genuinely unknown and mechanistically fraught — the disease is not simply "too much Notch3 signaling." A #1 rank built entirely on nomenclature overlap is the clearest single illustration of why this catalog's probability column cannot follow the engine's own score.

**BRCA2 → Fanconi anemia → olaparib (the dangerous one).** This is the most important cautionary entry in the whole catalog, because acting on it could hurt someone. Olaparib is a PARP inhibitor whose entire therapeutic logic is *synthetic lethality*: it selectively kills cells that are already BRCA-deficient, which is why it treats BRCA-mutant cancers. Fanconi anemia patients carry that deficiency in *every cell in their body* (biallelic BRCA2/FANCD1 disease). Giving them a drug engineered to kill BRCA2-deficient cells inverts the entire rationale — it is far more likely to be toxic than therapeutic. Same gene name, opposite clinical meaning. The engine cannot tell the difference between "the tumor has this defect" (exploit it) and "the patient has this defect everywhere" (never exploit it). P(useful) ~1%, and flagged red for safety.

**FLNA → Melnick–Needles syndrome → simufilam.** Melnick–Needles is an FLNA (filamin A) disorder; simufilam was promoted as a filamin-A-modulating drug, so the names line up. But simufilam is the discredited Cassava Sciences Alzheimer's candidate that failed Phase 3 in 2024 amid a federal fraud indictment of its co-developer. A target match to a scientifically-repudiated molecule is not a lead. <1%.

**PIK3CA → meningioma → alpelisib.** The cleanest frontier lead in this hunt. Alpelisib is an approved inhibitor of mutant PIK3CA (in breast cancer), PIK3CA mutations occur in a subset of meningiomas, and precision-oncology practice actively explores PIK3CA inhibitors across PIK3CA-mutant tumors. Mechanistically coherent, not yet established: ~15%.

**On the other 1,483 rare-disease rows.** Below the named top, the hunt is dominated by two structural patterns. 891 rows are gene→disease→drug bridges (like the ones above), and 600 are pure target-prioritization rows with no drug attached. The same rules apply at scale: a large fraction will be bare target matches that fail the direction/loss-vs-gain/germline check, a meaningful minority will be mechanistically coherent repurposing leads worth a specialist's afternoon, and the *aggregate* value is the long tail — thousands of screened starting points for diseases nobody else is systematically searching. If even 1–3% of the novel rare-disease rows are both real and non-obvious, that is dozens of genuine leads from a single automated run.

---

## 5. Neurodegeneration / neuropsychiatric hunt

77 ranked rows (later normalized/expanded to 131). The top of this hunt is dominated by *known neurogenetics* — the engine rediscovering the causative-gene lists for neurofibromatosis and microcephaly — which is calibration, not discovery. The interesting material is lower down: a metformin/DPP4/schizophrenia bridge that maps onto a real research frontier.

| Lead | Score | What makes it notable | Validity | Novelty | P(useful) |
|---|---|---|---|---|---:|
| NF1 → neurofibromatosis type 1 | 0.95 | Textbook causative gene — pure calibration | High | Known | ~1% |
| NF1 → neurofibromatosis-Noonan syndrome | 0.92 | Known RASopathy overlap | High | Known | ~2% |
| KIF11 / PCNT / WDR62 / MCPH1 / CDK5RAP2 / NDE1 → microcephaly | 0.89–0.90 | All six are *established* microcephaly genes — the engine recovered the known panel | High | Known | ~1% |
| **Metformin → DPP4 → schizophrenia** | 0.83 | Maps to real RCT-backed research on metformin in schizophrenia | Medium | Frontier | ~15% |
| Saxagliptin / anagliptin / omarigliptin / … → DPP4 → schizophrenia | 0.87–0.83 | Engine over-generalizing from the metformin signal to the whole gliptin class | Low | Fraught | ~4% |
| Proteinuria ↔ diffuse neurofibrillary tangles w/ calcification | 0.95 | Disease–disease co-mention cluster, not a mechanism | Low | Fraught | ~3% |
| Meningitis (bacterial) ↔ subarachnoid hemorrhage | 0.89 | Co-mention cluster; plausibly a shared-presentation artifact | Low | Known | ~3% |
| Tinnitus ↔ vertigo; otosclerosis ↔ tinnitus | 0.73–0.78 | Known clinical co-occurrence, no new content | High | Known | ~2% |
| Quinidine / kanamycin / streptomycin / phenytoin → tinnitus | (co-mention) | *Inverted:* these are known *causes* of ototoxic tinnitus, not treatments | High | Fraught | ~1% |

**The deep dives:**

**Metformin → DPP4 → schizophrenia (the real signal in the noise).** This is the one lead in the neuro hunt worth taking seriously as a research direction, and notably the engine itself scored it as only a *weak-to-moderate* bridge — an honest self-assessment. Metformin has multiple randomized controlled trials in schizophrenia, mostly targeting the metabolic side-effects of antipsychotics but with signals on cognitive and weight outcomes, and it is an active systematic-review topic. The engine surfaced a genuine, live question. The ~15% reflects that this is real and frontier, tempered by the fact that the specific *mechanistic framing* here (routing it through DPP4) is thinner than the metformin evidence itself.

**The gliptin/schizophrenia fan-out (over-generalization).** Having found the metformin/schizophrenia signal, the engine proposed the entire DPP4-inhibitor class — saxagliptin, anagliptin, omarigliptin, prusogliptin, bisegliptin, even valacyclovir (whose DPP4 link it flagged as "mechanistically ambiguous") — for schizophrenia. This is a characteristic failure mode: a real signal for one drug gets extrapolated to a target class without independent evidence for each member. These are marked Fraught not because they are impossible but because they inherit their entire plausibility from a single neighbor. ~4% each.

**The microcephaly gene panel (calibration, mislabeled as leads).** Ranks 4–10 are KIF11, PCNT, WDR62, MCPH1, CDK5RAP2, and NDE1 against microcephaly. Every one of these is a *textbook causative microcephaly gene*. The engine recovered the known panel — excellent evidence that its target-disease scoring is sound, but zero discovery value and no drug attached. These belong in the calibration column alongside NF1, not in a leads worklist.

**The tinnitus drugs (a direction inversion worth noting).** Quinidine, kanamycin, streptomycin, and phenytoin all appear associated with tinnitus. But these are classic *ototoxic* agents — they are known *causes* of tinnitus, not candidate treatments. The engine surfaced a real, strong association and got its *sign* wrong, exactly as in the melanoma/vemurafenib and olaparib/Fanconi cases. A useful reminder that a high association score is direction-blind.

---

## 6. Infectious / immunology / inflammation hunt

The largest non-rare hunt: 209 ranked rows. Its top is almost entirely *known-positive immunology* — the engine recovering approved biologic therapies — which is why the source itself flagged that calibration rows were crowding out novelty. As a discovery worklist the top is low-value; as proof the scoring works, it is excellent.

| Lead (drug → target → disease) | Score | What makes it notable | Validity | Novelty | P(useful) |
|---|---|---|---|---|---:|
| Golimumab / certolizumab → TNF → psoriatic arthritis | 1.40 | Approved anti-TNF biologics for the exact disease — calibration | High | Known | ~2% |
| Infliximab → IL12B → psoriasis | 1.24 | Established biologic-target-disease immunology | High | Known | ~2% |
| Ibalizumab → CD4 → HIV | 1.18 | Approved CD4-directed antibody for HIV — calibration | High | Known | ~2% |
| Tregalizumab / zanolimumab → CD4 → HIV | 1.20–1.28 | *Cautionary:* discontinued/non-HIV CD4 antibodies matched by target name | Low | Fraught | ~2% |
| Placulumab / cefotaxime → TNF → psoriatic arthritis | 1.25–1.27 | *Noise:* a non-anti-TNF antibody and an antibiotic mis-bridged onto TNF | Low | Fraught | ~1% |
| Ipratropium ↔ steroids; steroids ↔ theophylline | 1.24 | Asthma co-prescription co-mentions, not new biology | High | Known | ~2% |
| Streptomycin → Klebsiella infections / rhinoscleroma | 0.95–0.98 | Actually a *correct* known antimicrobial indication (rhinoscleroma) | High | Known | ~3% |
| Thalassemia ↔ Salmonella infections | 0.91 | Real, textbook susceptibility association | High | Known | ~3% |
| TNF / CD4 / CD8A → proteinuria | 1.20–1.25 | Broad inflammation-in-kidney-disease association; vague as a lead | Medium | Frontier | ~7% |

**The deep dives:**

**The anti-TNF / anti-CD4 top block (calibration, and a mixed bag underneath).** Golimumab and certolizumab for TNF-driven psoriatic arthritis, infliximab for IL12B in psoriasis, and ibalizumab for CD4 in HIV are all *approved, correct* pairings. The engine ranking them at the top is a clean calibration win. But immediately beneath them sit target-name look-alikes that do *not* carry the same weight: tregalizumab and zanolimumab are anti-CD4 antibodies developed for other indications (and discontinued/repurposed), and placulumab (an antibody) and cefotaxime (an antibiotic) landed on the psoriatic-arthritis list only because they touched "TNF" in the source graph. This is the calibration-vs-noise problem: correct approved biology and mechanistically-empty name matches receive nearly identical scores.

**The pathogen associations that are actually right.** Two low-glamour rows deserve credit. Streptomycin against *Klebsiella*/rhinoscleroma is a *correct* known indication — rhinoscleroma (chronic *Klebsiella rhinoscleromatis* infection) really is treated with aminoglycosides. And thalassemia's association with *Salmonella* susceptibility is textbook. The engine recovered real infectious-disease knowledge. Zero novelty, but these are the reassuring "it isn't hallucinating" rows.

**TNF / CD4 / CD8A → proteinuria (real but shapeless).** These immune targets genuinely relate to kidney inflammation and proteinuria — the biology is real. But "TNF associates with proteinuria" is not, by itself, an actionable therapeutic hypothesis; it is a broad, well-known relationship with no drug and no direction. It scores as Frontier/Medium and ~7% because a specialist *could* build something from it, but the engine has not done that work.

---

## 7. Metabolic / cardiovascular / renal hunt

The most tightly-bounded hunt: 35 rows, with metformin/DPP4 material explicitly carried as *known context* rather than claimed discovery. The top is target-context (associations without a drug); the drug-bearing rows are conventional antihypertensives.

| Lead | Score | What makes it notable | Validity | Novelty | P(useful) |
|---|---|---|---|---|---:|
| TNF → proteinuria (target-disease) | 6.80 | Highest-scored row; broad inflammation association, no drug | Medium | Frontier | ~7% |
| CD4 → proteinuria | 6.51 | Same class of immune-renal association | Medium | Frontier | ~6% |
| CD8A → proteinuria | 5.40 | Same class | Medium | Frontier | ~5% |
| Steroids → proteinuria | 4.94 | Steroids in proteinuric kidney disease is standard care — known | High | Known | ~2% |
| Nifedipine → hypertension | 4.80 | Approved antihypertensive — pure calibration | High | Known | ~1% |
| Verapamil → hypertension | 4.80 | Approved antihypertensive — pure calibration | High | Known | ~1% |
| Alogliptin / linagliptin / saxagliptin / sitagliptin / vildagliptin → DPP4 → type 2 diabetes | 4.0–4.4 | Approved gliptins for diabetes — explicitly flagged as known context | High | Known | ~1% |

**The deep dive:**

This hunt is mostly a calibration exercise, and usefully so. Nifedipine and verapamil for hypertension, and the gliptin class for type 2 diabetes, are all approved, correct, and were *labeled as known context by the engine itself* rather than dressed up as findings — an honest touch. The only rows with any research texture are the immune-target→proteinuria associations (TNF, CD4, CD8A), which are real inflammation-in-nephropathy biology but, as in the infectious hunt, too broad to be a lead without a specialist attaching a mechanism and a molecule. Nothing in this hunt is novel; its worth is as further evidence that the scoring recovers established cardiometabolic biology.

---

## 8. The human-review atlas: the leads that rose to the top

When all five hunts were consolidated, deduplicated, and re-ranked by a blended confidence×novelty score into the 726-lead review atlas, these rose to the very top. Tellingly, the atlas's top is *not* the same as any single hunt's top — the combined novelty weighting pushed the broad renal-inflammation and disease-cluster associations up, and pushed the known-approved calibration pairings down (they were routed to the 69-row calibration set instead). That reranking is the atlas working as intended.

| Atlas rank | Lead | Area | Confidence | Novelty | What makes it notable | P(useful) |
|---:|---|---|---:|---:|---|---:|
| 1 | TNF → proteinuria | metabolic/renal | 0.56 | 0.75 | Highest combined score; real immune-renal biology, no drug/direction yet | ~7% |
| 2 | Diffuse neurofibrillary tangles w/ calcification ↔ proteinuria | neuro | 0.44 | 0.73 | High-novelty *because* it is an odd cross-domain co-mention — novelty score rewards strangeness, not correctness | ~3% |
| 3 | CD4 → proteinuria | metabolic/renal | 0.55 | 0.72 | Immune-renal association | ~6% |
| 4 | CD8A → proteinuria | metabolic/renal | 0.54 | 0.70 | Immune-renal association | ~5% |
| 5 | Subarachnoid hemorrhage ↔ bacterial meningitis | neuro | 0.40 | 0.64 | Cross-domain cluster; plausibly shared acute-presentation artifact | ~3% |

**What the atlas top actually teaches.** Rank 1 is a real but shapeless association; ranks 2 and 5 are *disease–disease co-mention clusters* that scored high on novelty precisely because they are unusual pairings — and unusual is not the same as correct. This exposes a structural point the main paper's blindspot section makes: the engine's novelty score measures *surprise within its own graph*, not *novelty against the literature*, and surprise is exactly what artifactual co-mentions produce. The genuinely promising frontier leads from the individual hunts (BRAF/CFC/trametinib, PIK3CA/meningioma/alpelisib, metformin/schizophrenia) sit *below* these in the blended atlas ranking. A human reviewer working the atlas top-down would hit noise before signal — which is itself a useful finding about how to weight the ranking.

---

## 9. Drug-combination hypotheses

A separate miner generated 1,750 ranked drug-pair hypotheses from the atlas components. Every single one is *blocked* — not because the pairs are bad, but because the miner is fail-closed: it refuses to promote any pair missing component safety data, pair-interaction evidence, or external synergy evidence, and the safety substrate covered only 14 of 1,023 drug components. So the 1,750 are best read as a *structured worklist naming exactly what evidence each pair still needs*, not as ranked bets.

| Rank | Pair | Context | What makes it notable | P(useful) |
|---:|---|---|---|---:|
| 1 | Metformin + sitagliptin | type 2 diabetes | Already a marketed fixed-dose combination — pure calibration | ~1% |
| 2 | Metformin + saxagliptin | type 2 diabetes | Already marketed together — calibration | ~1% |
| 3 | Metformin + alogliptin | type 2 diabetes | Already marketed together — calibration | ~1% |
| 4 | Metformin + linagliptin | type 2 diabetes | Already marketed together — calibration | ~1% |
| 5 | Sunitinib + everolimus | hereditary pheochromocytoma-paraganglioma | Both are real agents studied in PPGL; genuine frontier combination | ~12% |

**The one external-evidence hook.** The miner cross-checked all pairs against DrugComb, a preclinical cell-line synergy database, and got 28 exact pair-key matches covering 47 rows. Those pairs carry independent *preclinical* corroboration that their combination does something measurable in cells — the only rows in the entire combination set with outside support. That corroboration does not clear the safety/interaction blocks and is not clinical efficacy, but it is the right place for a reviewer to start.

**Reading the top honestly.** The top four combination "hypotheses" are metformin+gliptin pairs that are *already sold as single pills* — the miner rediscovered marketed combination products, which is calibration, not discovery. The first genuinely interesting row is sunitinib + everolimus for hereditary pheochromocytoma-paraganglioma: both drugs are individually studied in PPGL, the combination has a real angiogenesis-plus-mTOR rationale, and it is not established. That is the combination lead worth a specialist's attention. As with the single-agent hunts, the useful signal sits just below a layer of already-known material at the top.

---

## 10. The portfolio view: what the whole set is worth

Putting the individual numbers together, the honest shape of the value is a portfolio, not a jackpot:

- **Calibration tier (~10–15% of named leads, and the 69-row reference set):** approved pairings and known disease-gene panels — NF1/selumetinib, the anti-TNF and anti-CD4 biologics, the antihypertensives, the gliptins, the microcephaly genes. Discovery value ≈ 0. Their worth is that they *validate the machine*: an unguided engine that keeps placing correct, approved biology at the top of independent hunts is an engine whose scoring can be trusted on the harder rows.

- **Frontier tier (a minority of named leads):** BRAF/CFC-syndrome → trametinib (~20%), PIK3CA/meningioma → alpelisib (~15%), metformin → schizophrenia (~15%), IGF1R/breast → metformin+exemestane (~12%), sunitinib+everolimus → PPGL (~12%), NF1/MPNST → JQ1 (~9%). These are the actual bets. Each is plausible, mechanistically defensible, not yet established, and — crucially — the engine reached each one *unprompted*. None is a sure thing; a 15–20% lead is, against industry base rates, a strong one.

- **Fraught tier (a meaningful slice of the top-ranked rare-disease and class-extrapolation rows):** tarextumab/CADASIL, simufilam/Melnick-Needles, olaparib/Fanconi, the dabrafenib and anti-EGFR siblings, the gliptin/schizophrenia fan-out, the ototoxic "tinnitus treatments." These have low or *negative* expected value and, in the olaparib/Fanconi case, real safety risk. They are the price of a scoring function that rewards target-name overlap without a mechanism-direction check.

- **The long tail (the bulk of the 726 pre-blindspot ready + 1,082 held):** individually low-probability, collectively the point. If just 1–3% of the *novel* leads across all hunts turn out to be both real and non-obvious after audit and expert review, that is on the order of **10–30 genuine, useful contributions** from a single automated run — most of them in rare and neglected diseases where nothing else was systematically searching. That is the number that justifies the whole exercise.

The realistic expected value of this specific run is therefore: a handful of frontier leads worth a specialist's serious time (led by the MEK-inhibitor RASopathy and PIK3CA-meningioma hypotheses), a larger set of screened rare-disease starting points worth cheap triage, a clear demonstration that the scoring recovers known biology, and a concrete, worked catalog of the failure mode (target-match without mechanism) that the next version of the engine should screen out. **AI filled the funnel; a human still has to empty it — but the funnel is real, and a few things in it are worth carrying downstream.**

---

## 11. Honest caveats on every number in this document

- **The probabilities are subjective priors, not measurements.** No hypothesis here has been tested at a bench. Every P(useful) is my judgment, anchored to a literature check and to industry base rates, and a different expert would move individual numbers by 5–10 points. Treat them as *relative* rankings and rough magnitudes, not precise odds.
- **"Useful" is defined narrowly and deliberately harshly:** real, non-obvious, *and* translationally actionable. That is why approved, correct biology scores *low* — there is no new value in rediscovering it — even though those are the rows we are *most* certain are true. Do not read a low number on a calibration row as "the engine got it wrong"; it got it exactly right, which is the point.
- **The novelty judgments are against the published literature as I checked it, not against the engine's internal novelty score.** The two disagree, and where they do (the atlas top), the literature check is the one to trust.
- **The fraught-tier leads are not merely "less good" — several would fail on mechanism or, in one case, could harm a patient.** The olaparib/Fanconi row in particular must never be read as a treatment suggestion.
- **The large-bucket estimates (the rare-disease tail, the held rows) are statistical characterizations, not row-by-row verdicts.** Only the named leads received individual judgment; the rest are described by pattern.
- **Nothing here is medical advice.** Every entry is a computational hypothesis awaiting expert review, and some of the named entries are included specifically as examples of hypotheses that should be *rejected*.

### Sources consulted for the novelty and mechanism judgments

Selumetinib (Koselugo) FDA approval for NF1 plexiform neurofibroma (2020; adult expansion 2025). · MEK-inhibitor repurposing in RASopathies (trametinib/selumetinib) — review and case-series literature. · Metformin in schizophrenia — randomized controlled trials and systematic reviews. · Alpelisib in PIK3CA-mutant tumors (approved in breast cancer; explored across PIK3CA-mutant solid tumors). · Tarextumab (anti-Notch2/3) development discontinued after Phase 2 oncology failures. · Simufilam Phase 3 failure (2024) and fraud indictment of its co-developer. · Olaparib synthetic-lethal selectivity for BRCA-deficient cells (PARP-inhibitor mechanism). · NF1 loss as a mechanism of BRAF-inhibitor resistance in melanoma. · Aminoglycoside (streptomycin) treatment of rhinoscleroma; thalassemia–Salmonella susceptibility. · Drug-development cost (~US$2.6B capitalized) and clinical success rates (~10% Phase I→approval); 2026 AI-drug-discovery landscape (~173 clinical-stage AI programs, no fully AI-originated approval yet).


# Part III — The Rejection Ledger {: .part}

*The safety story. Every hypothesis, drug pairing, and safety signal the engine identified as wrong, dangerous, confounded, or unproven — with the reason and the human stakes. Rejecting a bad idea before anyone acts on it is quieter than proposing a good one, but for a system meant to feed real medicine it is at least as important.*

## 1. First, a necessary distinction: "bad lead" is not "bad drug"

This document must open with a clarification, because the phrase "drugs the system proved weren't good" is easy to misread.

The engine did **not** discover that any medicine is bad. Almost every drug named in this ledger is a legitimate, often life-saving therapy: doxorubicin and cytarabine are cornerstone chemotherapies, olaparib is a highly effective cancer drug, metformin is one of the most widely-used and safest medicines on earth. Nothing here overturns that.

What the engine flagged is narrower and more useful: **specific proposed uses, pairings, and signals that are wrong, unproven, or dangerous.** A drug that is excellent for one purpose can be useless — or lethal — when pointed at the wrong disease, combined with the wrong partner, or matched to a gene by name without regard to mechanism. Every "rejection" below is a rejection of a *hypothesis about a drug*, not a verdict on the drug itself. Read every entry as "this **idea** is bad," never "this **medicine** is bad." Where a distinction matters for a real person's safety, this document makes it explicit.

---

## 2. Why the rejections are the safety story

Generating hypotheses is only half of a discovery engine's job. The other half — the half that decides whether the engine is safe to connect to real medicine — is refusing the ones that should not move forward. There are two reasons this matters a great deal.

**The economic reason.** Drug development is brutally expensive: roughly a decade and ~US$2.6 billion of capitalized cost per approved drug, against low-single-digit success odds. The most expensive resource in that pipeline is not the hypothesis — it is the wet-lab time, animal studies, and eventually human trials spent testing it. An engine that can *reject* a plausible-looking but wrong lead before a single experiment is run protects exactly that resource. A correct "no" can be worth more than a speculative "maybe."

**The human reason.** Some wrong leads are not merely wasteful — they are hazardous. A hypothesis engine that produces a confident, evidence-decorated recommendation to give a patient a drug that would harm them is worse than useless. The engine's fail-closed discipline — *when in doubt, block* — exists precisely so that a dangerous idea (like the headline case in Category A) is stopped at the machine, not at the bedside.

The design principle throughout is **fail-closed**: a lead is blocked unless it has affirmatively cleared its evidence, safety, and counter-evidence gates. Missing data is treated as a block, never as a pass. That is why the numbers in this ledger are large: of 1,877 total generated hypotheses, **1,082 were blocked or demoted before ever reaching a human reviewer**, and of 1,750 drug-combination hypotheses, **all 1,750 were blocked.** The following categories break down what was caught and why.

---

## 3. Category A — Mechanistically dangerous: leads that could harm a patient

These are the most important rejections in the entire body of work, because acting on them could hurt someone. In each, the engine's own scoring initially ranked the lead *highly* — the danger is exactly why a downstream rejection discipline is non-negotiable.

### A1. Olaparib for Fanconi anemia — the drug would attack the patient's own cells

**The lead:** Fanconi anemia is (in one genetic subtype) a disorder of biallelic *BRCA2* deficiency. Olaparib is a PARP inhibitor that acts on the BRCA/DNA-repair axis. On a pure target-name basis, the match looked strong enough to rank near the very top of the rare-disease hunt.

**Why it is dangerous:** Olaparib's entire therapeutic logic is *synthetic lethality* — it works in cancer precisely because it **selectively kills cells that are already BRCA-deficient.** In a BRCA2-mutant tumor, that is a feature: the drug kills the cancer and spares normal cells. But a Fanconi anemia patient carries the BRCA2 deficiency in **every cell of their body.** Giving them a drug engineered to kill BRCA2-deficient cells inverts the entire rationale — it would be expected to be broadly toxic, not therapeutic. The same gene name means "exploit this" in cancer and "never exploit this" in the germline disease. An automated system that scores on target overlap cannot, by itself, tell those two situations apart.

**Human impact:** This is the clearest illustration of why a hypothesis engine must never be a consumer-facing recommender. A desperate family reading a confident, well-formatted lead pairing a real cancer drug with their child's rare disease could be catastrophically misled. The value of the engine here is entirely in the *rejection*: the lead is flagged, held behind review, and travels with the warning that it is a computational hypothesis — several of which are wrong or harmful. **Severity: Dangerous. Status: blocked, flagged as a worked example of the failure mode.**

### A2. Dabrafenib for cardiofaciocutaneous syndrome — the right disease, a drug that can make it worse

**The lead:** Cardiofaciocutaneous (CFC) syndrome is a RASopathy driven by over-active RAS/MAPK signaling, frequently through *BRAF*. Dabrafenib is a BRAF inhibitor. The engine paired them — and, tellingly, scored dabrafenib *identically* to the mechanistically-correct MEK inhibitor trametinib for the same disease.

**Why it is wrong:** BRAF inhibitors like dabrafenib can cause **paradoxical activation** of the MAPK pathway in cells with certain upstream RAS/RAF states — the opposite of the intended effect. In a developmental disorder of pathway over-activity, a drug that can *further* activate the pathway is not a benign miss; it is a mechanistically plausible way to make things worse. The engine found the correct disease and the correct pathway but could not distinguish a drug that *dampens* it (trametinib — a genuine research frontier) from one that can *inflame* it (dabrafenib). **Severity: Wrong-direction. Status: separated from the viable sibling lead in review.**

---

## 4. Category B — Backwards: drugs that cause, or worsen, the very condition

A recurring, instructive failure the engine surfaced is the **direction inversion**: a strong, real association between a drug and a condition, where the drug is the *cause* of the condition or a driver of *resistance* — not a treatment. A high association score is direction-blind; catching these is a core rejection function.

| Rejected lead | What the engine "saw" | The actual relationship | Severity |
|---|---|---|---|
| Quinidine → tinnitus | Strong drug–symptom association | Quinidine is **ototoxic** — a known *cause* of tinnitus | Wrong-direction |
| Kanamycin → tinnitus | Strong drug–symptom association | Aminoglycoside ototoxicity — a *cause* of tinnitus/hearing loss | Wrong-direction |
| Streptomycin → tinnitus | Strong drug–symptom association | Aminoglycoside ototoxicity — a *cause* of tinnitus | Wrong-direction |
| Phenytoin → tinnitus | Strong drug–symptom association | Phenytoin toxicity can *cause* tinnitus/ototoxic effects | Wrong-direction |
| NF1-mutant melanoma → vemurafenib | Real gene–drug–cancer association | NF1 loss is a documented mechanism of **resistance** to BRAF inhibitors like vemurafenib | Wrong-direction |

**Why these matter:** Each is a case where the raw data is *correct* — quinidine really is strongly associated with tinnitus — but the clinically meaningful direction is the reverse of a treatment. If an engine surfaced "quinidine for tinnitus" as a lead and a reviewer took the association at face value, the proposal would be to treat a symptom with one of its causes. The rejection discipline (and the engine's own note that these were co-mention associations, not causal treatment claims) is what keeps a strong-but-inverted signal from being read as a therapy.

**Human impact:** Direction-blindness is one of the most general risks of association-based discovery, and this ledger's value is partly in naming it concretely. A downstream user now knows that the engine can rank an ototoxic drug against the very symptom it causes — and that mechanism-direction screening, not raw association strength, is what separates a lead from its mirror image.

---

## 5. Category C — Dead or discredited molecules

Some leads matched a disease's gene to a drug that no longer exists as a viable therapy — because it failed, was withdrawn, or was scientifically repudiated. A target match to a dead molecule is not a lead.

### C1. Tarextumab for CADASIL cerebral arteriopathy

**The lead:** CADASIL is caused by *NOTCH3* mutations. Tarextumab is an anti-Notch2/3 antibody. The names align so cleanly that the engine ranked this **#1 in the entire rare-disease hunt.**

**Why it is not a lead:** Two independent problems. First, tarextumab is a **discontinued** oncology drug — it failed in pancreatic and small-cell lung cancer and is not an available therapy. Second, even mechanistically, whether *blocking* Notch3 helps, harms, or does nothing in a disease of *mutant Notch3 accumulation* in blood-vessel walls is genuinely unknown; CADASIL is not simply "too much Notch3 signaling." A #1 ranking built entirely on nomenclature overlap is the sharpest single demonstration of why the engine's internal score cannot be trusted without a mechanism-and-viability check. **Severity: Dead-drug + unproven mechanism. Status: blocked.**

### C2. Simufilam for Melnick–Needles syndrome

**The lead:** Melnick–Needles syndrome is an *FLNA* (filamin A) disorder. Simufilam was promoted as a filamin-A–modulating drug, so the target names line up.

**Why it is not a lead:** Simufilam is the **scientifically discredited** Cassava Sciences Alzheimer's candidate. It failed its Phase 3 trials in 2024, and its development was shadowed by a federal fraud indictment of its co-developer over manipulated research data. A target match to a repudiated, fraud-tainted molecule carries negative value — pursuing it would chase a compound the scientific community has already rejected on integrity grounds. **Severity: Discredited drug. Status: blocked.**

---

## 6. Category D — The confounded safety signal, correctly caught

This is the ledger's best example of the engine catching a *statistical* trap rather than a mechanistic one — and it is worth telling in full, because it is exactly the kind of false alarm that naive data-mining produces and confident systems get wrong.

**The signal:** In an adverse-event database screen, the pair **metformin + trametinib** threw a serious safety signal — a real, serious report in the FDA's public FAERS adverse-event system, with reactions including a lower-gastrointestinal hemorrhage. A naive reading would conclude: "these two drugs together cause dangerous bleeding — flag the combination as unsafe."

**Why that reading is wrong:** The engine pulled and dissected the actual underlying case report instead of trusting the aggregate signal. What it found:

- The report listed **19 concomitant drugs**, not two — a classic polypharmacy case.
- Metformin and trametinib were present but only as **concomitant** medications, not the reported cause.
- The **primary suspect drug was the anticoagulant apixaban (Eliquis)** — a blood thinner whose association with gastrointestinal bleeding is well established and entirely sufficient to explain the reported reaction.

The engine therefore classified the signal as **confounded** — a serious event correctly attributed to an anticoagulant in a many-drug report, *not* evidence that metformin and trametinib interact dangerously — and kept the pair blocked pending human review rather than either clearing it or falsely condemning it.

**Human impact:** This cuts both ways, and both are valuable. A system that *manufactured* a metformin–trametinib interaction warning from this report would inject a false safety scare into the literature. A system that *ignored* the signal entirely would be reckless. The engine did neither: it isolated the confounder, named apixaban as the real primary suspect, explained the polypharmacy structure, and held the pair for review. That is precisely the judgment that protects both patients (from a fabricated warning) and researchers (from wasting time chasing a phantom interaction). **Severity: Confounded signal, correctly demoted. Status: blocked with mechanistic explanation.**

---

## 7. Category E — Drug-pair adverse-interaction signals

Separately, a screen against a large curated drug-drug interaction resource surfaced seven drug pairs carrying elevated adverse-event association ratios (a proportional reporting-ratio, or PRR, is how strongly two drugs co-occur with adverse reports relative to chance). These are **not** proposed therapies — they are *interaction warnings*: pairs that the data suggests may not be safe together. The engine kept every one blocked, and — importantly — refused to treat the raw statistical signal as confirmed without independent corroboration.

| Drug pair | Adverse-signal strength (max PRR) | Independent confirmation? | Status |
|---|---:|---|---|
| Saxagliptin + sitagliptin | 60 | none found | blocked, unconfirmed |
| Etanercept + ribavirin | 40 | none found | blocked, unconfirmed |
| Infliximab + sitagliptin | 40 | none found | blocked, unconfirmed |
| Sitagliptin + valacyclovir | 40 | none found | blocked, unconfirmed |
| Metformin + trametinib | 40 | one FAERS report — confounded (see Category D) | blocked, confounded |
| Etanercept + sitagliptin | 30 | none found | blocked, unconfirmed |

**What the engine did right here:** For six of the seven pairs, the interaction signal existed *only* in the statistical resource, with **no independent confirmation** from regulatory labels or literature — so the engine explicitly marked them "signal present, not independently confirmed, still blocked" rather than promoting them to warnings. A note on the top row: saxagliptin and sitagliptin are *both DPP-4 inhibitors*, so a co-reporting signal for the two together most plausibly reflects redundant/duplicative prescribing rather than a novel toxic interaction — another reason the raw ratio needed context, not blind trust.

**Human impact:** Adverse-event databases are noisy; unfiltered, they generate false interaction scares that can frighten patients off useful drugs. The engine's discipline — surface the signal, demand independent confirmation, block until you have it — is the correct posture for turning noisy pharmacovigilance data into something a human can safely review.

---

## 8. Category F — High-risk and unknown-safety drugs, held for review

Every drug-bearing therapeutic lead the engine produced was automatically blocked from promotion until a safety review completed. Two sub-groups are worth naming.

**F1. Real drugs with serious known toxicity, flagged for mandatory review.** These are essential medicines — nobody is calling them "bad" — but the engine correctly attached their known danger profile so no lead using them could be treated as casually safe:

| Drug (in an oncology lead) | Known risk profile the engine attached |
|---|---|
| Doxorubicin | Boxed warning; contraindications; serious/fatal adverse reports |
| Cytarabine | Boxed warning; contraindications; serious/fatal adverse reports |
| Sirolimus | Boxed warning; contraindications; serious/fatal adverse reports |
| Metformin | Boxed warning (lactic acidosis); contraindications; large adverse-report volume |
| Gemcitabine | Contraindications; warnings; serious/fatal adverse reports |
| Vemurafenib | Contraindications; interactions; serious/fatal adverse reports |
| Selumetinib | Contraindications; interactions; serious/fatal adverse reports |
| Trametinib / mirdametinib | Contraindications; warnings; serious adverse reports |

The point is not that these drugs are disqualified — it is that the engine refused to let a hypothesis using a boxed-warning chemotherapy be ranked as though it were harmless. Each is held under a standing rule: no clinical promotion until a safety review is complete.

**F2. Research compounds with *no* safety data — blocked because nothing is known.** Three leads relied on early research chemicals with no FDA label and no adverse-event record at all: **AZ628**, **JQ1 (a BET-bromodomain tool compound)**, and **VTX-11e**. The engine did not treat "no data" as "no risk." It failed these closed — *safety source unavailable* — precisely because an absence of safety information is itself a reason to stop, not a green light. **Severity: Unknown-safety. Status: blocked, fail-closed.**

**Human impact:** The uniform rule — every drug lead blocked until safety is reviewed, and unknown safety treated as a block — is what keeps the engine from ever silently implying that a hypothesis is safe to try. For a tool meant to feed real medicine, "we don't know yet, so stop" is the ethically correct default.

---

## 9. Category G — The systematic falsification sweep: 1,082 leads blocked

Beyond the marquee cases, the engine ran a fail-closed counter-evidence sweep across all 1,877 generated hypotheses. The outcome is the quantitative backbone of this ledger:

| Outcome | Count | Meaning |
|---|---:|---|
| **Blocked or demoted before human review** | **1,082** | Failed a required-evidence or counter-evidence gate |
| Blocked — missing required evidence/safety | 1,066 | Fail-closed: needed trial or safety data was absent |
| Demoted — hard counter-evidence found | 16 | The literature/data actively *contradicted* the lead |
| Demoted — an associated clinical trial was **stopped** | 1 | A halted trial is a real-world negative signal |
| Passed the sweep with no counter-evidence (still not cleared) | 795 | No contradiction found — but *not* proof of correctness |

The dominant blockers, by reason:

- **~1,028 leads** blocked because no clinical-trial evidence existed for the proposed drug–disease pairing.
- **~991 leads** blocked because required safety data was missing (fail-closed).
- **~90 leads** blocked because the drug carried a high-risk/boxed-warning label section.
- **16 leads** actively contradicted by counter-evidence, and **1** attached to a stopped trial.

**Two honest points about these numbers.** First, most of the 1,082 were blocked for *missing* evidence, not *disproving* evidence — the engine is saying "not shown," not "shown false." That is the correct, conservative reading. Second, the 795 that passed with no counter-evidence are **not** validated; "we found nothing against it in the current sources" is a long way from "it works." The sweep's job is to *remove* the clearly-unsupported, not to bless the remainder.

**Human impact:** This is triage at a scale no human could do by hand — nearly 1,900 hypotheses each checked against literature, trials, drug-gene evidence, target data, and safety records, with a documented reason for every block. It means a human reviewer inherits a worklist that has already had its clearly-unsupported and actively-contradicted members filtered out, each with the receipts for *why*.

---

## 10. Category H — The combination miner: all 1,750 pairs blocked

The drug-combination miner generated 1,750 ranked drug-pair hypotheses — and blocked **every single one.** This is not a failure of the miner; it is the fail-closed rule working at full strength. A combination cannot be promoted unless *all three* of component safety, pair-interaction evidence, and external synergy evidence are present, and the available safety substrate covered only a handful of the drug components.

| Blocking reason | Pairs affected |
|---|---:|
| A component was already blocked/demoted upstream | 1,750 (all) |
| Pair-interaction evidence missing (fail-closed) | ~1,745 |
| External synergy evidence missing (fail-closed) | ~1,703 |
| Component safety data missing (fail-closed) | ~1,727 |
| Overlapping component safety flags — review required | 17 |

Even the top-ranked "hypotheses" here are instructive rejections. The four highest — metformin + sitagliptin, + saxagliptin, + alogliptin, + linagliptin, all for type 2 diabetes — are combinations **already sold as fixed-dose products**; the miner rediscovered marketed pills rather than proposing anything new, and blocked them anyway for missing its evidence gates. Only a preclinical-synergy cross-check added any outside support: 28 pairs matched a cell-line synergy database, the sole rows with independent (and merely *preclinical*, non-clinical) corroboration.

**Human impact:** Drug combinations are where danger multiplies — two individually-tolerable drugs can interact badly, and the space of possible pairs is enormous. An engine that generates 1,750 combination ideas and then *refuses to promote any of them without safety and interaction evidence* is modeling exactly the caution the domain demands. The 1,750 are best read not as recommendations but as a **worklist that names, for each pair, precisely which evidence must be gathered before anyone considers it.**

---

## 11. What the rejection discipline means for humanity

Step back from the individual cases and the shape of the contribution becomes clear.

**A discovery engine is only as trustworthy as its "no."** It is easy to build a system that spits out plausible-sounding drug ideas; the value — and the safety — is entirely in whether it can also recognize which of its own ideas are wrong, unproven, or dangerous, and say so plainly. This engine caught a drug that would attack a patient's own cells (olaparib/Fanconi), drugs pointed at symptoms they actually cause (the ototoxic "tinnitus treatments"), dead and discredited molecules ranked #1 by name overlap (tarextumab, simufilam), a scary-looking safety signal that was really an anticoagulant in disguise (metformin/trametinib), and roughly 1,082 of its own hypotheses that could not clear their evidence gates. Every one of those is a mistake that *did not* propagate downstream.

**It protects the two scarcest resources in medicine: lab time and patient safety.** Every correct rejection is wet-lab time not wasted on a doomed lead and — in the dangerous cases — a harmful idea stopped at the machine rather than the bedside. Against a backdrop where a single drug program costs ~US$2.6 billion and a decade, and where naive data-mining routinely produces false safety scares, a *systematic, explainable, fail-closed* rejection layer is a real, if unglamorous, contribution.

**It makes automated discovery honest.** The engine never claimed efficacy, never treated missing data as safety, and attached a documented reason to every block. That transparency is the opposite of a black box: a human expert can read *why* each lead was rejected and overrule it if they disagree. In an era of confident AI systems that rarely show their reasoning, an engine whose most-repeated output is a well-justified "not yet, and here is exactly what's missing" is modeling the right behavior.

The realistic statement of value is therefore symmetrical to the hypothesis catalog. There, the message was "AI fills the funnel; humans empty it." Here it is the necessary complement: **AI can also help guard the funnel — catching the wrong, the backwards, the dead, and the dangerous before a human, or a patient, acts on them.** For a technology meant to touch real medicine, that guarding function is not a lesser one than the generating; it is part of what would make the generating safe to use at all. None of it removes the need for expert review — the dangerous cases in this ledger were caught by mechanistic reasoning applied *to* the engine's output, not by the engine's own scoring (§12) — but a screening layer that filters the clearly-wrong before a human ever sees it is a sensible and useful default.

---

## 12. Honest limits of this ledger

- **The engine's rejections are as bounded as its knowledge.** A lead marked "no counter-evidence found" only means none was found *in the current sources*; a broader search could contradict it. Absence of a rejection is not a clearance.
- **Missing-evidence blocks are not disproofs.** The overwhelming majority of the 1,082 blocked leads failed for *absent* data, not *contradicting* data. Many could become viable once the missing trial or safety evidence exists. Blocked ≠ wrong.
- **The dangerous cases were caught by human-supplied mechanistic reasoning applied to the output, not by the engine's raw scoring.** The engine *ranked* olaparib/Fanconi near the top; it is the rejection layer and expert review that flag it. The scoring function itself still needs mechanism-direction, loss-vs-gain, and germline-vs-somatic screens it does not yet have. The rejections in this ledger are the argument *for* adding them.
- **These are computational judgments, not clinical rulings.** Nothing here is medical advice. Some entries — the ototoxic drugs, olaparib/Fanconi — are included specifically as examples of ideas that must be rejected, and none should be read as guidance to do anything to a patient.
- **The severity and reason labels are interpretive.** The raw block statuses and counts come from the engine's own outputs; the plain-language "why it's wrong" and severity tags are my reading of them against the published science.

### Sources consulted for the mechanism and status judgments

Olaparib synthetic-lethal selectivity for BRCA-deficient cells (PARP-inhibitor mechanism). · BRAF-inhibitor paradoxical MAPK activation in RAS/non-V600 states; MEK inhibition as the mechanistically-correct RASopathy strategy. · NF1 loss as a mechanism of BRAF-inhibitor resistance in melanoma. · Aminoglycoside (streptomycin, kanamycin) and quinidine/phenytoin ototoxicity as *causes* of tinnitus/hearing loss. · Tarextumab (anti-Notch2/3) discontinuation after Phase 2 oncology failures. · Simufilam Phase 3 failure (2024) and fraud indictment of its co-developer. · Apixaban (Eliquis) association with gastrointestinal bleeding. · FDA boxed-warning / adverse-event context via openFDA drug label and FAERS. · Drug-development cost (~US$2.6B capitalized) and clinical success rates.
