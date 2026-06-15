# 07 — Assay: Signal & Bits Engine

> **Living-system role:** differentiation + the self-model — learning what carries grounded information and knowing how sure it is (A31 — DOCTRINE §1b)

Implements A7/A8/A9 and the paper's **information-theoretic differentiation contract**: every lens must add ≥ **0.05 bits** about a real outcome; no pair may correlate above **0.6**. Analyzes embedder capabilities and decides what to keep.

## 1. What Assay measures

| Quantity | Symbol | Definition | Decision it drives |
|---|---|---|---|
| Per-lens signal | `I(slot_k ; anchor)` | MI between a slot's vectors and a grounded outcome | admit/park/retire a lens (≥0.05 bits to admit) |
| Pairwise redundancy | `corr(slot_a, slot_b)` / `I(a;b)` | linear corr + normalized MI between two slots | reject a duplicative lens (≤0.6 to admit) |
| Panel sufficiency | `I(panel ; anchor)` | MI of the whole panel about the outcome | the DPI ceiling (A8); flags "panel can't predict this" |
| Cross-term gain | `I(a,b ; anchor) − max(I(a;anchor),I(b;anchor))` | extra bits a pair carries beyond its members | Loom materialization gate (`06`) |
| Effective rank | `n_eff` | non-redundant lens count via the redundancy graph | sizing/cost budgets (A9) |
| Marginal lens value | `I(panel;anchor) − I(panel∖k;anchor)` | bits lost if lens k removed | "is this lens load-bearing?" |

## 2. Estimators (chosen for correctness on small grounded samples)

Anchored samples are often scarce (a few hundred labeled outcomes), so Assay ships multiple estimators and picks by sample size / type:

| Estimator | For | Notes |
|---|---|---|
| **KSG** (Kraskov–Stögbauer–Grassberger) | continuous↔continuous, continuous↔discrete | k-NN based; the production default for vector↔outcome MI; bias-corrected, no binning. |
| **Partitioned histogram NMI** | high-d, large-n | fast, used for streaming redundancy on the agreement graph (absorbed from ContextGraph `pairwise_mi` `partitioned_histogram_nmi_v1`). |
| **Linear corr / CCA** | quick redundancy gate | cheap first-pass for the ≤0.6 rule; promotes to MI if borderline. |
| **Binary-outcome logistic probe** | anchor∈{Pass,Fail} | bits = reduction in cross-entropy of a calibrated probe; matches the ME-JEPA "≈1 bit verdict" framing. |

Every estimate carries a **bootstrap confidence interval** and a **sample count**; Assay **fails closed** below a minimum sample quorum (`CALYX_ASSAY_INSUFFICIENT_SAMPLES`, default n≥50) rather than reporting a noisy number (A16).

All estimators run in Forge (k-NN via the same ANN graphs; histograms/probes batched on GPU). Dimensionality reduction (random projection / PCA to a stable-rank subspace) precedes KSG on high-d slots to control k-NN bias.

## 3. The differentiation contract (gated in-engine)

```
admit_lens(candidate_slot, anchor, panel):
  bits        = I(candidate ; anchor)                      # KSG / probe
  max_corr    = max over k in panel of corr(candidate, slot_k)
  if bits   < 0.05  -> REJECT  (CALYX_ASSAY_LOW_SIGNAL)    # adds no grounded info
  if max_corr > 0.6 -> REJECT  (CALYX_ASSAY_REDUNDANT)     # duplicates an existing lens
  else ADMIT, store bits_about[anchor] on the Slot
```

The paper's contract made executable, **enforced at admission and re-checked by Anneal** as the corpus grows. A lens decaying below 0.05 bits (drift) is auto-**parked** (kept, not searched) with an alert; a newly-redundant pair triggers a review. Thresholds are config-overridable per vault but default to the paper's values verbatim (`0.05`, `0.6`).

## 3b. Oracle self-consistency, measured from recurrence (A29)

The paper's hard ceiling is `τ_corr ≤ oracle_self_consistency` (flakiness + validity). Calyx measures it **natively** from grounded recurrence (`25 §4c`): when the **same action recurs across time** (recurrence signature — all content lenses agree, temporal lenses differ) carrying an outcome each time, Assay checks whether those outcomes **agree**:
- outcomes agree across occurrences → **consistent** (low flakiness); the oracle reproduces its verdict.
- outcomes differ on content-identical occurrences → **flaky**; the self-consistency ceiling drops, and Assay reports it.
- Frequency itself is a **grounded anchor** (a real count) Assay can compute bits about.

