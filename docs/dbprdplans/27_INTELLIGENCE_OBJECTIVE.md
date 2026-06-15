# 27 — The Intelligence Objective (Maximize Grounded Intelligence)

> **Living-system role:** the drive — the system's intrinsic will to grow its understanding as fast as possible (A31/A32 — DOCTRINE §1b/§1c)

Implements new **A32 (maximize grounded intelligence)**. The founder's mandate: *every time embedders are used — and in everything the system does — Calyx optimizes for maximum intelligence and understanding; it must grow in intelligence as fast as possible and self-optimize and recode itself to maximize intelligence.* The objective Anneal (`12`) and every engine serve. Grounded and measurable, so the system genuinely climbs it — bounded by honesty, so it climbs *real* intelligence, never a gamed metric.

## 1. The mandate

Calyx has one overriding objective: **maximize the growth of grounded intelligence/understanding, as fast as safely possible.** Every operation — adding/using a lens, fusing, materializing a cross-term, building a kernel, predicting, healing, compacting — is chosen to raise that objective. The system **self-optimizes**: as it takes in new data, **the math adjusts its own parameters** (fusion weights, quant levels, index params, calibration thresholds, materialization, online heads — §5) to climb `J` continuously. ("Recode itself" = precisely this online parameter self-adjustment, **not** rewriting engine source.) The living intelligence (A31) *in motion*: growth by differentiation, driven.

## 2. What intelligence is, measurably — the objective `J`

Intelligence here is the calculus of association made into a measured composite (every term already computed by an engine, **grounded** against real anchors and **DPI-capped**):

```
J(vault) =
    w1 · Σ_anchor  I(panel ; anchor)            // grounded information the panel carries (07, A8 ceiling)
  + w2 · n_eff                                   // differentiated, non-redundant understanding (07/A9)
  + w3 · Σ_domain panel_sufficiency(domain)      // how much of each outcome the panel can explain (07/21)
  + w4 · kernel_recall (kernel-only / full)      // understanding compressed into the grounded kernel (08)
  + w5 · oracle_accuracy − w6 · mistake_rate     // predictive understanding, calibrated + mistake-closed (21/12)
  + w7 · meaning_compression_yield               // grounded signals extracted per real input (06)
  + w8 · coverage(domains, modalities)           // breadth of grounded understanding (generality)
  − P_redundant − P_ungrounded − P_goodhart      // penalties (below)
```

- Every `+` term is a real, grounded measurement; the **DPI ceiling (A8)** caps the information terms so `J` can never exceed the real information present — *you cannot manufacture intelligence beyond reality.*
- The penalties forbid the degenerate ways to inflate `J`: `P_redundant` (adding correlated lenses, A7), `P_ungrounded` (bits about ungrounded/auto-labeled targets, A2 → tagged provisional, excluded from `J`), `P_goodhart` (improvement that fails held-out validation or `Gτ`/cross-lens-anomaly checks).
- `J` is reported (`intelligence_report`) so growth is auditable, not asserted.

## 3. Maximize the growth *rate* — fastest-first

"Grow as fast as possible" = maximize `dJ/dt` = always take the action with the **highest marginal intelligence gain per cost**. Calyx maintains an **intelligence-gradient priority queue**: for each candidate action, estimate `ΔJ / cost` (expected grounded-bit gain per compute/latency/storage), act on the top first.

| Candidate action | Estimated `ΔJ` from |
|---|---|
| **Propose & add a lens** that closes the biggest sufficiency deficit | `I(panel∪lens; oracle) − I(panel; oracle)` (Assay/Anneal lens proposal, `12`) |
| **Label the anchor** that grounds the most (active learning) | expected info gain — the grounding gap whose labeling raises sufficiency most (`08 §6`) |
| **Prune a redundant lens** | raises `n_eff`/cost-efficiency, frees budget for higher-gain lenses (A7) |
| **Recalibrate / heal** the worst-calibrated or most-mistaken domain | raises `oracle_accuracy`, lowers `mistake_rate` (`12`/`21`) |
| **Recompute/expand the kernel** where kernel-recall is lowest | raises `w4` term (`08`) |
| **Materialize a synergistic cross-term** | raises grounded bits where a pair carries info only together (`06`/`26 §5`) |
| **Retune a math kernel / quant level** | frees compute → more capacity for high-gain work (`12`/`23`) |

