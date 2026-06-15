# 22 — The Formula Library (Baked Into the Backend)

Implements A22. The founder's mandate: **bake the formulas into the database and backend** so the DB extracts associations and intelligence **automatically** — the user never re-derives or re-implements them. Every formula is from the Royse corpus (Doctrine §2), implemented in `calyx-forge` (math) + the named engine, callable, provenanced, self-optimized by Anneal.

Notation: `n` inputs, `N` lenses, slot vectors `v_k`, anchor/outcome `Y`, panel `Φ = {v_1..v_N}`.

## 1. Derived Data Abundance (Loom, `06`)

| Formula | Definition | Where coded | Notes |
|---|---|---|---|
| **DDA yield** | `signals(n,N) = n·(N + C(N,2) + 1)` | `loom::abundance` | upper bound; the `C(N,2)` are cross-terms (associations between associations) |
| **Cross-term** | `x_{ab} ∈ {concat, v_a⊙v_b, cos(v_a,v_b), v_a−v_b}` | `loom::cross_term` | lazy unless Assay-gated (`06 §4`) |
| **Meaning compression yield** | `mc(input) = (materialized signals) / 1 input` | `loom::meaning_compression` | per-input intelligence extracted; complements compression-as-intelligence |

## 2. The differentiation contract (Assay, `07`)

| Formula | Definition | Threshold (verbatim) | Where |
|---|---|---|---|
| **Per-lens signal** | `I(v_k; Y)` mutual information | admit iff **≥ 0.05 bits** | `assay::lens_signal` |
| **Pairwise redundancy** | `corr(v_a,v_b)` / normalized MI | admit iff **≤ 0.6** | `assay::pair_redundancy` |
| **KSG estimator** | k-NN MI (Kraskov–Stögbauer–Grassberger) | k default 3–5; bootstrap CI | `forge::knn` + `assay::ksg` |
| **Partitioned NMI** | histogram normalized MI, streaming | `partitioned_histogram_nmi_v1` | `assay::nmi` |
| **Effective rank** | `n_eff` = non-redundant lens count (stable rank of the redundancy graph) | — | `assay::n_eff` |
| **Marginal lens value** | `I(Φ;Y) − I(Φ∖k;Y)` | — | `assay::marginal_value` |

## 3. The bound (everywhere — Doctrine §9)

| Formula | Definition | Where |
|---|---|---|
| **DPI ceiling** | any predictor reading `Φ` has `I(predictor; Y) ≤ I(Φ; Y)` | `assay::dpi_ceiling` |
| **Panel sufficiency** | `I(Φ; Y)` vs `H(Y)`; sufficient iff `I(Φ;Y) ≥ τ_MI` | `assay::panel_sufficiency` |
| **Per-sensor decomposition** | each slot's marginal bits + redundancy + sole-carrier flag | `assay::attribution` |
| **Abundance honesty** | report `C(N,2)` as *upper bound under approximate independence, capped at `n_eff`* | `loom::abundance_report` |

## 4. The grounding kernel (Lodestar, `08`) — at any scope

| Formula | Definition | Where |
|---|---|---|
| **Association graph** | directed `G`: `a→b` from agreement × directional confidence + citation/entity edges | `lodestar::build_graph(scope)` |
| **Kernel graph (~10%)** | high in/out-degree + betweenness + low groundedness-distance; LP rounding | `lodestar::kernel_graph` |
| **Grounding kernel (~1%)** | approximate **minimum feedback vertex set** of `G` (directed FVS) | `mincut::dfvs_approx` |
| **MFVS approx** | LP-relaxation `O(log τ* log log τ*)`; tournament 2-approx; bounded-genus `O(g)` | `mincut::*` |
| **Kernel-only recall** | reconstruct/answer held-out from kernel; `recall_kernel / recall_full` (gate ≥ 0.95) | `lodestar::recall_test` |
| **Hop attenuation** | path score `× 0.9^hop` along association edges | `paths::attenuate` |
| **Grounding gaps** | kernel members not reaching an anchor → cheapest label plan | `lodestar::grounding_gaps` |

**Scope is a parameter** (A21): `scope ∈ {AllAssociations, Collection(id), Domain(anchor), Subgraph(query), TimeWindow(t0,t1), Tenant(id), Custom(filter)}`. The same MFVS runs on whatever subgraph the operator selects — freedom of scope (`08 §x`).

## 5. Teleological Constellation Training / the guard (Ward, `09`)

