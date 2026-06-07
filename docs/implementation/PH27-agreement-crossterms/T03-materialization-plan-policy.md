# PH27 · T03 — `MaterializationPlan` + `plan_cross_terms` policy

| Field | Value |
|---|---|
| **Phase** | PH27 — Agreement graph + cross-terms (lazy) |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/materialization.rs` (≤500) |
| **Depends on** | T02 (lazy xterms) · PH28 (Assay pair_gain hook, wired after PH28) |
| **Axioms** | A8, A9 |
| **PRD** | `dbprdplans/06 §4` |

## Goal

Implement the per-pair, per-anchor materialization policy that decides which
cross-terms are stored eagerly in the xterm CF vs remain lazy (one matmul on
demand). The policy is: Agreement = always eager (scalar, cheap); Delta /
Interaction = eager only when `Assay.pair_gain(a,b|anchor) ≥ 0.05 bits`; Concat
= eager only when Sextant has promoted the pair (query-pattern justification).
This is the mechanism that keeps storage `O(n·n_eff)` not `O(n·N²)`.

## Build (checklist of concrete, code-level steps)

- [ ] Define `PairDecision` enum: `EagerStore`, `LazyCache`, `Skip` (for fully redundant pairs already captured by another materialized form)
- [ ] Define `MaterializationPlan`: `{ cx_id: CxId, pair_decisions: HashMap<(SlotId, SlotId), HashMap<CrossTermKind, PairDecision>> }`
- [ ] Implement `plan_cross_terms(cx_id, panel, assay_hook: &dyn AssayGate, sextant_hook: &dyn SextantPromoter, clock: &dyn Clock) -> MaterializationPlan`:
  - enumerate `active_pairs(panel)` — slot pairs where both states are `Active`
  - for each pair `(a,b)`: Agreement → always `EagerStore`
  - for each pair `(a,b)`: if `assay_hook.pair_gain(a,b,anchor) >= 0.05` → Interaction = `EagerStore`; else `LazyCache`
  - for each pair `(a,b)`: if `sextant_hook.promotes_concat(a,b)` → Concat = `EagerStore`; else `LazyCache`
  - Delta always `LazyCache` (directional contrast; materialized on demand only)
- [ ] Stub `AssayGate` trait (returns `0.0` bits until PH28 wires the real implementation); wire `AssayGate` into PH28
- [ ] Stub `SextantPromoter` trait (returns `false` until PH26 wires query-pattern data); note the hook point in code comments
- [ ] Expose `materialized_count(plan) -> usize` — count of `EagerStore` decisions; used by `abundance_report` to prove storage is not `C(N,2)`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: with stub `AssayGate` (0.0 bits always) and stub `SextantPromoter` (false always), only Agreement decisions are `EagerStore`; all Delta/Interaction/Concat are `LazyCache`; `materialized_count == n_active_pairs` (one Agreement per pair)
- [ ] unit: with a mock `AssayGate` returning `0.06 bits` for pair `(a,b)` and `0.0` for all others, only `(a,b)` Interaction is `EagerStore`
- [ ] proptest: `materialized_count(plan) <= active_pairs_count(panel)` always (never exceeds the number of active pairs, never reaches `C(N,2)` Agreement + Interaction unless all pairs pass the gate)
- [ ] edge: empty panel → `MaterializationPlan` with empty decisions; single-slot panel → zero active pairs; panel with all inactive slots → zero active pairs
- [ ] fail-closed: `plan_cross_terms` with a `CxId` that has no slot data → `CALYX_ASTER_NOT_FOUND`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `materialized_count` in the plan for a planted panel (N=13 lenses, stub assay gate = all zeros bits)
- **Readback:** run `cargo test materialization_plan_agreement_only -- --nocapture`; print plan summary showing `materialized_count = 78` (one Agreement scalar per pair, no Interaction), confirming storage is `78n` not `78n + more`
- **Prove:** the plan log must not contain any `EagerStore` for `Interaction` or `Concat` when the stub gate returns 0.0 bits; confirm by running the test on aiwonder and capturing output.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH27 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
