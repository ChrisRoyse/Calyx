# 11. Assay — Signal Bits, Redundancy Contract, and Panel Sufficiency

**Source files covered:**
- `crates/calyx-assay/src/lib.rs`
- `crates/calyx-assay/src/ksg.rs`
- `crates/calyx-assay/src/logistic.rs`
- `crates/calyx-assay/src/nmi.rs`
- `crates/calyx-assay/src/contract.rs`
- `crates/calyx-assay/src/formulas.rs`
- `crates/calyx-assay/src/gate.rs`
- `crates/calyx-assay/src/sufficiency.rs`
- `crates/calyx-assay/src/attribution.rs`
- `crates/calyx-assay/src/stratified.rs`
- `crates/calyx-assay/src/estimate.rs`
- `crates/calyx-assay/src/bootstrap.rs`
- `crates/calyx-assay/src/n_eff.rs`
- `crates/calyx-assay/src/store.rs`
- `crates/calyx-assay/src/loom_adapter.rs`
- `crates/calyx-assay/src/recurrence_anchor.rs`
- `crates/calyx-assay/src/bayesian.rs`
- `crates/calyx-assay/src/total_correlation.rs`
- `crates/calyx-assay/src/transfer_entropy.rs`
- `crates/calyx-assay/src/periodicity.rs`
- `crates/calyx-assay/src/recurrence_hazard.rs`
- `crates/calyx-assay/src/mmd.rs`
- `crates/calyx-assay/src/projection.rs`
- `crates/calyx-assay/src/formula_catalog.rs`
- `crates/calyx-assay/src/formulas.rs`
- supporting: `crates/calyx-core/src/error.rs` (error codes), `crates/calyx-aster/src/cf/family.rs` (CF id)
- planning ref: `docs/dbprdplans/07_ASSAY_SIGNAL_BITS.md`

The crate `lib.rs` doc line is: "Assay signal-bit measurement, panel sufficiency, and persistence contracts."

This crate depends on `calyx-aster`, `calyx-core`, `calyx-loom`, `blake3`, `rand`, `rand_chacha`, `serde`, `serde_json` (`crates/calyx-assay/Cargo.toml`). There are no CUDA/Forge dependencies in the build graph; all math here is CPU Rust. The planning doc's claim that "all estimators run in Forge … batched on GPU" is **not** reflected in the code; `project_gpu` is an explicit stub that returns an error (see §6.4 and Gaps).

---

## 1. Information-theoretic core: bits per lens

Assay measures information in **bits** (base-2 logs throughout). Three estimators produce a per-lens / per-pair / per-panel mutual-information (MI) number, each wrapped in an `MiEstimate` with a bootstrap CI, sample count, estimator kind, and trust tag.

### 1.1 Shared estimate type (`estimate.rs`)

`MiEstimate` is the universal carrier of any bits result.

| Field | Type | Meaning |
|---|---|---|
| `bits` | `f32` | Point MI estimate (clamped `>= 0.0` in `MiEstimate::new`). |
| `ci_low` | `f32` | Lower CI bound (clamped to `<= bits`, `>= 0`). |
| `ci_high` | `f32` | Upper CI bound (clamped to `>= bits`). |
| `n_samples` | `usize` | Paired-sample count the estimate was computed on. |
| `estimator` | `EstimatorKind` | Which method produced it. |
| `trust` | `TrustTag` | `Trusted` or `Provisional`. |

`MiEstimate::point(bits, n, est, trust)` builds an estimate with a synthetic band of `(|bits|*0.15).max(0.02)` either side (used when no bootstrap is available, e.g. outcome-entropy rows).

`EstimatorKind` variants (serde `snake_case`): `Ksg`, `HistogramNmi`, `LogisticProbe`, `Bootstrap`, `PanelSufficiency`, `OutcomeEntropy`, `PairGain`.

`TrustTag` variants: `Trusted`, `Provisional`. Trust is set by `trust_for_anchor(Option<&Anchor>)`: an anchor is **grounded** (and hence `Trusted`) iff `is_grounded_anchor` holds — `anchor.source` is non-blank, and `anchor.confidence` is finite, `> 0.0`, and `<= 1.0`. Otherwise `Provisional`. `require_grounded_anchor(&Anchor)` returns `Trusted` or errors `CALYX_ASSAY_INSUFFICIENT_SAMPLES`. `provisional_without_anchor(_)` always downgrades to `Provisional` (used by the non-anchor sufficiency/attribution entry points so they never claim trust).

### 1.2 KSG estimator (`ksg.rs`) — production default

This is the **Kraskov–Stögbauer–Grassberger** k-nearest-neighbor MI estimator, KSG estimator **1** (the joint-radius / count-in-each-marginal form). It is the default for vector↔outcome MI.