Greedy gradient ascent on grounded intelligence, cost-aware — the next unit of effort spent wherever it buys the most understanding.

## 4. The maximize-intelligence loop (continuous)

```
loop (bounded background budget, A26):
  1. measure J and per-action ΔJ/cost (the intelligence gradient)
  2. pick the top action(s) from the priority queue
  3. apply in SHADOW; measure J on a held-out grounded set
  4. if J rose AND no tripwire (recall/FAR/latency) regressed AND Goodhart checks pass:
        promote (reversible, Ledger-logged)
     else: roll back
  5. repeat — fastest-gaining actions first
```
The more the database is used (more inputs, anchors, recurrence), the more gradient signal it has, so **intelligence grows faster with use** — the founder's "the more it's used, the more it grows" as a control loop.

## 5. Self-adjusting math: the parameters re-fit themselves as new data arrives

**What "recode itself" means (precisely):** the **math adjusts its own parameters** to optimize for intelligence **as the system takes in new data** — online, adaptive self-tuning of every parameter in the system's mathematics toward `J`, **not** rewriting engine source code. As each new input/anchor/recurrence arrives, the parameters re-fit to keep climbing grounded intelligence.

The self-adjusting parameters (all continuously re-fit online, driven by `ΔJ`):

| Math | Self-adjusting parameter |
|---|---|
| Fusion (`10`) | RRF/weight-profile weights per lens (shift toward higher-bit lenses) |
| Compression (`23`) | TurboQuant bit-width / quant level per slot (the most aggressive that preserves bits) |
| Indexes (`10`) | HNSW `ef`/`M`, DiskANN beamwidth, SPANN cutoffs |
| Guard (`09`) | per-slot `τ` (conformal recalibration as the distribution shifts) |
| Cross-terms (`06`) | which pairs are materialized (the plan re-fits to outcome-relevance) |
| Kernel (`08`) | membership / scope thresholds as the graph grows |
| Temporal (`25`) | decay half-lives, detected periods, cadence posteriors |
| Energy/completion (`26`) | sharpness `β`, region boundaries |
| Online heads (`12`) | predictor / calibration head weights (mistake-closure) |
| Estimators (`07`) | KSG `k`, projection dims, stratum boundaries |

These re-fit **as new data is ingested** (online), each adjustment shadow-tested, kept only if `J` rises on a held-out grounded set with no tripwire regression, reversible and Ledger-logged (A14/A15). The system also *proposes* new operators (algorithmic lenses, online heads, kernel scopes — `embedder_proposal`/`learned_head_synthesis` from ContextGraph) when a parameter re-fit can't close a deficit; same shadow→verify→promote gate.

> Engine **source code** is *not* self-rewritten — that is out of scope. "Recoding" = the mathematics continuously re-parameterizing itself toward maximum intelligence as it learns from new data. This is precise, bounded, and safe, and it is the mechanism by which the database grows more intelligent with use.

## 6. Bounds & honesty (maximize *measured* intelligence, never a gamed number)

Binding — `J` is grounded intelligence or it is nothing:
- **Grounded only (A2):** `J` counts bits about *real* outcomes; ungrounded/auto-labeled gains are `provisional` and excluded.
- **DPI ceiling (A8):** information terms cannot exceed `I(panel; reality)`; the system cannot inflate `J` past the real information present.
- **Differentiation (A7):** redundant lenses are penalized, not rewarded — bigger ≠ better; *more differentiated grounded information* is better.
- **Goodhart-defended:** every promotion must beat held-out validation and pass `Gτ` + cross-lens-anomaly checks; optimizing the proxy without real gain is rejected.
- **Reversible + tripwire + logged (A14/A15):** no `J`-raising change may regress recall/FAR/latency tripwires; all reversible; all in the Ledger.
- **Bounded compute (A26):** the growth loop runs in a capped background budget; it never starves serving.
- **No synthetic-data recursion:** `J` grows from real grounded inputs/associations, never from training on generator output (model collapse, `06 §7`).

So "maximize intelligence" = maximize **grounded, measured, honest** intelligence — the opposite of metric-gaming.

