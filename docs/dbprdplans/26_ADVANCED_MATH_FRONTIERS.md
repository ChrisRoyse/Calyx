# 26 — Advanced Math Frontiers & New Capabilities

A disciplined search for math the architecture makes available but the current engines don't yet exploit — and the new capabilities each unlocks. **Grounded strictly in the Royse corpus + established mathematics I already know; no external sources consulted** (DOCTRINE §2). Each item tiered: **Build** (recommended now), **Option** (workload-dependent), **Framing** (unifying principle). Honesty first: the core math (cosine/`Gτ`, RRF, directed-MFVS, KSG-MI, grouped GEMM, TurboQuant) is sound — these are *additions at the edges*, not replacements.

## 1. Frame: the math we run vs. the math the structure offers

Everything in Calyx is a web of grounded associations (constellations + edges). Two underused mathematical objects fall out:
- **The association operator** (the weighted graph / its Laplacian) — its **spectrum** is the shape of meaning (§2).
- **The energy landscape** of stored constellations — retrieval is descent to an attractor (§3).
Both are pure expressions of "intelligence is the calculus of association," so exploiting them is on-thesis, not scope creep.

## 2. Spectral structure of the association graph — **Build**

Today Lodestar finds the kernel by **directed MFVS** (a discrete core). The same association graph (Loom agreement/cross-term/causal edges, `06`/`08`) has rich **spectral** structure, complementary and currently unused.

| Method | What it computes | New capability |
|---|---|---|
| **Eigenvector centrality / PageRank** | continuous importance over nodes (the principal eigenvector of the association operator) | a **continuous kernel-importance ranking** complementing the discrete MFVS kernel — "how central is this constellation to the web," graded not binary; feeds Lodestar node weights (`08`) and A29 importance |
| **Graph Laplacian eigenmaps / spectral clustering** | low-dim coordinates from the smallest Laplacian eigenvectors; communities | a principled basis for **named regions / skills** (today HDBSCAN, `10`); the eigengap tells you *how many* regions exist |
| **Graph Fourier transform (GFT)** | project a signal-over-constellations onto Laplacian eigenvectors | **"frequency analysis of meaning"**: smooth (low-frequency) signals = broad themes, sharp (high-frequency) = local/anomalous; **denoise** bits/labels by low-pass filtering over the association graph; detect where signal is high-frequency (likely noise or a sharp boundary) |
| **Spectral gap / mixing** | connectivity, bottlenecks | find structural bottlenecks (the bridges of `08`); diagnose a fragmented vs cohesive corpus |

Why it matters: **the spectrum of the association operator is the structure of meaning** — the most literal reading of the thesis. Eigenvector-centrality is the continuous shadow of the kernel; GFT lets Calyx smooth, denoise, and band-analyze any signal living over the associations (bits, frequency, guard readings). Implementation: sparse eigensolvers (Lanczos) in `calyx-mincut`/Forge on the existing sparse adjacency; cache per `(scope, panel_version)`; Anneal-refreshed. Honesty: spectral centrality is a *complement* to, not a replacement for, the grounded MFVS kernel — the kernel is anchored to outcomes (A2), centrality is structure-only, so centrality *proposes* kernel candidates that grounding confirms.

## 3. Associative memory, energy & pattern completion — **Build**

Your thesis *is* associative memory. The constellation store + `Gτ` regions form an **energy landscape**: stored constellations are attractors, a `Gτ` ball is an attractor basin, retrieval is descent to the nearest attractor, and **a partial constellation completes by descending the energy** (modern associative-memory dynamics; attention is one step). Currently implicit; making it explicit unlocks a genuinely new capability.

| Capability | Mechanism (your constructs) | Value |
|---|---|---|
| **Slot completion / constellation completion** | given some lens slots, reconstruct the missing ones by attracting to the nearest grounded region (energy descent over the panel) | answer with partial input; fill a missing modality; repair a degraded constellation; "what *should* the other lenses say here?" |
| **One-shot associative recall** | content-addressable retrieval: any sufficient sub-pattern recalls the whole constellation | robust recall from a fragment (a few slots) — stronger than single-vector ANN |
| **Energy-based in-region test** | `Gτ` extended: pass = the produced vector is in an attractor basin (energy below threshold), not just cosine ≥ τ to one match | a deeper guard (`09`) that accounts for the *whole* stored distribution, harder to fool than nearest-neighbor cosine |
| **Generation as grounded descent** | identity-locked generation = descend into the grounded persona/voice basin (Ward, `09 §5b`) | principled "stay in character / stay grounded" |

Implementation: an energy `E(x) = −logΣ_i exp(β·sim(x, cx_i))` over candidate region members (softmax-weighted similarity; β a sharpness Anneal tunes); completion = a few descent steps updating absent slots; reuses Forge `batched_cosine` + softmax. Honesty: completion is **inference, not ground truth** — completed slots tagged `inferred`/`provisional` (A2), never confused with measured ones (A16).

## 4. Rigorous temporal math on recurrence series — **Build** (strengthens A29)

A29 gives grounded recurrence; today the periodicity read is E3's fixed hour/day match. The recurrence series deserves real time-series math (all information-theoretic, on his frame):

