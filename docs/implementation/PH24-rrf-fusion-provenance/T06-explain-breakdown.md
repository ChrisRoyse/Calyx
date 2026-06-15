# PH24 · T06 — `explain=true` per-lens breakdown

| Field | Value |
|---|---|
| **Phase** | PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/search.rs` (≤500), `crates/calyx-sextant/src/explain.rs` (≤500) |
| **Depends on** | T05 (this phase) |
| **Axioms** | A15, A17 |
| **PRD** | `dbprdplans/10 §1`, `dbprdplans/10 §5`, `dbprdplans/10 §8` |

## Goal

When `Query.explain == true`, each `Hit` carries a queryable per-lens + provenance
breakdown that an agent can read to understand which lens found the hit and how
grounded it is. The overhead must be ≤ 3 ms over non-explain (`10 §8`). This
card makes that path wire-complete and verified.

## Build (checklist of concrete, code-level steps)

- [x] `crates/calyx-sextant/src/explain.rs`:
  ```rust
  pub struct ExplainHit {
      pub hit: Hit,
      pub fusion_strategy: String,   // e.g. "rrf" / "weighted_rrf:general" / "single:slot_0"
      pub slots_searched: Vec<SlotId>,
      pub query_vec_norms: Vec<(SlotId, f32)>,  // L2 norm of embedded query per slot
      pub total_candidates_before_topk: usize,
  }
  pub fn explain_search(
      query: &Query,
      map: &SlotIndexMap,
      embedder: &dyn EmbedQuery,
      ledger: &dyn LedgerProvider,
      clock: &dyn Clock,
  ) -> Result<Vec<ExplainHit>, CalyxError>;
  ```
- [x] `ExplainHit` wraps `Hit` — the `per_lens` field is always fully populated
      when explain=true (already true from prior cards; assert it here with a check)
- [x] When `explain=false`, `search()` calls the fast path (T05); when `explain=true`,
      calls `explain_search()` which adds the metadata fields above
- [x] The overhead test: run 100 queries with explain=false, record p50 latency;
      run same 100 queries with explain=true, assert `explain_p50 ≤ base_p50 + 3000 µs`
- [x] `ExplainHit` derives `serde::Serialize` for JSON output via CLI (PH62)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `explain=true` → every `ExplainHit.hit.per_lens` is non-empty
- [x] unit: `explain=true` → `fusion_strategy` string is non-empty and matches the
      strategy variant (e.g. starts with "rrf" for `FusionStrategy::Rrf`)
- [x] unit: `explain=true` and `explain=false` return the same `Hit.cx_id` list
      (the breakdown is additive, not order-changing)
- [x] unit: `slots_searched` contains exactly the slots that contributed to fusion
- [x] proptest: `ExplainHit` serializes to valid JSON for any valid `Hit`
- [x] edge: `explain=true` on empty result → returns `Ok(vec![])` with no panic
- [x] edge: overhead test — measured on aiwonder: explain latency delta ≤ 3 ms
      (this is an integration-style test; mark `#[ignore]` for CI, run for FSV)
- [x] fail-closed: `ExplainHit` with a zero `LedgerRef` → test asserts this is
      unreachable (provenance is always set, from T05)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant explain -- --nocapture --ignored`
- **Readback:** `cargo test -p calyx-sextant explain -- --nocapture --ignored 2>&1 | grep -E 'explain_delta|strategy'`
- **Prove:** prints `explain_delta_us=NNN` where NNN ≤ 3000; prints at least one
  `strategy=rrf per_lens_count=N provenance=XXXX` line with non-zero provenance hex

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH24 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
