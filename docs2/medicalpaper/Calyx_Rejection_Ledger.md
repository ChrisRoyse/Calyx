# The Rejection Ledger: Every Lead the Engine Ruled Out, Why It Was Wrong, and Why That Matters for People

*A companion to the Calyx hypothesis-engine paper and hypothesis catalog. Those documents cover what the engine proposed. This one covers what it refused — every hypothesis, drug pairing, and safety signal the engine identified as wrong, dangerous, confounded, or unproven, together with the reason and the human stakes. Rejecting a bad idea before anyone acts on it is quieter than proposing a good one, but for a system meant to feed real medicine it is at least as important.*

---

## Table of contents

1. [First, a necessary distinction: "bad lead" is not "bad drug"](#1-first-a-necessary-distinction-bad-lead-is-not-bad-drug)
2. [Why the rejections are the safety story](#2-why-the-rejections-are-the-safety-story)
3. [Category A — Mechanistically dangerous: leads that could harm a patient](#3-category-a--mechanistically-dangerous-leads-that-could-harm-a-patient)
4. [Category B — Backwards: drugs that cause, or worsen, the very condition](#4-category-b--backwards-drugs-that-cause-or-worsen-the-very-condition)
5. [Category C — Dead or discredited molecules](#5-category-c--dead-or-discredited-molecules)
6. [Category D — The confounded safety signal, correctly caught](#6-category-d--the-confounded-safety-signal-correctly-caught)
7. [Category E — Drug-pair adverse-interaction signals](#7-category-e--drug-pair-adverse-interaction-signals)
8. [Category F — High-risk and unknown-safety drugs, held for review](#8-category-f--high-risk-and-unknown-safety-drugs-held-for-review)
9. [Category G — The systematic falsification sweep: 1,082 leads blocked](#9-category-g--the-systematic-falsification-sweep-1082-leads-blocked)
10. [Category H — The combination miner: all 1,750 pairs blocked](#10-category-h--the-combination-miner-all-1750-pairs-blocked)
11. [What the rejection discipline means for humanity](#11-what-the-rejection-discipline-means-for-humanity)
12. [Honest limits of this ledger](#12-honest-limits-of-this-ledger)

---

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
