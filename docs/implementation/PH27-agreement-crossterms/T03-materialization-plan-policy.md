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
demand). The policy is: Agreement = always eager (scalar, cheap); Delta =
always lazy; Interaction = eager only when `Assay.pair_gain(a,b|anchor) ≥ 0.05
bits`; Concat = lazy/on-demand until a later Sextant promoter wires real query-
pattern justification.
This is the mechanism that keeps storage `O(n·n_eff)` not `O(n·N²)`.

## Implemented state

Post-sweep #319 wires the PH28 live adapter:
`calyx_assay::AsterAssayMaterializationGate` reads AsterVault slot vectors and
grounded binary anchors, computes `AssayGate::pair_gain`, implements Loom's
`PairGainGate`, and returns `0.0` with `last_error()` recorded when slot or
anchor state is missing. `LoomStore::materialize_plan` now writes every eager
plan entry into the xterm CF, so FSV reads kind counts from physical rows
instead of trusting planner return values.

This implemented state supersedes the original stub-oriented checklist below:
Stage 5 no longer depends on a dummy Assay gate for real materialization
decisions. The remaining Sextant promotion hook is intentionally deferred;
`Concat` stays lazy until query-pattern evidence is introduced.

## Build (checklist of concrete, code-level steps)

- [ ] Define `PairDecision` enum: `EagerStore`, `LazyCache`, `Skip` (for fully redundant pairs already captured by another materialized form)
- [ ] Define `MaterializationPlan`: `{ cx_id: CxId, pair_decisions: HashMap<(SlotId, SlotId), HashMap<CrossTermKind, PairDecision>> }`
- [ ] Implement `plan_cross_terms(cx_id, panel, assay_hook: &dyn AssayGate, sextant_hook: &dyn SextantPromoter, clock: &dyn Clock) -> MaterializationPlan`:
  - enumerate `active_pairs(panel)` — slot pairs where both states are `Active`
  - for each pair `(a,b)`: Agreement → always `EagerStore`
  - for each pair `(a,b)`: if `assay_hook.pair_gain(a,b,anchor) >= 0.05` → Interaction = `EagerStore`; else `LazyCache`
  - for each pair `(a,b)`: Concat = `LazyCache` until the later Sextant promotion hook is wired
  - Delta always `LazyCache` (directional contrast; materialized on demand only)
- [x] Replace the original stub Assay path with the PH28/#319
  `AsterAssayMaterializationGate` live adapter.
- [x] Keep Sextant promotion deferred by policy; `Concat` remains lazy until a
  later query-pattern promoter exists.
- [ ] Expose `materialized_count(plan) -> usize` — count of `EagerStore` decisions; used by `abundance_report` to prove storage is not `C(N,2)`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: with a static zero-gain gate, only Agreement decisions are
  `EagerStore`; all Delta/Interaction/Concat are lazy.
- [x] unit: with a positive-gain gate, qualifying Interaction rows are
  `EagerStore`.
- [ ] proptest: `materialized_count(plan) <= 2 * active_pairs_count(panel)` always (Agreement plus qualifying Interaction; Delta/Concat do not inflate eager storage)
- [ ] edge: empty panel → `MaterializationPlan` with empty decisions; single-slot panel → zero active pairs; panel with all inactive slots → zero active pairs
- [ ] fail-closed: `plan_cross_terms` with a `CxId` that has no slot data → `CALYX_ASTER_NOT_FOUND`

## FSV (read the bytes on aiwonder — the truth gate)

> **Post-sweep #319 superseding readback:** Run:
> ```
> CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue319-aster-materialization-gate-20260608 \
>   cargo test -p calyx-assay aster_materialization_gate_aiwonder_fsv -- --ignored --nocapture --test-threads=1
> ```
> Then read `aster-materialization-gate-readback.json` plus the live and
> missing-anchor xterm CF SST files. The live path must show Agreement and
> Interaction xterm rows; missing anchor/slot paths must record fail-closed
> errors and no eager Interaction materialization.

- **SoT:** `materialized_count` in the plan for a planted panel (N=13 lenses, stub assay gate = all zeros bits)
- **Readback:** run `cargo test materialization_plan_agreement_only -- --nocapture`; print plan summary showing `materialized_count = 78` (one Agreement scalar per pair, no Interaction), confirming storage is `78n` not `78n + more`
- **Prove:** the plan log must not contain any `EagerStore` for `Interaction` or `Concat` when the stub gate returns 0.0 bits; when the gate returns 0.06 bits for every pair, Agreement + Interaction are eager but Delta/Concat remain lazy.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence attached via #319:
  `/home/croyse/calyx/data/fsv-issue319-aster-materialization-gate-20260608`
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
