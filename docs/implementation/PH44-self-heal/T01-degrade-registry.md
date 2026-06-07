# PH44 · T01 — Degrade registry + health flags

| Field | Value |
|---|---|
| **Phase** | PH44 — Self-Heal (Rebuild Derived, Degrade Flags) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/heal/degrade.rs` (≤500) |
| **Depends on** | — (first card; used by all other T* in this phase) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/12 §2`, `dbprdplans/24 §7` |

## Goal

Define `DegradeRegistry`: tracks the health state of every healable component
(ANN index, kernel index, guard profile, each lens endpoint) as one of `Ok /
Degraded / Failing / Parked`. Components in `Degraded` state are served with a
`degraded: true` flag in results; `Failing` lens endpoints are excluded from
routing (remaining lenses serve); `Parked` lenses are silently excluded. The
registry is the single source of truth for serving-path health decisions.

## Build (checklist of concrete, code-level steps)

- [ ] `enum ComponentHealth { Ok, Degraded { since: LogicalTime, reason: String }, Failing { since: LogicalTime, reason: String }, Parked { since: LogicalTime, reason: String } }`.
- [ ] `enum ComponentKind { AnnIndex { slot_id: SlotId }, KernelIndex { scope: ScopeId }, GuardProfile { slot_id: SlotId }, LensEndpoint { lens_id: LensId } }`.
- [ ] `struct DegradeRegistry { components: HashMap<ComponentKind, ComponentHealth>, clock: Arc<dyn Clock> }`.
- [ ] `fn set_health(&mut self, kind: ComponentKind, health: ComponentHealth)` — updates state and writes an `AnnealLedger` entry (`action=DegradeChange`).
- [ ] `fn health(&self, kind: &ComponentKind) -> &ComponentHealth` — fast read path; no locking on the hot serving path (use `Arc<RwLock<_>>` with short-lived read locks).
- [ ] `fn active_lenses(&self, all: &[LensId]) -> Vec<LensId>` — returns lenses not in `Failing` or `Parked`; used by search fusion to route queries.
- [ ] `fn degraded_components(&self) -> Vec<(ComponentKind, ComponentHealth)>` — used by `calyx anneal status`.
- [ ] Persist registry snapshot to `anneal_health` CF in Aster; reload on restart.
- [ ] Never transitions from `Degraded` → `Ok` without an explicit heal confirmation (from T03 rebuild complete); prevents premature health clearing.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: set lens `L1` to `Failing`; `active_lenses([L1, L2])` returns `[L2]`.
- [ ] unit: set ANN index to `Degraded`; `health(AnnIndex{slot})` returns `Degraded`; set to `Ok` after rebuild confirmation → `health` returns `Ok`.
- [ ] proptest: for any sequence of `set_health` calls, `active_lenses` never returns a `Failing` or `Parked` lens.
- [ ] edge: all lenses in `Failing` → `active_lenses` returns empty vec (not a panic); single component registry → `degraded_components` returns it; component not registered → `health` returns `Ok` (unknown = assumed ok, not assumed broken).
- [ ] fail-closed: CF persist failure during `set_health` → `CALYX_ASTER_CF_UNAVAILABLE`; in-memory state still updated (serve can continue); error surfaced to caller.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_health` CF + `degraded_components()` return value.
- **Readback:** `calyx anneal status --health` — prints all components with health state and `since` timestamps.
- **Prove:** set ANN index `slot_0` to `Degraded`; call `status --health`; confirm output contains `AnnIndex(slot_0): Degraded`; confirm `active_lenses` does not include any lens in `Failing`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH44 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