| Formula | Definition | Where |
|---|---|---|
| **`Gτ` guard** | pass slot k iff `cos(produced_k, matched_k) ≥ τ_k` | `ward::guard` |
| **Constellation pass** | all required slots pass (or `KofN`) | `ward::verdict` |
| **τ calibration** | conformal: choose `τ_k` to bound false-accept rate at confidence `1−α` | `ward::calibrate` |
| **Novelty → new region** | fail ⇒ new safe region (not silent accept) | `ward::novelty` |

## 6. The Oracle (Oracle/AGI, `21`)

| Formula | Definition | Where |
|---|---|---|
| **Oracle self-consistency ceiling** | `τ_corr ≤ oracle_self_consistency`; decompose into flakiness + validity | `oracle::ceiling` |
| **Consequence prediction** | JEPA step: `(panel_t, action) → panel_{t+1}/outcome` | `oracle::predict` |
| **Butterfly expansion** | recurse consequences with hop attenuation | `oracle::expand` |
| **Super-intelligence predicate** | 6-tier `∧` (clean ∧ sufficient ∧ kernel ∧ calibrated ∧ goodhart ∧ mistake-closed) | `oracle::super_intelligence` |
| **Sufficiency falsification** | refuse confident prediction if `I(Φ;Y) < H(Y)`; return deficit | `oracle::predict` (honesty gate) |

## 7. Search & navigation (Sextant, `10`)

| Formula | Definition | Where |
|---|---|---|
| **RRF** | `Σ_i weight_i / (rank_i + 60)` across lenses | `sextant::rrf` |
| **Weighted RRF** | per-intent weight profile | `sextant::weighted_rrf` |
| **ColBERT MaxSim** | `Σ_q max_d (q·d)` late interaction | `forge::maxsim` |
| **Causal gate** | high-confidence causal ×1.10, low ×0.85 (post-retrieval) | `sextant::causal_gate` |
| **Cross-lens anomaly** | high in lens a, low in lens b vs neighborhood → blind-spot | `loom::blind_spots` |
| **Define (Gärdenfors)** | term = constellation other lenses form at one lens's index | `sextant::define` |

## 8. Epistemic symmetry (Oracle/AGI §5)

| Formula | Definition | Where |
|---|---|---|
| **Reverse query** | answer → question/cause via asymmetric back-edges + kernel-toward-antecedents | `oracle::reverse_query` |
| **Q↔A equivalence** | bidirectional traversal of the association graph | `paths::bidirectional` |

## 9. Self-optimization (Anneal, `12`) — the formulas tune themselves

Every formula above has tunable parameters (k in KSG, `ef`/`M` in ANN, `60` in RRF, `0.9` hop attenuation, `τ_k`, MFVS approximation depth, quant level). **Anneal autotunes each per workload**, A/B against the incumbent on live traffic, promotes only on a measured win with no tripwire regression, all reversible + Ledger-logged. The math doesn't just run — it **optimizes itself for the job the function is being used on** (the founder's requirement).

## 10. Determinism, provenance, honesty (binding)

- Every formula runs in Forge with a **CPU-SIMD ↔ CUDA bit-parity** contract (A13) + a determinism mode for replay (`11`).
- Every computed number carries **sample count + CI + estimator + corpus-shard provenance** (Ledger); below quorum it **fails closed** (A16), never a noisy point estimate.
- Every "trusted" number requires **grounding** (A2); ungrounded results are tagged `provisional`.
- No formula reports abundance beyond the **DPI ceiling** (A8).

## 11. One table to find any formula

| I want to… | Call | Engine |
|---|---|---|
| Count signals from my data | `abundance_report` | Loom |
| Know a lens's worth | `lens_signal`, `marginal_value` | Assay |
| Know if my panel can even answer | `panel_sufficiency`, `dpi_ceiling` | Assay |
| Find the kernel of *anything* | `build_kernel(scope)` | Lodestar |
| Guard a generation | `guard`, `calibrate` | Ward |
| Predict consequences | `oracle_predict`, `expand` | Oracle |
| Reverse an answer to its question | `reverse_query` | Oracle |
| Search across everything | `search`, `kernel_answer`, `ASK` | Sextant |
| Tune the math to my workload | automatic | Anneal |

**One sentence:** every formula in the Royse calculus — DDA, the differentiation contract, the DPI bound, the multi-scope grounding kernel, `Gτ`, the Oracle's self-consistency ceiling and sufficiency test, RRF, and Q↔A reversibility — is a baked-in, callable, grounded, self-tuning backend primitive, so the database extracts intelligence automatically and the user never writes the math again.
