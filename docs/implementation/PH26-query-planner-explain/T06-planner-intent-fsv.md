# PH26 · T06 — Planner intent FSV: per-case + unbounded rejection

| Field | Value |
|---|---|
| **Phase** | PH26 — Query planner + intent + explain |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/tests/planner_intent.rs` (≤500) |
| **Depends on** | T05 (this phase) |
| **Axioms** | A17, A15, A16 |
| **PRD** | `dbprdplans/10 §2`, `dbprdplans/10 §7`, `dbprdplans/17 §7.3` |

## Goal

The PH26 exit gate: verify on aiwonder that each intent auto-selects the correct
strategy, `explain=true` returns the full per-lens + provenance breakdown, and
an unbounded plan is rejected with `CALYX_SEXTANT_PLAN_UNBOUNDED`. This closes
Stage 4 — Sextant. With Stage 0–4 + a migration shadow (PH64), Calyx answers a
real vault with multiple lenses, provenance, lexical search, and a smart planner:
the demo that justifies the project.

## Build (checklist of concrete, code-level steps)

- [x] `tests/planner_intent.rs` — always-runs test suite (no external dataset):
      Build a `SlotIndexMap` with:
      - one dense HNSW slot (128-dim, 100 random vecs, seed=42)
      - one sparse InvertedIndex slot (same 100 constellations, text from the
        PH25 T06 corpus)
      - register dense as `SlotKind::Dense`, sparse as `SlotKind::Sparse`
      Test each intent case:
      ```rust
      let cases = [
          ("def foo(x: int):",      IntentLabel::Code,     "single_lens"),
          ("why did Rome fall",     IntentLabel::Causal,   "weighted_rrf:causal"),
          ("who founded Apple",     IntentLabel::Entity,   "weighted_rrf:entity"),
          ("events in 1789",        IntentLabel::Temporal, "weighted_rrf:temporal"),
          ("semantic similarity",   IntentLabel::Semantic, "rrf"),
          ("summarize this",        IntentLabel::General,  "rrf"),
      ];
      ```
      For each: call `plan()`, assert `intent` matches, assert `strategy_chosen`
      starts with the expected prefix
- [x] Unbounded plan test:
      - Create `query` with `k = usize::MAX` → assert `plan()` returns
        `Err(CalyxError::CALYX_SEXTANT_PLAN_UNBOUNDED)`
      - Create `query` with `ef` via `Query.rerank.top_k_candidates = usize::MAX`
        equivalent → same error
- [x] Explain test:
      - Run `planned_explain_search` with `explain=true` on a "causal" query →
        assert: `hits[0].inner.hit.per_lens.len() >= 1`,
        `hits[0].intent == IntentLabel::Causal`,
        `hits[0].inner.hit.provenance != LedgerRef::zero()`
- [x] Print summary line at end:
      ```
      intent=code strategy=single_lens ok=true
      intent=causal strategy=weighted_rrf:causal ok=true
      intent=entity strategy=weighted_rrf:entity ok=true
      intent=temporal strategy=weighted_rrf:temporal ok=true
      intent=general strategy=rrf ok=true
      unbounded_plan rejected=CALYX_SEXTANT_PLAN_UNBOUNDED ok=true
      explain_breakdown non_empty=true intent_label_present=true provenance_ok=true
      ```

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] integration (always runs): all 6 intent cases pass strategy check
- [x] integration: unbounded plan rejected (2 variants)
- [x] integration: explain breakdown has correct fields for causal query
- [x] unit: `LedgerRef::zero()` is a defined constant and differs from any stub ref
- [x] edge: `plan()` on a query with `lenses = Explicit([])` (empty slot list) →
      `CALYX_SEXTANT_NO_LENSES`
- [x] fail-closed: any case where `plan()` returns `Err` causes the entire search
      to abort — assert no partial hits are returned

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stdout of `cargo test -p calyx-sextant planner_intent -- --nocapture`
  on aiwonder
- **Readback:** `cargo test -p calyx-sextant planner_intent -- --nocapture 2>&1 | grep -E 'intent=|unbounded|explain'`
- **Prove:** all 7 summary lines printed (one per case + unbounded + explain);
  screenshot of this output attached to the PH26 GitHub issue as final FSV
  evidence; PH26 is DONE when this screenshot is on the issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH26 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
