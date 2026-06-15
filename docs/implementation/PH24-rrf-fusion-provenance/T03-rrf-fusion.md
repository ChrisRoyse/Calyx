# PH24 · T03 — RRF `Σ w/(rank+60)` fusion

| Field | Value |
|---|---|
| **Phase** | PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/fusion/rrf.rs` (≤500) |
| **Depends on** | T02 (this phase) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/10 §2` |

## Goal

Implement Reciprocal Rank Fusion: `fused_score(cx) = Σ_i weight_i / (rank_i + 60)`
summed over all slots where the cx appears. This is the general multi-lens
strategy (`10 §2`) and the one that must achieve recall@10 ≥ single-lens + Δ
(≥15%) on the real qrels. The constant 60 is the standard Cormack et al. value
and must not be configurable here.

## Build (checklist of concrete, code-level steps)

- [x] `RrfStrategy` struct with `weights: HashMap<SlotId, f32>` (default weight
      1.0 if slot not in map)
- [x] `fn fuse(&self, ctx: &FusionContext) -> Result<Vec<Hit>, CalyxError>`:
      1. For each slot in `ctx.query_vecs`, call `map.search(slot, vec, k * OVER_FETCH, ef)`
         where `OVER_FETCH = 3` (retrieve 3× to give RRF enough candidates)
      2. Assign ranks: rank 0 = highest raw_score for that slot
      3. Accumulate per-cx: `score += weight / (rank as f32 + 60.0)`,
         record `PerLensEntry { slot, rank: rank as u32, raw_score, weight, contribution }`
      4. Sort final `HashMap<CxId, (f32, Vec<PerLensEntry>)>` by fused_score desc
      5. Take top-k, construct `Hit` for each
      6. If `ctx.explain`, keep full `per_lens`; if not, keep it anyway (cheap)
- [x] `CALYX_SEXTANT_NO_LENSES` if `ctx.query_vecs` is empty
- [x] Hits that appear in only 1 slot are still valid (partial participation)
- [x] Wire `FusionStrategy::Rrf` in dispatcher → `RrfStrategy` with uniform weights

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 2 slots, 10 vecs each, query → confirm a cx that ranks #1 in both
      slots has the highest fused score (= `1/(1+60) + 1/(1+60) = 0.0328...`)
- [x] unit: cx that appears in only 1 slot ranks lower than one appearing in
      both (all else equal)
- [x] unit: `per_lens` entries have correct `contribution = weight / (rank+60)`
      for each slot, within f32 tolerance 1e-6
- [x] proptest: fused_score is non-negative for any inputs
- [x] proptest: result list is sorted descending by fused_score
- [x] edge: only 1 slot → results match SingleLens with weight=1.0 (up to
      OVER_FETCH ordering differences; assert same top-1)
- [x] edge: query_vecs empty → `CALYX_SEXTANT_NO_LENSES`
- [x] fail-closed: a slot returns an error → propagated as
      `CALYX_SEXTANT_SLOT_NOT_FOUND`, fusion aborts (not partial)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant rrf_fusion -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant rrf_fusion -- --nocapture 2>&1`
- **Prove:** test prints `cx_in_both_slots fused=0.032786... cx_in_one_slot fused=0.016393...`
  (exact values for k=60 denominator, validated against formula)

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH24 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