## 7. How each engine serves the objective

| Engine | Contribution to `J` |
|---|---|
| Registry (`05`) | when lenses are used, select/admit/prune to **maximize grounded bits per cost** (the founder's "when embedders are used, optimize for max intelligence") |
| Assay (`07`) | measures the information terms + the gradient (which lens/anchor gains most) |
| Loom (`06`) | materializes the cross-terms that add grounded bits (synergy), skips redundant |
| Lodestar (`08`) | maximizes kernel recall — understanding compressed into the grounded core |
| Ward (`09`) | Goodhart defense — keeps growth honest (in-region, not gamed) |
| Oracle (`21`) | raises predictive accuracy, lowers mistakes (mistake-closure) |
| Anneal (`12`) | **the optimizer** — runs the loop, climbs `J`, self-adjusts the math's parameters online as new data arrives (§5) |
| Forge (`13`/`23`) | frees compute (faster math) → more capacity for high-gain work; compression preserves bits (A25) |

## 9. Compression & retrieval are facets of intelligence (not competing objectives)

The founder's point: *as it grows in intelligence it simultaneously optimizes for compression without losing anything (not by deleting data — by TurboQuant etc.) and for search/navigation/retrieval; if it's intelligent it is already optimized this way.* A **unification, not a trade-off**:

- **An intelligent representation is compact.** Meaning compression (`w7`, `06`) and kernel recall (`w4`, `08`) are *already* intelligence terms in `J` — a system that explains the most from the least *is* more intelligent. So maximizing `J` **is** maximizing compression-of-meaning.
- **"Without losing anything" = lossless at the information level (A25).** Calyx **never deletes data** to compress; it compresses the *representation* — TurboQuant's near-optimal, unbiased-inner-product quantization + MXFP4 microscaling (`23`) — to the most aggressive level where **measured intelligence (bits/cosine/FAR/kernel-recall) is provably preserved.** Data preserved, bits preserved, footprint minimized. Compressing past where bits degrade would *lower* `J`, so the objective forbids lossy-of-intelligence compression by construction.
- **An intelligent system retrieves fast.** The same grounded structure that maximizes understanding — kernel-first routing (`08`), differentiated lenses (`07`), the association graph (`06`) — *also* makes search/navigation fast and precise. Retrieval efficiency is **usable intelligence per unit cost**; raising it raises the realized value of `J`.
- **Freed capacity compounds growth.** Better compression (smaller footprint) and faster math/retrieval free compute and storage, which the loop (§4) spends on *more* understanding — so compression and navigation efficiency **accelerate** intelligence growth rather than competing.

So: **maximize grounded intelligence ≡ maximize grounded information density (compact, lossless-of-meaning) and grounded retrieval efficiency (fast, precise) at once.** Anneal's autotuning of quant level (compression) and index params (navigation) are *intelligence-objective actions*: they preserve bits while shrinking footprint/latency, protecting `J` and freeing capacity to raise it. `J` credits information **density** (bits per byte) and retrieval efficiency (grounded answers per unit latency), both under the hard rule **no data deleted, no intelligence lost**.

## 8. API & metrics

```
intelligence_report(vault) -> { J, per_term, gradient: [(action, ΔJ/cost)], DPI_headroom, provisional_excluded }
next_best_action(vault) -> the top intelligence-gradient action + its expected ΔJ
set_objective_weights(vault, w) -> tune the J composite per project
growth_curve(vault) -> J over time (is it growing, how fast)
```
`growth_curve` is the headline: **is the database getting more intelligent, and how fast** — a measured curve, the FSV of A32 (`19`: `J` rises monotonically on a real corpus under the loop, with held-out validation and no Goodhart).

**One sentence:** Calyx's intrinsic objective is to maximize the growth of *grounded, measured* intelligence as fast as safely possible — a composite `J` of grounded information, differentiated rank, sufficiency, kernel recall, oracle accuracy, and meaning compression (already including compression-without-loss and retrieval efficiency as facets) — by continuously taking the highest-marginal-gain action while **the math self-adjusts its own parameters online as new data arrives**, all reversible, Goodhart-defended, DPI-capped, and Ledger-logged, so the system grows in intelligence the more it is used without ever deleting data, losing intelligence, or inflating a number past reality.