```
oracle_self_consistency(domain) -> { flakiness, validity, ceiling }
  // flakiness estimated from disagreement among recurring (content-identical, time-distinct) events' anchors
```
Unique to Calyx: because it dedups by content and tracks recurrence over time, it can *empirically* measure the central ceiling of *The Oracle and the Kernel* — no other store has the signal. It caps every predictor's reported confidence (`21`).

## 3c. Stratified bits & the frequency→bits coupling (refines A7, `26 §9`)

The differentiation contract (A7) is refined so a lens carrying a **rare-but-critical** class is not lost (MI on imbalanced anchors underweights the rare class — a real blind spot):

- **Stratified bits:** compute `I(lens; outcome)` **per outcome stratum**, not only globally. Admit a lens if it clears **≥0.05 bits on *some* grounded stratum** (e.g. the sole carrier of the rare class), even if aggregate MI < 0.05. Matches the ME-JEPA per-sensor/per-class decomposition (`§5`).
- **Recurrence as an anchor:** `AnchorKind::Recurrence` (the Bayesian rate, `26 §6`) is a grounded outcome lenses are scored against — `I(lens; rate)`. This *extends* A7's targets; the contract is unchanged.
- **No raw-frequency multiplier (binding):** frequency drives *importance* (kernel/retention) and surprise (`−log p`, rare = high) drives *anomaly/learning*; **bits stay = MI** (preserves the DPI honesty, A8). Multiplying bits by raw frequency would reward low-information common detectors — forbidden.

## 4. Panel sufficiency & the kernel-existence test

Assay computes `I(panel; anchor)` — the **substrate-sufficiency** number from *The Oracle and the Kernel*. Interpretation, exposed in `abundance_report`:

- `I(panel; anchor)` ≪ `H(anchor)` (e.g. 0.46 of the ~1 bit a Pass/Fail verdict needs — ME-JEPA's measured negative) → **no predictor/kernel/guard reading this panel can close the gap** (DPI). Calyx surfaces this as a **red flag with a localized deficit** (which slots are missing the bits) and routes it to Anneal's lens-proposal path (`12`) — *the fix is a new outcome/behavior lens, not more training.*
- `I(panel; anchor)` ≈ `H(anchor)` → panel sufficient; Lodestar's kernel is trustworthy.

This makes "is this architecture even capable?" a **cheap measurement that runs before any model is trained** — the paper's headline contribution, as a database query.

## 5. Per-sensor decomposition (where the bits live)

Assay reports a **bit-attribution table** per anchor: each slot's marginal bits, its redundancy with the rest, and whether it's the *sole* carrier of any signal (the ME-JEPA "no sensor is load-bearing / signal survives only in the syntactic class" diagnostic). The actionable artifact: an agent reads it and knows exactly which lens to add or cut.

## 6. Outputs & storage

- `Slot.bits_about: Map<AnchorKind,f32>` (with CI + sample count + estimator + ts) — refreshed incrementally.
- Pairwise MI/corr rows in a small `assay` store keyed `(a,b,anchor,shard_hash,ts)` (mirrors ContextGraph `CF_MEJEPA_PAIRWISE_MI`), with provenance to the corpus shard the estimate was computed on (reproducibility, `11`).
- `n_eff` and `I(panel;anchor)` cached per panel_version + corpus shard.

## 7. Honesty rules (inherited, binding)

- Assay MUST only label bits "**trusted**" when computed against a **grounded** anchor (A2); bits about an ungrounded/auto-labeled target are tagged `provisional`.
- Assay MUST report the **DPI ceiling** alongside any abundance claim (A8) — no selling `C(N,2)` as independent information.
- Assay MUST attach sample count + CI to every number and fail closed below quorum.

## 8. Assay API (summary; full in `18`)

```
lens_signal(slot, anchor) -> {bits, ci, n, estimator}
pair_redundancy(a, b) -> {corr, nmi}
panel_sufficiency(anchor) -> {I_panel, H_anchor, deficit, per_slot_attribution}
n_eff(panel) -> f32
marginal_value(slot, anchor) -> bits_if_removed
differentiation_check(candidate, anchor) -> Admit | Reject{reason}
```

**One sentence:** Assay knows, in bits, what every lens is worth and whether the panel can even answer the question — turning lens selection from guesswork into a measurement.

Sources: KSG estimator (Kraskov, Stögbauer & Grassberger, *Phys. Rev. E* 2004); [KNN information-estimator analysis](https://arxiv.org/abs/1810.11571); data-processing inequality (Cover & Thomas).
