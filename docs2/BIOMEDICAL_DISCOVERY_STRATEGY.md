# Calyx Biomedical Association-Mining & Discovery Strategy

**Status:** strategy / plan (2026-06-24). Author: AI agent, grounded in the Calyx engine docs
(`docs2/calyxdocs/10,11,12,16,17`) and external literature-based-discovery research (cited §8).
**Goal:** use the Calyx kernel + Oracle + Loom over the biomedical corpus to surface
**undiscovered cross-domain associations** — and ideally a drug-repurposing or mechanism
hypothesis no human has yet connected — with every accepted step **grounded** (sufficiency-proven),
so a 100-hop research chain stays trustworthy.

> **Honesty contract (the whole point).** Calyx is not a magic oracle. Its guarantee is the
> **honesty gate**: every Oracle answer either clears `I(panel;outcome) ≥ H(outcome)` or returns
> `Insufficient` with a per-sensor bit deficit. We therefore **only ever build the chain on
> answers that cleared the gate**; a refusal is not a dead end, it tells us exactly which lens is
> short and by how many bits. That is what makes "follow the answers, they're correct" valid —
> we discard everything ungrounded. Do not overstate this; do not chain on a refusal.

---

## 1. The real Calyx discovery surface (what actually exists, per the code)

| Capability | What it does (grounded in code) | Source doc |
|---|---|---|
| **Loom agreement graph** | For each constellation, cosine of every slot-pair → an undirected weighted edge list over the 14 lenses. Cross-terms: agreement (cosine), delta, interaction (Hadamard), concat. | 10 §2–4 |
| **Loom blind-spot detector** | Flags `(cx, lens_a, lens_b)` where `lens_a_similarity − lens_b_neighbor_mean ≥ 0.5` — i.e. one lens sees a strong association another lens's neighborhood denies. **This disagreement is the novelty signal.** | 10 §6 |
| **Loom temporal lead/lag** | Over recurrence series, median signed Δt between co-occurring events → "A precedes B by N s" (directional/causal hint). | 10 §8.1 |
| **Kernel (Lodestar)** | Over a *directed association graph* (built from Loom cross-terms + anchors): SCC condense → Brandes betweenness → top-fraction kernel graph (≈10%) → DFVS approx → **members (≈1% MFVS)**. Scored by `0.40·degree + 0.40·betweenness + 0.20·groundedness`. | 12 §2 |
| **`kernel_answer`** | Route a query to its nearest **anchored** kernel node, then walk association edges to the target, scoring each hop `edge_weight · 0.9^hop`. The grounded answer *path*. | 12 §3.2 |
| **`bridges(scope_a, scope_b)`** | Members present in **both** scoped kernels — constellations that ground two domains at once. **This is a literal Swanson B-term finder.** | 12 §5.2 |
| **Spectral (Fiedler)** | Laplacian eigenmaps; 2nd eigenvector sign bisects latent communities; `spectral_gap` = connectivity. Structure-only candidate proposer. | 17 §6.2 |
| **Label propagation** | Harmonic extension: clamp kernel-node labels, diffuse to the rest with `exp(−λ·hops)` confidence. Spreads a grounded label across the graph. | 12 §6.2 |
| **`reach` / `reach_scored`** | Bidirectional-BFS shortest path; weighted best-first walk (product of edge weights × 0.9^hop). The raw graph-walk primitive under the chains. | 17 §B.4 |
| **Oracle honesty gate** | `sufficient = panel_bits ≥ anchor_entropy_bits`; else `Insufficient{per_sensor_deficit}`. The accuracy guarantee. | 16 §3 |
| **Oracle `reverse_query`** | Outcome → causes, depth-3 backward chain (cause → cause-of-cause), grounded vs provisional, confidence `n/(n+1)`. | 16 §7 |
| **Oracle `butterfly`/predict** | Action → predicted outcome (recurrence posterior) + consequence tree, depth ≤4, hop attenuation ×0.7, min-confidence 0.05. | 16 §4–5 |
| **Oracle `super_intelligence`** | 6-tier readiness predicate (OracleClean, PanelSufficient, KernelExists, Calibrated, GoodhartDefended, MistakeClosed) — tells us *whether a domain is ready to be trusted*. | 16 §8 |
| **Assay `bits` / sufficiency** | KSG mutual-information `I(panel; anchor)` in bits, per-lens attribution, `n_eff`, DPI ceiling. The grounding math. | 11 |
| **Search fusions** | `kernel-first`, `rrf`, `weighted-rrf`, `single-lens`, `pipeline` — 14 lenses × 5 fusions = many distinct "views" of the same query. | 09 / CLI |