Constant: `MIN_ASSAY_SAMPLES: usize = 50` — the hard sample floor used across KSG, NMI, logistic and TC quorum checks.

Public entry points:
- `ksg_mi_continuous(x, y, k)` — continuous↔continuous; trust defaults to `Provisional`.
- `ksg_mi_continuous_with_anchor(x, y, k, anchor)` — same, trust from anchor.
- `ksg_mi_continuous_discrete(x, labels, k)` — continuous↔discrete; labels `&[usize]` are **one-hot encoded** into `y` rows (one column per distinct label, ordered by first appearance via a `BTreeMap`), then routed through the continuous path.
- `ksg_mi_continuous_discrete_with_anchor(x, labels, k, anchor)` — same, trust from anchor.

`x` and `y` are `&[Vec<f32>]` (n rows, each a vector). Both must be rectangular and finite (`validate_rectangular_finite`).

**Validation / fail-closed (`validate_sample_counts`):** errors `CALYX_ASSAY_INSUFFICIENT_SAMPLES` if `x.len() != y.len()`, or `len < MIN_ASSAY_SAMPLES (50)`, or `k == 0`, or `k >= n`.

**Exact per-point computation (`ksg_bits_from_validated_samples`), for each sample i over n points:**
1. `eps = kth_joint_radius(x, y, i, k)` — the distance to the k-th nearest neighbor of point i in the **joint** space, where the joint distance between i and j is `max(chebyshev(x_i, x_j), chebyshev(y_i, y_j))` (Chebyshev / L∞ in each marginal, max across the two marginals). The k-th value is taken (`distances[k-1]`), floored at `f32::EPSILON`.
2. `nx = neighbor_count(x, i, eps)` — count of j ≠ i with `chebyshev(x_i, x_j) < eps` (strict).
3. `ny = neighbor_count(y, i, eps)` — likewise in the y marginal.
4. Local term (in nats):
   `psi(k) + psi(n) - psi(nx + 1) - psi(ny + 1)`
   where `psi` is the digamma function (`digamma`).
5. Convert to bits: `local / ln(2)` (`std::f64::consts::LN_2`).

The estimate is `mean(local_bits).max(0.0)` — the average over all points, clamped non-negative.

`digamma(x)` is computed by upward recurrence to `x >= 7` then the asymptotic series `ln(x) - 1/(2x) - 1/(12x²) + 1/(120x⁴)`.

