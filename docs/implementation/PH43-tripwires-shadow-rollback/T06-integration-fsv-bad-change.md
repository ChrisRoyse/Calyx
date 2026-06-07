# PH43 · T06 — Integration: bad-change auto-revert FSV scenario

| Field | Value |
|---|---|
| **Phase** | PH43 — Tripwires + Shadow-First + Reversible/Rollback |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/integration_fsv.rs` (≤500) · `crates/calyx-anneal/tests/fsv_bad_change.rs` (≤500) |
| **Depends on** | T01, T02, T03, T05 |
| **Axioms** | A14, A15, A16 |
| **PRD** | `dbprdplans/12 §6`, `dbprdplans/27 §4` |

## Goal

Wire `TripwireRegistry` + `ShadowExecutor` + `RollbackStore` + `AnnealLedger`
into the `AnneaSubstrate` facade and prove the full safety loop end-to-end: a
deliberately-bad change (recall-degrading config) triggers the tripwire in
shadow, the revert fires before any promotion touches the live path, the Ledger
records the revert entry, and the live config pointer is byte-identical to the
prior artifact. This is the phase's FSV gate made into a deterministic test.

## Build (checklist of concrete, code-level steps)

- [ ] `struct AnnealSubstrate { tripwires: TripwireRegistry, shadow: ShadowExecutor, rollback: RollbackStore, ledger: AnnealLedger, budget: BudgetEnforcer }` — the facade every Anneal action goes through.
- [ ] `fn propose_change<A: AnnealAction>(&mut self, key: ArtifactKey, candidate: A, incumbent: A) -> ChangeOutcome` — the single entry point: `prepare` snapshot → `run_shadow` → if `Promote`: call `rollback.promote`, write Ledger `Promote` entry, return `ChangeOutcome::Promoted(change_id)`; if `Revert`: write Ledger `Revert` entry, do NOT call `rollback.promote`, return `ChangeOutcome::Reverted { reason, change_id }`.
- [ ] `enum ChangeOutcome { Promoted(ChangeId), Reverted { reason: ShadowRevertReason, change_id: ChangeId } }` — callers use this to decide next action.
- [ ] `fn rollback_explicit(&mut self, change_id: ChangeId) -> Result<(), CalyxError>` — operator-triggered rollback of a previously-promoted change; calls `rollback.rollback` + writes a Ledger `Revert` entry.
- [ ] `fn status(&self) -> AnnealStatus { tripwire_states, budget, recent_changes: Vec<AnnealLedgerEntry> }`.
- [ ] Integration test `fsv_bad_change`: create a synthetic vault; set `recall@k` tripwire to `0.90`; craft a `BadRecallAction` whose `apply_shadow` returns `recall=0.70`; call `propose_change`; assert `ChangeOutcome::Reverted`; assert Ledger contains `Revert` entry with `change_id`; assert `live_ptrs` still points to incumbent hash (read from RollbackStore).
- [ ] No `SystemTime::now()` anywhere — all clocks via injected `Arc<dyn Clock>`; all RNG seeded.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] integration: bad-recall candidate → `ChangeOutcome::Reverted`; Ledger has `Revert` entry; live ptr unchanged (exact hash comparison).
- [ ] integration: good candidate (beats incumbent on all metrics) → `ChangeOutcome::Promoted`; Ledger has `Promote` entry; live ptr updated to candidate hash.
- [ ] integration: `rollback_explicit` on a promoted change → live ptr back to prior hash; Ledger has second `Revert` entry with the same `change_id`.
- [ ] edge: propose a change while budget is exhausted → `ChangeOutcome::Reverted { reason: BudgetExhausted }`; no Ledger promotion entry written.
- [ ] fail-closed: Ledger write fails mid-`promote` → `CALYX_LEDGER_WRITE_FAIL`; `rollback.promote` is NOT called; live ptr unchanged.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Ledger CF `kind=Anneal` entries + `RollbackStore.live_ptrs` after the bad-change scenario.
- **Readback:** `calyx readback ledger --kind Anneal --last 3` (shows the `Revert` entry); `calyx anneal rollback-status --change-id <id>` (shows prior ptr hash).
- **Prove:** run `fsv_bad_change` scenario on aiwonder: before → no Anneal entries; inject bad change → `calyx readback ledger` shows 1 Anneal entry with `action=Revert`; `calyx anneal rollback-status` shows `reverted=true` and `live_ptr=<prior_hash>`; compare `prior_hash` against the pre-change `xxd` of the config slot → byte-identical.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH43 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
