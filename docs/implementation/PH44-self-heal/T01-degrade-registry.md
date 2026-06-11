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

- [x] `enum ComponentHealth { Ok, Degraded { since: LogicalTime, reason: String }, Failing { since: LogicalTime, reason: String }, Parked { since: LogicalTime, reason: String } }`.
- [x] `enum ComponentKind { AnnIndex { slot_id: SlotId }, KernelIndex { scope: ScopeId }, GuardProfile { slot_id: SlotId }, LensEndpoint { lens_id: LensId } }`.
- [x] `struct DegradeRegistry { components: HashMap<ComponentKind, ComponentHealth>, clock: Arc<dyn Clock> }`.
- [x] `fn set_health(&mut self, kind: ComponentKind, health: ComponentHealth)` — updates state and writes an `AnnealLedger` entry (`action=DegradeChange`).
- [x] `fn health(&self, kind: &ComponentKind) -> &ComponentHealth` — fast read path; no locking on the hot serving path (use `Arc<RwLock<_>>` with short-lived read locks).
- [x] `fn active_lenses(&self, all: &[LensId]) -> Vec<LensId>` — returns lenses not in `Failing` or `Parked`; used by search fusion to route queries.
- [x] `fn degraded_components(&self) -> Vec<(ComponentKind, ComponentHealth)>` — used by `calyx anneal status`.
- [x] Persist registry snapshot to `anneal_health` CF in Aster; reload on restart.
- [x] Never transitions from `Degraded` → `Ok` without an explicit heal confirmation (from T03 rebuild complete); prevents premature health clearing.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: set lens `L1` to `Failing`; `active_lenses([L1, L2])` returns `[L2]`.
- [x] unit: set ANN index to `Degraded`; `health(AnnIndex{slot})` returns `Degraded`; set to `Ok` after rebuild confirmation → `health` returns `Ok`.
- [x] proptest: for any sequence of `set_health` calls, `active_lenses` never returns a `Failing` or `Parked` lens.
- [x] edge: all lenses in `Failing` → `active_lenses` returns empty vec (not a panic); single component registry → `degraded_components` returns it; component not registered → `health` returns `Ok` (unknown = assumed ok, not assumed broken).
- [x] fail-closed: CF persist failure during `set_health` → `CALYX_ASTER_CF_UNAVAILABLE`; in-memory state still updated (serve can continue); error surfaced to caller.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_health` CF + `degraded_components()` return value.
- **Readback:** `calyx anneal status --health` — prints all components with health state and `since` timestamps.
- **Prove:** set ANN index `slot_0` to `Degraded`; call `status --health`; confirm output contains `AnnIndex(slot_0): Degraded`; confirm `active_lenses` does not include any lens in `Failing`.

## Implementation evidence

- Code: `crates/calyx-anneal/src/heal/degrade.rs`,
  `crates/calyx-cli/src/anneal_status.rs`, `anneal_health` CF in Aster.
- Tests: `crates/calyx-anneal/tests/degrade.rs` covers routing, heal
  confirmation, reload, property sequences, and fail-closed persistence.
- aiwonder FSV root:
  `/home/croyse/calyx/data/fsv-issue400-degrade-20260611T081455Z`.
  Readbacks include `ph44-degrade-health-readback.json`,
  `calyx-anneal-status-health.txt`, `anneal-health-cf-readback.txt`,
  `anneal-health-wal-readback.txt`, and `anneal-health-sst-head.txt`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines (line-count gate)
- [x] FSV evidence captured for the PH44 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