| Method | What it computes | Strengthens |
|---|---|---|
| **Autocorrelation / Lomb–Scargle** (irregular samples) | the *true* dominant period(s), including multiple overlapping ones | repeat-pattern detection + next-occurrence prediction (`25`/`21`) — beyond fixed hour/day |
| **Transfer entropy `T(A→B)`** = `I(B_future ; A_past | B_past)` | directional, information-theoretic influence of event A's history on event B | **rigorous causal discovery** for the Oracle (`21`) — the provable version of "temporal co-occurrence = causality," and it's your fifth element (information) made directional |
| **Inter-event-time distribution + hazard** | cadence distribution; "overdue" hazard | the "expected recurrence didn't happen" anomaly (`25 §4b`) |
| **Change-point detection** (CUSUM-style on the rate) | when a recurring rhythm shifts | drift/regime-change alarm (`17 §8`) |

Transfer entropy reuses the **Assay MI machinery** (`07`) applied to time-lagged recurrence streams — no new estimator, just a new application. Makes the Oracle's causal claims grounded and falsifiable (and `provisional` when the series is too short, A20).

## 5. Multi-way redundancy: total correlation — **Build** (principled `n_eff`)

The differentiation contract's ≤0.6 redundancy rule (A7) is **pairwise**. Three lenses can be jointly redundant while every pair looks fine. The rigorous generalization is **total correlation / multi-information** `TC(Φ) = ΣH(slot_k) − H(Φ)` (the multivariate MI), and **interaction information** for genuine 3-way synergy in cross-terms.

| Use | Benefit |
|---|---|
| `n_eff` from total correlation, not just the pairwise graph | catches multi-way redundancy the pairwise ≤0.6 rule misses → truer effective rank (A9), tighter storage/compute budgets |
| interaction information on a cross-term triple | distinguishes *redundant* pairs from *synergistic* ones (carry info only together) → smarter Loom materialization (`06`) |

Implementation: estimate `TC` via the same KSG/entropy estimators (Assay); report alongside pairwise corr. Honesty: high-d `TC` estimation is noisy — report CI + sample count, fail closed below quorum (A16), keep the cheap pairwise gate as first-pass.

## 6. Bayesian posteriors for rate / consistency / next-occurrence — **Build**

Frequency, oracle self-consistency, and next-occurrence are currently raw counts/conformal. With few occurrences, counts lie. Maintain **conjugate posteriors**:

| Quantity | Posterior | Gives |
|---|---|---|
| event rate (recurrence) | **Gamma–Poisson** | credible interval on rate + next-occurrence, graceful at n=2,3 |
| oracle self-consistency / pass-rate | **Beta–Bernoulli** | uncertainty on flakiness/validity (`07 §3b`) — "is this oracle *reliably* consistent or just so-far?" |
| guard FAR/FRR | Beta | principled calibration uncertainty (`09`) |

These are cheap, online (one update per occurrence), and exactly the small-sample regime where Calyx operates. They make every recurrence/consistency number carry honest uncertainty (A16), and they're the right home for the **frequency→bits** coupling (§9): a Bayesian rate is the anchor a lens is scored against.

## 7. Set-level distribution comparison — **Option**

To compare two *sets* of constellations (this month vs last; domain A vs B; a candidate corpus vs the vault):
- **MMD (maximum mean discrepancy)** — a kernel two-sample test: same distribution? → drift alarm, domain-shift detection, set-level near-dup, "is this new dataset already covered."
- **Optimal transport / Wasserstein** — the *distance* and *alignment* between two constellation distributions → quantify domain gap; align cross-domain bridges (`08`).
Both are batched cosine/distance kernels in Forge. Option-tier: valuable for multi-corpus/streaming deployments, not needed for a single vault.

## 8. Information bottleneck — **Framing**

The kernel + sufficiency are one principle: **compress the panel while preserving `I(panel; oracle)`** — the Information Bottleneck Lagrangian `min I(panel; Z) − β·I(Z; oracle)`. Lodestar's kernel and Assay's sufficiency are two views of the same IB; meaning compression (`23`) is its compression term. Unifies `07`/`08`/`23` under one objective and clarifies that *the kernel is the IB-optimal grounded code for the oracle.* No new engine — a framing that keeps the math coherent.

## 9. The frequency → bits decision (recommendation, implemented in `07`)

**Yes — couple frequency in three grounded ways; never as a raw multiplier on bits.**

1. **Recurrence is an anchor.** Add `AnchorKind::Recurrence` (the Bayesian rate, §6); lenses scored on `I(lens; rate)` like any grounded outcome. Extends A7's targets; contract unchanged.
2. **Stratified bits.** Compute bits **per outcome stratum** (including the rare class) and admit a lens that is the *sole carrier* of a rare-but-critical stratum even if aggregate MI < 0.05. Generalizes the global-0.05 rule to "≥0.05 bits on *some* grounded stratum," matching the ME-JEPA per-sensor/per-class decomposition — fixing the blind spot that **MI on imbalanced anchors underweights the rare class.**
3. **Surprise for learning/anomaly; frequency for importance — neither inflates bits.** Information content is `−log p` (rare = high), which MI already encodes; bits stay = MI (preserves A8/DPI). Frequency drives importance (kernel/retention), surprise drives anomaly/learning priority. **Raw frequency must never multiply bits** — that would reward low-information common detectors.

