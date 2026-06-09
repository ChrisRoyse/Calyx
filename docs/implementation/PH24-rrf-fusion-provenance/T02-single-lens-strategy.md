# PH24 · T02 — `SingleLens` fusion strategy

| Field | Value |
|---|---|
| **Phase** | PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/fusion/single.rs` (≤500), `crates/calyx-sextant/src/fusion/mod.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH23 T06 (`SlotIndexMap`) |
| **Axioms** | A16, A17 |
| **PRD** | `dbprdplans/10 §2` |

## Goal

Implement the `SingleLens` strategy — one slot's ANN results converted directly
into `Hit` values. This is the lowest-latency path (`10 §2`) and the baseline
for the recall comparison that drives the PH24 FSV gate. It also defines the
`FusionContext` structure that RRF and WeightedRRF build on.

## Build (checklist of concrete, code-level steps)

- [x] `crates/calyx-sextant/src/fusion/mod.rs`:
  ```rust
  pub trait FusionImpl: Send + Sync {
      fn fuse(&self, ctx: &FusionContext) -> Result<Vec<Hit>, CalyxError>;
  }
  pub struct FusionContext<'a> {
      pub map: &'a SlotIndexMap,
      pub query_vecs: &'a HashMap<SlotId, Vec<f32>>,  // pre-embedded query per slot
      pub k: usize,
      pub ef: usize,
      pub explain: bool,
  }
  pub fn fuse(strategy: &FusionStrategy, ctx: &FusionContext) -> Result<Vec<Hit>, CalyxError>;
  ```
- [x] `crates/calyx-sextant/src/fusion/single.rs`: `SingleLensStrategy`:
      - calls `map.search(slot, query_vec, k, ef)`
      - converts each `(CxId, raw_score)` to a `Hit` with:
        - `fused_score = raw_score`
        - `per_lens = [PerLensEntry { slot, rank, raw_score, weight: 1.0, contribution: raw_score }]`
        - `cross_terms_used = []`
        - `guard = None`
        - `provenance = LedgerRef::stub(cx_id, 0)` (real in T05)
        - `freshness = FreshnessTag { built_at_seq: 0, stale_by: None }` (real in T05)
        - if `explain=false`, `per_lens` may be populated regardless — it is cheap
      - `CALYX_SEXTANT_SLOT_NOT_FOUND` if the requested slot is not in the map
- [x] Wire `FusionStrategy::SingleLens(slot)` in `fuse()` dispatcher to call
      `SingleLensStrategy::fuse()`
- [x] `CALYX_SEXTANT_NO_LENSES` error variant: returned when `query_vecs` is empty

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: insert 20 vecs into slot A, run SingleLens search k=5 → returns
      exactly 5 `Hit`s; each `hit.fused_score == hit.per_lens[0].raw_score`
- [x] unit: `hit.per_lens.len() == 1` for every hit from SingleLens strategy
- [x] unit: hits are ordered by `fused_score` descending
- [x] proptest: for any k ≤ n, SingleLens returns exactly k results
- [x] edge: slot not in map → `CALYX_SEXTANT_SLOT_NOT_FOUND`
- [x] edge: empty `query_vecs` → `CALYX_SEXTANT_NO_LENSES`
- [x] fail-closed: `k=0` → `CALYX_SEXTANT_EF_TOO_SMALL` propagated from `HnswGraph`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant single_lens -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant single_lens -- --nocapture 2>&1`
- **Prove:** test prints `strategy=single_lens k=5 hits=5 ordered=true`; this
  establishes the baseline recall number that T07 will compare multi-lens against

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH24 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