**Complexity:** O(n²·dim) per estimate (brute-force pairwise distances, no ANN index despite the plan's claim of "k-NN via the same ANN graphs").

A **bootstrap paired CI** (see §1.5) is computed by resampling `(x_i, y_i)` pairs and re-running the point estimator; if the bootstrap yields nothing the call errors `CALYX_ASSAY_INSUFFICIENT_SAMPLES`.

`ksg_mi_continuous_point(x, y, k) -> f32` is a crate-internal point-only variant reused by total-correlation / transfer-entropy.

### 1.3 Logistic probe estimator (`logistic.rs`) — binary anchors

For a `bool` outcome (Pass/Fail). `logistic_probe_mi(samples, labels)` and `logistic_probe_mi_with_anchor(..)` return a `LogisticProbeReport { estimate: MiEstimate, accuracy: f32, selected_field: &'static str }` (`selected_field` is always `"logistic_probe"`).

Despite the name, the probe is a **mean-difference linear classifier**, not a fitted logistic regression:
1. `class_means` — compute the per-dimension mean of `samples` for positive-label rows and for negative-label rows (`pos_mean`, `neg_mean`).
2. `direction = pos_mean - neg_mean`; `midpoint = (pos_mean + neg_mean) * 0.5`; `threshold = dot(midpoint, direction)`.
3. Predict `true` for a row iff `dot(row, direction) >= threshold`.
4. `accuracy` = fraction of correct predictions.
5. `bits = binary_mi(labels, predictions)` — the **2×2 confusion-matrix MI** between true label and predicted label: `Σ_{y,p} P(y,p)·log2( P(y,p) / (P(y)·P(p)) )`, clamped `>= 0`.

So the reported bits = MI between the label and the linear probe's prediction (≤ 1 bit for a balanced binary anchor). Fail-closed: `CALYX_ASSAY_INSUFFICIENT_SAMPLES` if `samples.len() != labels.len()` or `< min_samples` (default `MIN_ASSAY_SAMPLES = 50`; the gate can override). CI via bootstrap.

### 1.4 Partitioned-histogram NMI (`nmi.rs`) — redundancy estimator

`partitioned_histogram_nmi(x: &[f32], y: &[f32], bins) -> NmiReport`. Used for cheap pairwise redundancy on scalar streams.

Steps: clamp `bins = bins.max(2)`; equal-width bin each of x and y over its own `[min, max]` range (`bin_values`, last bin inclusive); compute marginal entropies `Hx`, `Hy` (base-2) and joint entropy `Hxy` over the `(bin_x, bin_y)` pairs; `MI = (Hx + Hy - Hxy).max(0)`; `NMI = MI / sqrt(Hx·Hy)` (0 if denom 0).

`NmiReport` fields: `nmi`, `mi_bits`, `x_entropy_bits`, `y_entropy_bits`, `bins`, `n_samples`. Fail-closed (`CALYX_ASSAY_INSUFFICIENT_SAMPLES`): mismatched lengths, `< 50` samples, or any non-finite value. Tests assert NMI(x,x) ≥ 0.8 and NMI of independent ≤ 0.1.

### 1.5 Bootstrap CI (`bootstrap.rs`)

Deterministic percentile bootstrap, ChaCha8-seeded.

Constants: `DEFAULT_BOOTSTRAP_RESAMPLES: usize = 200`, `DEFAULT_BOOTSTRAP_SEED: u64 = 0`.

`BootstrapConfig { resamples: usize, seed: u64 }` (const `new`, `Default` = 200/0). `BootstrapCi { mean, ci_low, ci_high, resamples }`.

- `bootstrap_mean_ci(values, resamples, seed)` / `_with_config` — resample-with-replacement means.
- `bootstrap_paired_ci(left, right, point_estimate, config, estimator)` — resamples aligned `(left_i, right_i)` index pairs `resamples` times, runs the supplied estimator closure on each resample; returns `Ok(None)` if empty / length mismatch / `resamples == 0`.

CI bounds: take the 2.5% and 97.5% percentiles of the resample estimates (`percentile_index` rounds `(len-1)*p`), then **widen** by the inter-percentile span: `ci_low = (p2.5 - span).min(point)`, `ci_high = (p97.5 + span).max(point)`, with `span = p97.5 - p2.5`. So Assay's CIs are deliberately conservative (roughly double-width).

### 1.6 Pair gain / cross-term bits (`gate.rs`)

`AssayGate { min_samples: usize }` (`Default` = 50) is the facade used by the Loom materialization gate.
- `lens_signal(samples, labels) -> LensSignal { estimate }` and `lens_signal_with_anchor(..)` — call the logistic probe with `min_samples`.
- `pair_gain(left, right, labels) -> PairGain` (and `_with_anchor`): compute the logistic-probe bits of `left` alone, `right` alone, and the **column-concatenation** of the two; then `pair_gain_from_estimates`.
- `pair_gain_from_estimates`: `gain_bits = (pair.bits - max(left.bits, right.bits)).max(0)` — extra bits a pair carries beyond its best member. CI: `ci_low = (pair.ci_low - max(left.ci_high,right.ci_high)).max(0)`, `ci_high = (pair.ci_high - max(left.ci_low,right.ci_low)).max(gain_bits)`.

`PairGain` fields: `left_bits, right_bits, pair_bits, gain_bits, ci_low, ci_high, n_samples`. `pair_gain_estimate[_with_anchor]` wraps `gain_bits` into an `MiEstimate` with `EstimatorKind::PairGain`.

### 1.7 PRD-22 formula wrappers (`formulas.rs`)

Thin named wrappers re-exported as `lens_signal`, `pair_redundancy`, `marginal_value`, `dpi_ceiling`:
- `lens_signal(signal_bits, max_pairwise_corr) -> AdmissionDecision` — delegates to `admit_lens`.
- `pair_redundancy(correlation) -> f32` — returns `|correlation|`; errors `CALYX_ASSAY_REDUNDANT` if non-finite or `> MAX_PAIRWISE_CORR (0.6)`.
- `marginal_value(panel_bits, panel_without_lens_bits) -> f32` — `(panel_bits - panel_without_lens_bits).max(0)` (bits lost if a lens is removed); inputs must be finite non-negative or `CALYX_ASSAY_INSUFFICIENT_SAMPLES`.
- `dpi_ceiling(panel_outcome_bits) -> f32` — identity on a validated non-negative input (the data-processing-inequality ceiling = `I(panel; anchor)`).

---

## 2. The redundancy contract (`contract.rs`)

### 2.1 Thresholds (the paper's values, verbatim, as constants)

| Constant | Value | Meaning |
|---|---|---|
| `MIN_SIGNAL_BITS` | `0.05` (f32) | A lens must add **≥ 0.05 bits** about an outcome to be admitted. |
| `MAX_PAIRWISE_CORR` | `0.6` (f32) | A candidate's max pairwise correlation with any panel lens must be **≤ 0.6**. |

These are compile-time constants; there is **no config-override path in this crate** (the plan says they are config-overridable per vault, but `contract.rs` hard-codes them — see Gaps).

### 2.2 `AdmissionDecision`

| Field | Type | Meaning |
|---|---|---|
| `admitted` | `bool` | Whether the lens passes. |
| `signal_bits` | `f32` | The bits used in the decision. |
| `max_pairwise_corr` | `f32` | The max correlation checked. |
| `stratified_override` | `bool` | True if admitted only via the rare-class stratified path (§2.4). |

Note: `admit_lens` only ever returns `Ok(AdmissionDecision { admitted: true, .. })` or an `Err`; rejection is signalled by the error, not by `admitted == false`.

### 2.3 Admission algorithm (`decide`)

`admit_lens(signal_bits, max_pairwise_corr)` calls `decide(signal_bits, max_pairwise_corr, false)`:
1. If `signal_bits` not finite → `CALYX_ASSAY_LOW_SIGNAL`.
2. If `max_pairwise_corr` not finite → `CALYX_ASSAY_REDUNDANT`.
3. If `signal_bits < 0.05` → `CALYX_ASSAY_LOW_SIGNAL` ("lens signal {bits} below 0.0500"). **(low-signal / drift rejection)**
4. If `max_pairwise_corr > 0.6` → `CALYX_ASSAY_REDUNDANT` ("pairwise correlation {corr} above 0.6000"). **(redundant-lens rejection / pruning)**
5. Else `Ok(admitted: true)`.

Redundancy detection is thus a **per-candidate max-correlation gate**: the caller computes the candidate's correlation against each existing panel lens (or NMI via `nmi.rs` / `pair_redundancy`), takes the max, and any candidate exceeding 0.6 is rejected as duplicative. There is no in-crate graph-clustering pruner; `n_eff` (§4.2) reports the effective non-redundant count but does not itself prune.

### 2.4 Stratified override for rare-but-critical classes (`stratified.rs`)

`admit_lens_with_strata(strata: &StratifiedBits, max_pairwise_corr)` admits a lens whose **global** MI is below 0.05 if it carries a rare stratum. `stratified_override` is set true iff:
`strata.effective_bits >= 0.05` **and** `strata.global_bits < 0.05` **and** some stratum has `sole_carrier == true`.
The decision then uses `effective_bits` as the signal.

`StratumBits { name, bits, frequency, sole_carrier }`. `StratifiedBits { global_bits, effective_bits, strata, no_frequency_multiplier }`. `stratified_bits(global_bits, strata)`: `effective_bits = max(global_bits, max bits over sole_carrier strata)`; `no_frequency_multiplier` is hard-coded `true` (binding rule: bits stay = MI, never multiplied by raw frequency).

---

## 3. Panel sufficiency (`sufficiency.rs`)

### 3.1 The metric and threshold

Sufficiency compares the panel's information about the outcome against the outcome's entropy:

- `deficit_bits = (anchor_entropy_bits - panel_bits).max(0.0)`
- `sufficient = panel_bits >= anchor_entropy_bits`

So the **threshold is `H(anchor)` itself** (`I(panel; anchor) >= H(anchor)`): a panel is sufficient iff it carries at least as many bits as the outcome's entropy. There is no slack/margin constant — equality suffices. `entropy_bits(labels)` computes `H = Σ -p·log2(p)` over label counts (base-2). The DPI ceiling is `I(panel;anchor)` itself (`dpi_ceiling`, §1.7).

### 3.2 `PanelSufficiency` and deficit types

`PanelSufficiency` fields: `panel_bits: f32`, `anchor_entropy_bits: f32`, `sufficient: bool`, `deficit_bits: f32`, `deficits: Vec<SufficiencyDeficit>`, `trust: TrustTag`.

Entry points (all build the same struct):
- `panel_sufficiency(panel_bits, anchor_entropy_bits, slots, trust)` — forces trust `Provisional`.
- `panel_sufficiency_with_anchor(.., anchor)` — trust from anchor.
- `panel_sufficiency_with_context(.., trust, context)` / `panel_sufficiency_with_anchor_and_context(.., anchor, context)` — add a `DeficitRoutingContext`.

`DeficitRoutingContext { panel_id: String, anchor: AnchorKind, computed_at_seq: u64 }` (`Default`: `panel:unspecified`, `AnchorKind::Reward`, seq 0).

`SufficiencyDeficit` fields: `panel_id: String`, `anchor: AnchorKind`, `slot: Option<SlotId>`, `per_slot_gaps: BTreeMap<SlotId, f32>`, `deficit_bits: f32`, `suggested_action: DeficitSuggestedAction`, `computed_at_seq: u64`, `reason: String`.

`DeficitSuggestedAction` (serde `snake_case`): `AddOutcomeAnchor`, `ProposeLens`, `IncreaseSamples`.

### 3.3 Localized deficit routing (`localized_deficits`)

When `!sufficient`:
- **No slots:** one deficit with `slot: None`, action `AddOutcomeAnchor`, reason "panel below anchor entropy".
- **With slots:** one deficit per slot, action `ProposeLens`, reason "slot marginal bits below sufficiency need". The total `deficit_bits` is split across slots **inversely proportional to each slot's marginal bits**: `weight_i = 1/(marginal_bits_i + 0.01)`, slot share `= deficit_bits · weight_i / Σ weight`. `per_slot_gaps` holds the same per-slot split. So the slots carrying the fewest bits absorb the largest "missing bits" attribution — pointing Anneal at where to add a lens.

Routing: `PanelSufficiency::route_to(sink)` pushes each deficit into a `SufficiencyDeficitSink`. `InMemoryDeficitSink { routed: Vec<SufficiencyDeficit> }` is the default in-memory implementation. (Cross-ref: deficits feed Anneal's lens-proposal path — see `15_anneal_optimization.md`.)

### 3.4 Per-sensor attribution (`attribution.rs`)

`SlotAttribution { slot: SlotId, marginal_bits: f32, sole_carrier: bool }`. `per_sensor_attribution(slot_bits: &[(SlotId, f32)], sole_threshold_bits)`: a slot is `sole_carrier` iff its bits `>= sole_threshold_bits` **and it is the only slot above that threshold** (`strong_slots == 1`). `BitsReport { slots, total_bits, trust }`; `bits_report(slots, trust)` (forces `Provisional`), `bits_report_with_anchor(slots, anchor)`; `total_bits = Σ marginal_bits`.

---

## 4. Panel-level information measures

### 4.1 Total correlation & interaction information (`total_correlation.rs`)

`TC(Φ) = Σ_k H(slot_k) − H(Φ)` — multivariate redundancy of a panel. Constants: `CALYX_TC_INSUFFICIENT_SAMPLES`, `MIN_QUORUM_TC_PER_SLOT = 50`, `DEFAULT_TC_K = 3`, `DEFAULT_TC_BOOTSTRAP_RESAMPLES = 500`.

`TotalCorrelationConfig { k, bootstrap_resamples }` (default 3 / 500). Quorum: `min_quorum_tc(slot_count) = 50 · slot_count`; below quorum a **provisional** `TCResult` is returned (not an error). `TCResult` fields: `tc, n_eff, ci_95: (f32,f32), n_samples, slot_count, sum_marginal_entropy, joint_entropy, provisional, error_code: Option<String>, trust, computed_at: Ts`. `n_eff_from_tc(slot_count, tc, sum_marginal_entropy)` derives an effective non-redundant lens count `≈ n·(1 − TC/Σ H_marginal)`, clamped to `[1, n]`.

`interaction_information[_with_config](a, b, c, clock, ..) -> IIResult` (three slots). `IIResult { ii, sign: IISign, ci_95, n_samples, provisional, error_code, trust, computed_at }`. `IISign` ∈ `{ Redundant, Synergistic, Unclear }` (decided from the CI straddling zero). Requires `n >= 150` (`MIN_QUORUM_TC_PER_SLOT*3`) and `>= 50`. All TC/II results are tagged `TrustTag::Provisional` regardless of anchor.

### 4.2 Effective rank (`n_eff.rs`)

`stable_rank(matrix) -> NeffReport { n_eff, trace, frobenius_sq }` where `n_eff = trace² / Σ a_ij²` (the stable rank of a redundancy/Gram matrix, 0 if Frobenius² is 0). This is the A9 "non-redundant lens count" used for sizing/cost budgets.

### 4.3 Transfer entropy (`transfer_entropy.rs`)

`T(A→B) = I(B_future; A_past, B_past) − I(B_future; B_past)`, estimated by reusing the KSG point estimator on lagged samples. Constants: `CALYX_TE_INSUFFICIENT_SAMPLES`, `MIN_TE_QUORUM = 30`, `DEFAULT_TE_WINDOW = 1`, `DEFAULT_TE_K = 3`, `DEFAULT_TE_BOOTSTRAP_RESAMPLES = 500`, `DEFAULT_TE_BOOTSTRAP_SEED = 52`, `DEFAULT_TE_LAGS = [1,2,4,8]`. `TransferEntropyConfig { window_size, k, bootstrap_resamples, bootstrap_seed }`. `Direction` ∈ `{ AToB("A_to_B"), BToA("B_to_A"), Unclear }`. `TEResult` carries forward/reverse TE, dominant direction, three CIs, lag, window, `provisional`, `n_samples`, `error_code`, `trust`, `computed_at`. Effective quorum: `max(MIN_TE_QUORUM, MIN_ASSAY_SAMPLES) = 50`. Sweep helpers: `transfer_entropy_sweep[_with_config]`, `max_transfer_entropy_lag`. Types `Timestamp = Ts`, `RecurrenceStream = [(Ts, f32)]`.

### 4.4 Other temporal / drift primitives (briefly)

- **Periodicity** (`periodicity.rs`): generalised floating-mean **Lomb–Scargle** GLS periodogram + slotted autocorrelation, permutation FAP. Constants: `MIN_PERIODICITY_SAMPLES = 8`, `DEFAULT_PERIODOGRAM_OVERSAMPLE = 10.0`, `DEFAULT_FAP_PERMUTATIONS = 100`, `DEFAULT_PERIODICITY_SEED = 0`, `DEFAULT_MAX_PEAKS = 3`, `SIGNIFICANT_PEAK_FAP = 0.01`, `MAX_FREQUENCY_GRID = 1<<20`, `MAX_ACF_SAMPLES = 8192`. Types: `PeriodogramConfig`, `PeriodogramPeak`, `PeriodicityReport`, `AutocorrelationReport`.
- **Recurrence hazard** (`recurrence_hazard.rs`): Gamma-renewal "overdue" hazard + Page two-sided CUSUM rate change-point. Constants: `MIN_HAZARD_GAPS = 3`, `MIN_CUSUM_GAPS = 4`, `DEFAULT_OVERDUE_ALPHA = 0.05`, `CV_DETERMINISTIC = 1e-6`, `DEFAULT_CUSUM_SLACK_K = 0.5`, `DEFAULT_CUSUM_THRESHOLD_H = 5.0`, `DEFAULT_MIN_SIGMA_FRAC = 1e-3`. Types: `InterEventHazardReport`, `RateShift{SpeedUp,SlowDown}`, `CusumChangePoint`, `CusumConfig`, `CusumReport`.
- **MMD drift** (`mmd.rs`): Gaussian-kernel two-sample MMD² with permutation p-value + change-point scan. Constants: `MIN_MMD_SAMPLES = 4`, `MAX_MMD_SAMPLES = 2048`, `DEFAULT_MMD_PERMUTATIONS = 99`, `DEFAULT_MMD_ALPHA = 0.01`, `DEFAULT_MMD_SEED = 609`. Types: `MmdConfig`, `MmdReport`, `ChangePointReport`.
- **Random projection** (`projection.rs`): deterministic ±1 (blake3-seeded) random projection pre-step before KSG on high-d slots. `target_projection_dim(rows, dim) = min(dim, max(1, 2·ceil(log2 rows)))`. `project_cpu` works; `project_gpu` is a stub returning `forge_device_unavailable`.

---

## 5. Recurrence anchors & oracle self-consistency (`recurrence_anchor.rs`)

Implements grounded "frequency-as-anchor" and oracle flakiness measurement from recurrence.

Constants: `CALYX_ASSAY_MISSING_OUTCOME_SLOT`, `DEFAULT_OUTCOME_ANCHOR_LABEL = "OutcomeAnchor"`, `OUTCOME_CONTEXT_FIELD = "outcome_anchor"`, **`CONSISTENT_AGREEMENT_THRESHOLD = 0.75`**.

- `RecurrenceAnchor { cx_id, frequency: u64, cadence_secs: Option<f64> }`; `frequency_anchor_for(cx_id, vault)` reads the `FREQUENCY_SCALAR` from the constellation base row.
- `Domain { id: String, cx_ids: Vec<CxId>, outcome_anchor: AnchorKind }`; default outcome anchor is `AnchorKind::Label("OutcomeAnchor")`.
- `OutcomeAgreement` ∈ `{ Consistent{agreement_rate}, Flaky{agreement_rate}, Insufficient{n} }`. Agreement is the **pairwise fraction of equal outcomes** over all `C(n,2)` occurrence pairs; `< 3` occurrences ⇒ `Insufficient`. `classify_agreement`: `>= 0.75` ⇒ `Consistent`, else `Flaky`.
- `oracle_self_consistency(domain, vault)` averages agreement rates over cx_ids with `frequency >= 3` (skips others); `oracle_self_consistency_from_agreements` averages over non-`Insufficient` entries, returning `1.0` when none. This is the `τ_corr` ceiling that caps predictor confidence (cross-ref `16_oracle_prediction.md`).
- `outcome_occurrence_context(kind, value)` encodes a grounded outcome observation into an Aster `OccurrenceContext`; `validate_anchor_value` rejects non-finite numbers/vectors. Errors use `CALYX_ASSAY_MISSING_OUTCOME_SLOT` (remediation: attach a grounded OutcomeAnchor per occurrence).

### 5.1 Bayesian posteriors (`bayesian.rs`)

Conjugate small-sample posteriors persisted in the Assay CF.

Constants: `CALYX_BAYES_INVALID_INTERVAL`, `DEFAULT_BAYES_PRIOR_ALPHA = 1.0`, `DEFAULT_BAYES_PRIOR_BETA = 1.0`, `BAYESIAN_POSTERIOR_KEY_PREFIX = b"bayesian/posterior/v1"`.

- `GammaPoisson { alpha, beta }` — recurrence rate posterior; `update(events, interval)`, `mean_rate`, `credible_interval[_95]`, `next_occurrence_expected`.
- `BetaBernoulli { alpha, beta }` — consistency/flakiness posterior; `update(successes, failures)`, `mean_consistency`, `reliability_probability(threshold)`, `is_reliable(threshold, confidence)`, `credible_interval[_95]`. Quantiles use a regularized incomplete beta / gamma via `special_fn` (`gammp`, `gammq`, `ln_gamma`).
- `BayesianPosteriorRow { domain_id, outcome_anchor, gamma_poisson, beta_bernoulli, written_at_seq: Seq }`; `bayesian_posterior_key(domain)`, `persist_bayesian_posterior`, `bayesian_posterior_for_domain`, `gamma_poisson_for_domain`, `beta_bernoulli_for_domain` (the latter two return defaults if absent). Stored in `ColumnFamily::Assay`.

---

## 6. "Signal bits" persistence (`store.rs`)

### 6.1 Where it lives

Assay rows are written to the Aster **`ColumnFamily::Assay`** (CF numeric id **6** — `crates/calyx-aster/src/cf/family.rs`, `cf_codec.rs`). See `04_storage_and_schema.md` and `06_aster_storage_engine.md`. `AssayStore` is an in-memory `BTreeMap<(AssayCacheKey, AssaySubject), AssayRow>` that round-trips through the CF (`persist_to_aster` via `CfRouter`, `persist_to_vault` / `load_from_vault` via `AsterVault`, `load_from_aster`).

### 6.2 Cache key (`AssayCacheKey`)

| Field | Type | Notes |
|---|---|---|
| `vault_id` | `Option<VaultId>` | **Must be `Some`** before persistence (`require_scoped`); else `CALYX_VAULT_ACCESS_DENIED`. |
| `anchor` | `AnchorKind` | The outcome the bits are about (default `Reward`). |
| `panel_version` | `u32` | Panel generation; `invalidate_panel(v)` drops all rows for a version. |
| `corpus_shard` | `String` | The shard the estimate was computed on (provenance/reproducibility). |

`AssayCacheKey::scoped(panel_version, corpus_shard, vault_id, anchor)` is the required constructor. `AssayCacheKey::new(..)` is `#[deprecated]` (unscoped; rejected at persist/load).

### 6.3 Subject, row, and on-disk key layout

`AssaySubject` (serde `snake_case`): `Lens { slot: SlotId }`, `Pair { a: SlotId, b: SlotId }`, `Panel`, `OutcomeEntropy`.

`AssayRow { cache_key: AssayCacheKey, subject: AssaySubject, estimate: MiEstimate, provenance: String, written_at_seq: u64 }`. The value is the JSON-serialized `AssayRow`.

The CF **key** (`assay_key`) is binary, big-endian, length-prefixed (and is re-derived on load and checked against the row — a mismatch errors `CALYX_ASTER_CORRUPT_SHARD`):
```
panel_version: u32 BE
len-prefixed: vault_id (string)
len-prefixed: anchor (serde_json bytes of AnchorKind)
len-prefixed: corpus_shard bytes
subject tag byte:
  0x00 Lens  + slot u32 BE
  0x01 Pair  + a u32 BE + b u32 BE
  0x02 Panel
  0x03 OutcomeEntropy
```
(`SlotId::get()` is serialized via `to_be_bytes`; len prefixes are `u32` BE.)

`AssayStore` API: `put`, `get`, `cache_hit`, `invalidate_panel`, `rows`, `len`, `is_empty`. The `bits_about` map mentioned in the plan is **not** a separate type here — the per-anchor bits live as `Lens`-subject rows keyed by anchor (one row per `(vault, anchor, panel_version, shard, slot)`); panel sufficiency lives in `Panel`/`OutcomeEntropy` rows.

### 6.4 Loom materialization gate (`loom_adapter.rs`)

`AsterAssayMaterializationGate<'a, S: VaultStore>` reads slot vectors + a grounded anchor for a set of `CxId`s at a snapshot and feeds `AssayGate::pair_gain` into `calyx_loom::plan_cross_terms_checked` to produce a `MaterializationPlan` (cross-ref `10_loom_associations.md` / `06`). `materialization_plan` propagates errors; `materialization_plan_fail_safe_lazy` substitutes `0.0` gain on any failure and records the last error. `anchor_bool` requires a grounded anchor (non-blank source, confidence in `(0,1]`) and a `Bool` or finite-`Number(>0)` value, else `CALYX_ASSAY_INSUFFICIENT_SAMPLES`. Slot vectors must be `Dense` or `Sparse` (`Multi`/`Absent` ⇒ `CALYX_STALE_DERIVED`).

---

## 7. Formula coverage catalog (`formula_catalog.rs`)

A self-verification artifact listing every PRD-22 Assay formula and the test that exercises it. Constants: `FORMULA_COVERAGE_SURFACE = "formula-coverage"`, `FORMULA_COVERAGE_ARTIFACT_KIND = "prd22.formula-coverage.v1"`, `FORMULA_COVERAGE_SCHEMA_VERSION = 1`, `FORMULA_COVERAGE_SOT_KEY = "formula_coverage/prd22"`, `CALYX_FORMULA_COVERAGE_MISSING`. Types: `FormulaCoverageStatus{Covered,Missing}`, `FormulaCoverageArtifact`, `FormulaCoverageRow`, `FormulaCoverageSummary`, `FormulaRowSpec`. `validate_formula_coverage` errors `CALYX_FORMULA_COVERAGE_MISSING` if any row is missing or the row count/schema mismatch the built-in `FORMULA_ROWS`.

---

## 8. Error taxonomy

Defined in `crates/calyx-core/src/error.rs`; `CalyxError` is `{ code: &'static str, message: String, remediation: &'static str }`.

| Code | Raised when |
|---|---|
| `CALYX_ASSAY_INSUFFICIENT_SAMPLES` | `n < 50` (or estimator-specific quorum), length mismatch, non-finite, bad `k`, ungrounded anchor where trust required, non-binary anchor in materialization gate. |
| `CALYX_ASSAY_LOW_SIGNAL` | `signal_bits` non-finite or `< 0.05`. |
| `CALYX_ASSAY_REDUNDANT` | `max_pairwise_corr` non-finite or `> 0.6`. |
| `CALYX_TC_INSUFFICIENT_SAMPLES` / `CALYX_TE_INSUFFICIENT_SAMPLES` | TC/TE config invalid (below-quorum returns a *provisional result*, not these errors). |
| `CALYX_ASSAY_MISSING_OUTCOME_SLOT` | recurrence occurrence lacks/mismatches an outcome-anchor observation. |
| `CALYX_BAYES_INVALID_INTERVAL` | non-finite/non-positive Bayesian interval or negative counts. |
| `CALYX_FORMULA_COVERAGE_MISSING` | formula-coverage artifact incomplete/mismatched. |
| `CALYX_VAULT_ACCESS_DENIED` | persisting/loading an unscoped (no `vault_id`) Assay row. |
| `CALYX_ASTER_CORRUPT_SHARD` | Assay CF key↔row mismatch, decode failure, sparse entry out of range. |
| `CALYX_STALE_DERIVED` | materialization gate: slot/anchor missing for a cx, or non-measured slot vector. |
| `CALYX_FORGE_DEVICE_UNAVAILABLE` | `project_gpu` (unimplemented). |

---

## 9. Gaps / not covered

- **No Forge/GPU execution.** The plan states estimators "run in Forge … batched on GPU"; the crate has no Forge dependency and `project_gpu` is an explicit stub (`CALYX_FORGE_DEVICE_UNAVAILABLE`, "until PH28 GPU projection is implemented"). All math is CPU, O(n²) brute-force for KSG/NMI.
- **Thresholds are not config-overridable in this crate.** `MIN_SIGNAL_BITS = 0.05` and `MAX_PAIRWISE_CORR = 0.6` are hard-coded `const`s; the plan's "config-overridable per vault" is not implemented here.
- **Logistic "probe" is a mean-difference linear classifier**, not a fitted/calibrated logistic regression; reported bits = confusion-matrix MI between label and prediction, not a calibrated cross-entropy reduction as the plan describes.
- **No automatic park/retire/re-check loop in this crate.** `admit_lens` is a pure gate; the "auto-park on drift, Anneal re-check" behavior described in the plan is not present in calyx-assay (it would live in calyx-anneal — `15`).
- **No graph-based redundancy pruner.** Redundancy is a per-candidate max-correlation/NMI gate; `n_eff` reports but does not prune.
- **`admit_lens` never returns `admitted: false`** — rejection is always via `Err`; callers must treat the error codes as the reject reasons.
- TC/II/TE results are always `TrustTag::Provisional` regardless of anchor grounding.
