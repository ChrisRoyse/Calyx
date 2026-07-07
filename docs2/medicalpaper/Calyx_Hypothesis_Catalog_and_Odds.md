# The Calyx Hypothesis Catalog: Every Named Lead, What Makes It Distinctive, and the Odds It Matters

*A companion catalog to the Calyx hypothesis-engine paper. Where the main paper argues the value of generating and pre-screening biomedical hypotheses at scale, this document does the opposite: it goes lead by lead. For every hypothesis the engine surfaced by name, it says what the lead actually is, what — if anything — makes it special or unique, and gives a candid, subjective probability that pursuing it would produce something genuinely useful.*

---

## Table of contents

1. [How to read this catalog](#1-how-to-read-this-catalog)
2. [The probability model, stated honestly](#2-the-probability-model-stated-honestly)
3. [Oncology hunt](#3-oncology-hunt)
4. [Rare-disease hunt](#4-rare-disease-hunt)
5. [Neurodegeneration / neuropsychiatric hunt](#5-neurodegeneration--neuropsychiatric-hunt)
6. [Infectious / immunology / inflammation hunt](#6-infectious--immunology--inflammation-hunt)
7. [Metabolic / cardiovascular / renal hunt](#7-metabolic--cardiovascular--renal-hunt)
8. [The human-review atlas: the leads that rose to the top](#8-the-human-review-atlas-the-leads-that-rose-to-the-top)
9. [Drug-combination hypotheses](#9-drug-combination-hypotheses)
10. [The portfolio view: what the whole set is worth](#10-the-portfolio-view-what-the-whole-set-is-worth)
11. [Honest caveats on every number in this document](#11-honest-caveats-on-every-number-in-this-document)

---

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