**Prerequisites the corpus does NOT yet satisfy (the work to be done — be honest):**
1. **Anchors.** Grounding (kernel groundedness, Oracle gate's `AnchorKind::Reward` assay rows) needs typed anchors. Our QA rows carry the correct answer *in the text* but attach no `anchors`. → **Anchors-at-ingest (the QA label is the natural anchor).**
2. **A materialized association graph.** The kernel consumes a `calyx_paths::AssocGraph` derived from Loom cross-terms. Loom cross-terms are **not yet woven** for the corpus. → **Weave the agreement graph + materialize cross-terms.**
3. **Oracle recurrence metadata.** `oracle_predict`/`reverse_query` match on `oracle.domain` / `oracle.action` / `outcome_anchor` metadata and recurrence series — absent in our corpus. → either (a) structure QA as domain=specialty, action=question, outcome=answer, or (b) drive discovery primarily through the **kernel + Loom graph** path (which needs only anchors + cross-terms), and use the Oracle gate as the *sufficiency filter* rather than the recurrence predictor.

> **Strategic consequence:** the fastest path to real discovery is the **kernel + Loom-graph + bits-sufficiency** route (needs anchors + cross-terms), not the Oracle-recurrence route (needs event/time structure we don't have). Use Oracle's honesty gate + `super_intelligence` as the trust layer on top.

---

## 2. The core method — grounded iterative discovery chains (the "100 questions" walk)

This operationalizes the operator's vision and **is** Swanson literature-based discovery (§8) realized on Calyx.

**Closed discovery (test a specific A↔C link):** given a disease A and a candidate drug/target C,
find the intermediate B that connects them. Calyx-native: `bridges(scope=A, scope=C)` returns the
constellations grounding both → candidate B-terms; each B is a sufficiency-checkable hypothesis
"A —(B)— C".

**Open discovery (find any novel C for a starting A):** from A, walk `reach_scored` /
`kernel_answer` out along high-agreement edges, at each hop keep only nodes whose
`bits`-sufficiency clears the gate, and flag hops that **cross domains** (clinical→molecular,
finance→clinical, legal→clinical) — cross-domain, gate-passing, high-weight edges between
previously-disjoint concepts are the undiscovered-public-knowledge candidates.

**The chain loop (one "question"):**
```
state = { frontier concept(s), accumulated grounded facts, visited set }
1. PROBE   — query the frontier concept N ways (vary fusion + phrasing + lens-emphasis, §4G)
2. GATE    — for each candidate association, compute bits-sufficiency / Oracle gate.
             Keep only gate-PASS results. Record refusals + their per-sensor deficit (these
             tell us what evidence/lens is missing — a research lead in itself).
3. NOVELTY — score each surviving association for novelty (cross-domain? blind-spot
             disagreement? not co-mentioned in source provenance? long graph distance but high
             agreement?). Rank by novelty × grounded-confidence.
4. STEP    — the top-ranked grounded, novel association becomes the next frontier concept.
             Append to the chain; "always go where the (grounded) answer takes us."
5. REPEAT  — ~100 hops, branching into a tree when several strong leads tie.
```
Each accepted hop is **sufficiency-proven**, so the 100th question rests on 99 grounded steps.
The chain terminates a branch when (a) the gate refuses with no cheap fix, (b) novelty collapses
(we've circled back), or (c) a hypothesis reaches a testable A→B→C with high grounded confidence
across ≥2 independent lens families → **flag for human/wet-lab review** (Swanson always required
experimental confirmation; so do we).

---

## 3. Why this can find what humans missed (the mechanism, honestly)

- **Embedder diversity is the search engine.** 14 lenses (clinical, scientific, finance, legal,
  code, multilingual, general, lexical) embed the *same* text into 14 geometries. An association
  invisible in clinical space can be obvious in, say, the legal or finance or code geometry. The
  **cross-lens disagreement** (Loom blind-spot, §1) is precisely "one viewpoint sees a link the
  others don't" — the algorithmic analogue of an interdisciplinary insight.
- **Disjoint-literature bridging.** `bridges` + long-graph-distance-but-high-agreement edges find
  A and C that are never co-mentioned in the source provenance yet are strongly associated through
  shared B structure — Swanson's "undiscovered public knowledge."
- **Grounding keeps it honest.** Naïve embedding similarity hallucinates; the bits-gate refuses
  associations the panel can't actually carry, so we don't chase noise. This is the key advantage
  over plain cosine-similarity repurposing pipelines (§8) which have no sufficiency filter.

---

## 4. The full tactic library (try all of these; sweep systematically)

### A. Anchoring & grounding tactics
- **QA-label anchors:** attach `anchor kind=label:<answer>` (or `test-pass` for verified-correct)
  to each constellation at ingest. This grounds the kernel and unlocks the Oracle gate.
- **Multi-anchor:** also anchor by medical specialty / dataset / claimed-fact, so we can build
  per-domain scoped kernels and run `bridges` between specialties.
- **Synthetic known-signal calibration first:** plant a known association (e.g. a constellation
  pair with identical content → agreement 1.0; a known drug–disease pair) and confirm `bits` and
  the gate recover it (agreement→1.0, bits≥H). Never trust a discovery run whose calibration
  fails. (This is the doctrine's FSV requirement.)

### B. Association-graph construction (the substrate)
- Weave Loom cross-terms across all 14 lenses for the corpus; materialize **agreement** eagerly,
  promote **interaction** cross-terms whose pair-gain ≥ 0.05 bits.
- Build the directed `AssocGraph` (agreement × directional-confidence edge weights) and the kernel
  on it; inspect `groundedness_fraction` (must be > 0 — else add anchors).

### C. Cross-domain bridge mining (Swanson B-terms)
- For every pair of domain scopes (specialty×specialty, clinical×molecular, clinical×finance/legal),
  run `bridges(scope_a, scope_b)`; rank bridge members by frequency/centrality. Each is a
  candidate mechanistic link. Closed-discovery seed list.

### D. Blind-spot / disagreement mining (richest novelty)
- Sweep the Loom blind-spot detector; collect High-severity `(cx, lens_a, lens_b)` triples. For
  each, read the text and the two lenses' nearest neighbors — the disagreement often encodes a
  cross-domain analogy (the FieldSHIFT mechanism, §8). Gate each before believing it.

### E. Temporal / directional tactics
- Where recurrence/time exists, use lead/lag to orient edges (A→B if A precedes B). For static QA,
  approximate direction via citation/derivation structure if available; otherwise treat edges as
  undirected hypotheses and let the gate + human review assign direction.

### F. Spectral community discovery
- Run Laplacian-eigenmap Fiedler bisection on the agreement graph to expose latent clusters;
  bridges *between* spectral communities are high-value cross-domain candidates. Use eigenvector
  centrality as an independent candidate proposer (then ground with the kernel).

### G. Search & prompting variation (probe each concept many ways — the operator's "vary style/length")
Build a **probe matrix** per frontier concept and run the full cross-product:
- **Fusion mode:** `kernel-first` (grounded funnel) · `rrf` · `weighted-rrf` · `single-lens` (probe
  one geometry at a time — run all 14) · `pipeline`.
- **Phrasing:** terse term · full clinical question · mechanistic framing ("via what pathway…") ·
  analogical framing ("what in domain X resembles…") · negation/contrast ("what contradicts…").
- **Length:** single entity · short phrase · full paragraph context. (Different lenses peak at
  different input lengths; vary to excite different lenses.)
- **Lens-emphasis:** weighted-RRF with weight mass shifted onto each lens family in turn, so the
  same query is "seen" from clinical-heavy, scientific-heavy, finance/legal-heavy stances.
- Record which (fusion × phrasing × lens) combinations surface associations the others miss —
  those combinations are themselves a learned asset (log them; reuse the productive ones).

### H. The Oracle chain (when domain/event structure is added)
- `reverse_query(outcome)` to backward-chain causes; treat each grounded cause as a new outcome and
  recurse beyond the built-in 3-hop limit (re-grounding each external hop). `butterfly` forward for
  consequences. Use `super_intelligence(domain)` to confirm a domain is trustworthy (all 6 tiers)
  before mining it hard.

### I. Self-evaluation & multi-perspective (from the LLM-hypothesis research, §8)
- After the graph/kernel surfaces a candidate A–B–C, run an **LLM evaluation pass** (RAG over the
  grounded provenance abstracts, SKiM-GPT style): "given these grounded facts, is A–B–C plausible,
  novel, and testable? what would falsify it?" Keep a transparent score + justification + the
  retrieved evidence. Multiple independent LLM "lenses" (diverse prompts/temperatures) increase
  hypothesis diversity and catch the implausible ones.

---

## 5. The optimal end-to-end strategy (phased, recommended)

**Phase 0 — Calibrate (must pass before any discovery is believed).**
Plant known signals; confirm `bits`/gate recover them; confirm a fresh small kernel grounds
(`grounded_fraction>0`). FSV against stored artifacts.

**Phase 1 — Prepare the substrate.**
(1) Re-ingest (or backfill) the ~199k corpus with **QA-label + specialty anchors** (hours at
batch=4, not days). (2) Weave Loom cross-terms across the 14 lenses. (3) Build the grounded kernel;
read groundedness + recall (`kernel_recall_test`, gate ≥0.95).

**Phase 2 — Static mining sweep (breadth).**
Run D (blind-spots), C (all domain-pair bridges), F (spectral communities + inter-community bridges)
to produce a ranked **candidate-association pool**, each gate-checked. This is the raw discovery
surface.

**Phase 3 — Grounded chain walks (depth).**
Seed chains from the highest-novelty gate-passing candidates and from operator-supplied disease/target
questions. Run the §2 loop with the §4G probe matrix, ~100 hops, branching. Every accepted hop is
sufficiency-proven; log the full provenance chain.

**Phase 4 — Hypothesis evaluation & ranking.**
For surviving A–B–C chains: §4I LLM self-evaluation with retrieved grounded evidence; rank by
(novelty × grounded-confidence × cross-domain-distance × evaluator-plausibility). Output a ranked,
fully-traceable hypothesis list — never a verdict. Flag the top candidates for human / wet-lab review.

**Phase 5 — Iterate & expand the corpus.**
Refusals with cheap fixes (per-sensor deficit) tell us which lens/evidence to add; add it, re-ground,
re-mine. Bring in the discovery vault (protein ESM2 + molecule ChemBERTa + DNA ModernGENA, anchored
on ChEMBL/BindingDB affinity) to extend chains into molecular space — that is where a concrete
drug/target hypothesis becomes testable.

---

## 6. Build tasks this requires (engineering, honest)
1. **Anchors-at-ingest** — thread typed anchors through the streaming ingest (same pattern as the
   provenance fix already shipped; the `anchors` field is base-CF-encoded). Highest priority.
2. **Loom weave/materialize over the corpus** — produce the agreement graph + cross-term CF.
3. **A discovery harness/agent** — implements the §2 chain loop + §4G probe matrix + §4I evaluation,
   logging every gated hop with provenance. (This is the "string of 100 questions" engine.)
4. **(Optional) Oracle event/domain structuring** — map QA into domain/action/outcome + recurrence
   to unlock `predict`/`reverse_query`/`super_intelligence` on the recurrence path.
5. **Degraded-flag fix** — compute `degraded` ignoring always-Absent temporal slots, so it reflects
   real content-lens failures during mining (SITREP §7 note).

---

## 7. Research grounding (the methods we're standing on)
- **Swanson literature-based discovery / ABC model & "undiscovered public knowledge"** — open vs
  closed discovery; two-node search (Smalheiser); A–B–C path hypotheses requiring experimental
  confirmation. (Swanson 1986/1997; Smalheiser 2017; PLOS One context-ABC 2019.) → our §2.
- **Embedding-based drug repurposing** — cosine/temporal-shift over PubMed embeddings reveals hidden
  drug–disease links (PubDigest CTEPH→riociguat; word2vec repurposing; TxGNN foundation model;
  DMAPLM dual-encoder contrastive). → our §3 mechanism + §5 Phase 5 molecular extension.
- **LLM hypothesis generation & cross-domain analogy** — FieldSHIFT domain translation; analogical
  reasoning (extract→search); multi-agent + tool-use raises diversity; self-evaluation raises novelty
  (BioVerge); RAG hypothesis scoring with transparent justification (SKiM-GPT, κ=0.84 vs experts). →
  our §4G (varied probing), §4I (self-evaluation).
- **The Calyx differentiator vs all of the above:** the **sufficiency/honesty gate** — none of the
  cited pipelines refuse-when-uncertain; Calyx does, which is what lets us chain 100 deep without
  compounding hallucination.

---

## 8. Honest risks & limits
- **Garbage-in:** associations are only as good as the corpus + the gate's calibration. Phase 0 is
  non-negotiable; a mis-calibrated estimator (issue #806 work) would let weak signals masquerade as
  grounded — verify the power-calibration gate is active.
- **Anchors define "grounded."** If the QA-label anchor is noisy, groundedness is noisy. Use
  verified/`test-pass` anchors where possible.
- **No causation, only association + direction hints.** Every output is a *ranked, traceable
  hypothesis*, never a verdict or a clinical recommendation. A discovered link "can't lie to a
  doctor" precisely because it carries its full grounded provenance chain and a sufficiency proof —
  but it still requires experimental validation before it is knowledge.
- **Compute:** the static sweep (blind-spots over 199k × C(14,2), all domain-pair bridges) and the
  chain walks are real GPU/CPU work; budget for it, and remember the GPU-exclusivity rule (no vault
  command during an ingestion).
- **Scope:** the current corpus is ~199k clinical-QA rows (no meditron, no miriad). It is a curated
  substrate good for proving the method; the biggest undiscovered-association yield will come after
  Phase 5 corpus expansion into full literature + molecular space.

---

*Bottom line:* the optimal strategy is **anchor → weave the cross-lens association graph → ground a
kernel → mine blind-spots/bridges/spectral-communities for candidates → walk grounded 100-hop chains
that always follow gate-passing answers → evaluate surviving A–B–C hypotheses with retrieved evidence
→ rank and hand the best to human/wet-lab review → expand the corpus where refusals point.** Every
step is sufficiency-proven, every claim is traceable, and the cross-lens disagreement is the engine
that can surface what a single-discipline human view has missed.
