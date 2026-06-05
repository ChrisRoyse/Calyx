# 06 — Loom: the Derived Data Abundance Engine

> **Living-system role:** cognition — weaving associations between associations is the act of thinking (A31 — DOCTRINE §1b)

Implements the paper's central productivity claim and its bound (A8/A9). Loom weaves cross-terms — associations between associations.

## 1. The DDA identity

n real inputs through N frozen, approximately-independent lenses yield up to:

```
signals(n, N) = n · ( N + C(N,2) + 1 )
              = n · ( N + N(N-1)/2 + 1 )
```

- `N` — the per-lens slot measurements.
- `C(N,2)` — the **cross-terms**: one association between each pair of lenses (the load-bearing novelty; associations *between* associations).
- `+1` — the whole-panel constellation signal (the joint, e.g. the named region).

For the shipped panels: text `N=13 → 13+78+1 = 92` signals/input (matches ContextGraph's stated 92); civic `N=21 → 21+210+1 = 232`; code `N=15 → 15+105+1 = 121`; media/ClipCannon `N=7 → 7+21+1 = 29`.

### Meaning compression (named, tracked)
The per-input yield — the `N + C(N,2) + 1` grounded signals Loom derives from **one** real input — is **meaning compression** (paper §, complementary to the compression-as-intelligence tradition of Solomonoff/Hutter). The classic tradition compresses a *corpus* to a short program; Calyx runs it the other way, **expanding one input into a rich, grounded, differentiated signal set**. Loom exposes a per-input `meaning_compression_yield` (signals materialized / input) in `abundance_report`, so value extracted per real datum is a measured quantity, not a slogan. The database-level answer to the data wall (§7): squeeze more grounded signal out of each licensed, real input instead of generating synthetic data.

## 2. The bound (stated plainly, enforced)

`C(N,2)` is an **upper bound under approximate independence, capped by the data-processing inequality and the panel's effective rank** (A8). Loom never pretends a cross-term is free information:

- A cross-term between two highly-correlated lenses carries ~0 new bits → Loom does **not** materialize it (Assay-gated, `07`).
- Total trustworthy abundance is bounded by `I(panel; outcome)` (DPI). Loom exposes this ceiling; it does not sell `C(N,2)` as if all pairs were independent.
- Realized gain = **reduced sample complexity, realized up to the panel's effective rank** `n_eff` (A9). Loom's materialization budget scales with `n_eff`, not raw `N`.

This honesty makes Calyx's "abundance" defensible (the paper's stated caveat, and ContextGraph's `dda-dpi-honesty-reframe`).

## 3. Cross-term kinds (recap from `03 §5`) and when each is used

| Kind | Cost | Materialize when |
|---|---|---|
| `Agreement` = `cos(v_a,v_b)` (scalar) | cheap | **always** for active pairs — drives blind-spot/anomaly detection and `n_eff`; tiny to store |
| `Delta` = `v_a − v_b` | medium | directional contrast queries; asymmetric-axis pairs |
| `Interaction` = blockwise `v_a ⊙ v_b` or low-rank `v_aᵀW v_b` | medium/high | pair shows MI gain about an outcome beyond either alone |
| `Concat` = `[v_a‖v_b]` (typed, reversible, indexable) | high (storage+ANN) | a region defined jointly by two axes is queried often (Sextant promotes it) |

## 4. Materialization policy (lazy by default)

```
plan_cross_terms(cx, panel) -> MaterializationPlan:
  active_pairs = pairs(slots where state=Active)
  for (a,b) in active_pairs:
    agreement(a,b)  := EAGER (scalar, ~free)             # always
    if Assay.pair_gain(a,b | anchor) ≥ 0.05 bits:        # non-redundant, outcome-relevant
        interaction(a,b) := EAGER for hot pairs, else LAZY
    if Sextant.promotes(Concat,a,b):                     # query pattern justifies the index
        concat(a,b) := EAGER + ANN-indexed
    else: LAZY  (compute on demand, cache with TTL)
```

- **Lazy** cross-terms are computed at query time from the two stored slot-vectors and LRU-cached; cost nothing at rest.
- **Eager** cross-terms are written to the `xterm` CF and (for Concat) ANN-indexed.
- The plan is **per-pair, per-anchor, adaptive** — Anneal re-runs it as query patterns and bits shift (`12`).

Result: storage is `O(n · n_eff)` not `O(n · N²)`, while *queryability* of all `C(N,2)` pairs is preserved (any lazy pair is one matmul away).

## 5. The agreement graph (a first-class output)

For each constellation Loom emits the symmetric **agreement vector** `[cos(v_a,v_b)]` over active pairs. Aggregated across the vault, this yields:

- **Redundancy graph** — pairs with high mean agreement → feeds `n_eff` and the ≤0.6 admission gate (`07`).
- **Blind-spot detector** — a constellation high-similarity in lens A but low in lens B vs its neighbors is a **cross-lens anomaly** (absorbed from ContextGraph `search_cross_embedder_anomalies`): exposed as a query (`10`) and a self-check (`12`).
- **Kernel-graph seed** — the agreement/definition graph is the directed graph Lodestar runs MFVS on (`08`).
- **Temporal co-occurrence cross-terms (A29, `25 §4c`)** — DDA extended across time: when two recurring events' occurrence times correlate (A recurs shortly before B), that is a grounded, directional **temporal association** — a cross-term between events' recurrence patterns. Feeds causal discovery and the Oracle's consequence prediction (`21`). Computed from the recurrence series, not the temporal retrieval lenses.

## 6. Compute placement

Cross-terms are linear algebra over slot vectors → run in Forge:
- `Agreement`: one batched normalized matmul per microbatch on sm_120 (32 GB VRAM holds large pair-blocks).
- `Interaction` low-rank: `W` is a small learned/random matrix per pair, applied batched.
- CPU SIMD path is bit-parity tested for embedded vaults without GPU (A13).

## 7. DDA as the data-wall answer (framing)

The paper's economic claim: the wall is hit by counting tokens, not associations. Loom is where Calyx "counts associations": from `n` real, licensed, non-synthetic inputs it derives up to `n·(N+C(N,2)+1)` grounded signals **without generating synthetic data** (no model-collapse recursion). Loom MUST tag every signal as `measured` (real input through a frozen lens) vs `derived` (a cross-term) — never as new external data. This keeps Calyx on the third path (measure more associations) and out of the **model-collapse** recursion that recursive self-generation causes (Shumailov et al., *Nature* 2024) — Calyx never trains on generator-synthesised data; it expands real inputs.

## 8. Loom API (summary; full in `18`)

```
weave(cx_id) -> AgreementVector + materialized xterms per plan
pair_value(a, b, anchor) -> bits gain (delegates to Assay)
cross_term(cx_id, a, b, kind) -> value            # lazy-computes + caches if not materialized
agreement_graph(vault, since_seq?) -> sparse adjacency (for Lodestar/Anneal)
blind_spots(cx_id | query) -> [pairs where lenses disagree vs neighborhood]
abundance_report(vault) -> { N, C(N,2), materialized, n_eff, I(panel;outcome) ceiling }
```

`abundance_report` is the honest dashboard: signals *in principle* (`C(N,2)`), *materialized*, *effective* rank, *DPI ceiling* — the four numbers that keep DDA truthful.