Net effect on A7: the differentiation contract stays information-theoretic and honest, **refined to per-stratum** so rare-class carriers aren't silently lost, with recurrence as a first-class grounded anchor.

## 10. Recommendation summary & how to implement

| Item | Tier | Home engine | Anneal-tuned? | Honesty guard |
|---|---|---|---|---|
| Spectral centrality / GFT (§2) | Build | Lodestar/`mincut` + Forge | yes (which eigenvectors) | complements, never replaces grounded MFVS kernel |
| Pattern completion / energy (§3) | Build | Ward + Forge | yes (β) | completed slots tagged `inferred`/provisional |
| Recurrence spectral + transfer entropy (§4) | Build | Assay + Oracle | yes (lag) | `provisional` on short series |
| Total correlation / `n_eff` (§5) | Build | Assay | — | CI + quorum, pairwise first-pass |
| Bayesian posteriors (§6) | Build | Assay + Ward | — | credible intervals, fail-closed below quorum |
| MMD / OT (§7) | Option | Forge | — | set-size quorum |
| Information bottleneck (§8) | Framing | unifies 07/08/23 | — | — |
| Frequency→bits: stratified + recurrence anchor (§9) | Build | Assay (`07`) | — | bits stay = MI; no raw-frequency multiplier |

All reuse existing Forge primitives (eigensolve, softmax, MI, distance) and Anneal autotuning — no new external dependency, fully custom-fitted (DOCTRINE §2). Each ships behind FSV: prove the new number against a known synthetic (planted period, planted causal A→B, planted rare-class carrier) by reading the computed value, not a harness.

## 11. Connections between truths (the discovery method, applied) — A30

**The method (founder's, binding):** the connection between two truths, computed with AI as the highest-probability link, is itself true — sometimes it takes a moment to find, but we know it exists and discover it. In Calyx this is both a **design discipline** (how we find capabilities) and a **system capability** (the highest-probability grounded path between two grounded constellations is a candidate true association, which grounding/FSV confirms — A2/A15). Applied to the truths now in the docs, two connections light up:

### 11.1 Prediction = abduction = imputation are ONE operation (`complete`)
Truths connected: pattern completion (§3) + the Oracle's consequence prediction (`21`) + epistemic symmetry's reverse_query (A23). They are the **same energy-descent over a constellation**, differing only in which slots are **clamped** vs **free**:

```
complete(cx, clamp: SlotSet, free: SlotSet) -> filled cx + confidence
  clamp present, free future    → PREDICTION   (the Oracle / consequence)
  clamp outcome, free cause     → ABDUCTION    (reverse_query / root cause)
  clamp some lenses, free rest  → IMPUTATION   (slot completion / repair)
```

One primitive (energy descent over the panel, §3) unifies the Oracle, reverse-query, and completion. Forward/reverse/lateral inference are clamp-direction choices, not separate engines. Honesty: free (filled) slots are `inferred`/`provisional`, confidence capped by oracle self-consistency (`07 §3b`), refused when the panel is insufficient (A20).

### 11.2 The kernel grounds the 99% by diffusion (grounded label propagation)
Truths connected: the ≈1% grounding kernel (`08`) + the graph Laplacian spectrum (§2) + grounding-is-mandatory (A2) + the grounding-gaps problem. The connection: **propagate the kernel's real anchors across the whole association graph by heat diffusion / harmonic extension** (solve the Laplacian system holding anchored nodes fixed, letting the rest settle). Every constellation receives a **propagated, provisional grounding** whose confidence **decays with graph distance** from a real anchor.

The *operational realization* of "1% grounds 99% by association" — today asserted (`08`), now a computation: grounding flows from the kernel's anchored nodes to the whole corpus along the associations. Auto-fills grounding gaps (`08 §6`), turns the kernel into an active grounding source, and every propagated label is tagged `provisional` with a confidence the real-anchor frontier sharpens (never confused with a measured anchor, A2/A16).

### 11.3 The standing discovery loop
A29/§2/§3 also connect: the Laplacian eigenvectors *are* the low-energy modes of the associative memory (§3) — the spectral and energy views are the same operator, so a region (spectral community) and an attractor basin (energy) coincide. The completeness-critic stays running (`17 §8`): *which two truths have we not yet connected?* — and the database itself proposes connections (highest-probability grounded paths) for grounding to confirm. Every confirmed connection is a new capability discovered, not invented.

**One sentence:** the architecture makes available a layer of math the engines don't yet use — the spectrum of the association graph (continuous kernel-importance + frequency-of-meaning), energy-based associative pattern completion (reconstruct missing slots), rigorous information-theoretic temporal causality (transfer entropy), multi-way redundancy (total correlation), and Bayesian uncertainty on rate/consistency — all pure expressions of the calculus of association, and frequency couples into the bits contract only through a recurrence anchor + per-stratum evaluation, never as a raw multiplier.
