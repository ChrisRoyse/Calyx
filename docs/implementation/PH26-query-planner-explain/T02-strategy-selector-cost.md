# PH26 · T02 — Strategy selector + cost model

| Field | Value |
|---|---|
| **Phase** | PH26 — Query planner + intent + explain |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/planner.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH24 T04 (profiles) · PH23 T06 (`SlotIndexMap`) |
| **Axioms** | A17, A16 |
| **PRD** | `dbprdplans/10 §2`, `dbprdplans/10 §7`, `dbprdplans/17 §7.3` |

## Goal

Map `IntentLabel → FusionStrategy` using the 14 ContextGraph profiles as
defaults, then estimate query cost using a simple index-size model. The cost
estimate drives the cap enforcement in T03. The mapping is overridable per A17.

## Build (checklist of concrete, code-level steps)

- [ ] `fn intent_to_strategy(label: IntentLabel, map: &SlotIndexMap) -> FusionStrategy`:
      ```
      Code      → SingleLens(code_slot)   if code_slot registered, else General fallback
      Causal    → WeightedRRF("causal")
      Entity    → WeightedRRF("entity")
      Temporal  → WeightedRRF("temporal")
      Speaker   → SingleLens(speaker_slot) if registered, else WeightedRRF("speaker")
      Style     → SingleLens(style_slot)  if registered, else WeightedRRF("style")
      Civic     → WeightedRRF("civic")
      Media     → WeightedRRF("media")
      Bridge    → WeightedRRF("bridge")
      Kernel    → FusionStrategy::KernelFirst  (deferred to PH33; stub returns Rrf)
      Semantic  → Rrf
      Lexical   → WeightedRRF("lexical")
      Multimodal→ WeightedRRF("multimodal")
      General   → Rrf
      ```
      If a required slot is absent, fall back to the next best strategy and log
      a structured warning (not an error — the query still executes)
- [ ] `CostEstimate` struct:
  ```rust
  pub struct CostEstimate {
      pub num_slots: usize,
      pub index_size_hint: u64,   // total len() across participating slots
      pub ef_factor: f32,         // ef / 10.0 as a multiplier
      pub has_rerank: bool,
      pub estimated_ms: f32,      // rough: num_slots * 2.0 + ef_factor * 0.5 + if has_rerank { 20.0 } else { 0.0 }
  }
  ```
- [ ] `fn estimate_cost(strategy: &FusionStrategy, map: &SlotIndexMap, ef: usize, has_rerank: bool) -> CostEstimate`
- [ ] `PlannerOutput` struct: `{ strategy: FusionStrategy, intent: IntentLabel, cost: CostEstimate, override_used: bool }`
- [ ] `fn plan(query: &Query, map: &SlotIndexMap) -> Result<PlannerOutput, CalyxError>`:
      1. If `query.fusion` is explicit → `override_used=true`, skip classify
      2. Else: classify intent → select strategy → estimate cost
      3. Return `PlannerOutput`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `intent_to_strategy(Code, map_with_code_slot)` → `SingleLens(code_slot)`
- [ ] unit: `intent_to_strategy(Code, map_without_code_slot)` → `Rrf` (fallback)
- [ ] unit: `estimate_cost` for 2-slot RRF with ef=100, no rerank →
      `estimated_ms ≈ 4.0 + 5.0 = 9.0` (within 0.1)
- [ ] unit: `plan` with explicit `query.fusion = Rrf` → `override_used=true`,
      `strategy=Rrf` regardless of query text
- [ ] proptest: `estimate_cost` is non-negative for any valid inputs
- [ ] edge: `intent_to_strategy(Kernel, map)` → `Rrf` stub (KernelFirst not yet
      available; document "deferred to PH33")
- [ ] edge: `map.slots()` is empty → `CALYX_SEXTANT_NO_LENSES`
- [ ] fail-closed: `plan` with a query that has no text and no anchor and no
      explicit slots → `CALYX_SEXTANT_NO_LENSES`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant strategy_selector -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant strategy_selector -- --nocapture 2>&1`
- **Prove:** prints `code_strategy=single_lens fallback_ok=true cost_2slot=NNN
  override_used=true`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH26 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
