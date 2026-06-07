# PH43 · T03 — Rollback store (prior artifact + pointer swap)

| Field | Value |
|---|---|
| **Phase** | PH43 — Tripwires + Shadow-First + Reversible/Rollback |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/rollback.rs` (≤500) |
| **Depends on** | T01 (TripwireRegistry identifies what to revert) |
| **Axioms** | A14, A15 |
| **PRD** | `dbprdplans/12 §6`, `dbprdplans/27 §4` |

## Goal

Implement `RollbackStore`: before any Anneal promotion, snapshot the prior
artifact (config entry, index pointer, or quant-level record) under a
`change_id`; after a promotion, the prior snapshot is retained until the change
is explicitly committed (confirmed good for N queries). Rollback is a single
atomic pointer swap — `rollback(change_id)` restores the prior artifact and the
live path is back to the previous state in O(1) without data movement.

## Build (checklist of concrete, code-level steps)

- [ ] `struct ChangeId(u64)` — monotonic, assigned from a vault-wide counter via the `Clock` + a seeded counter; never reused.
- [ ] `struct ArtifactSnapshot { change_id: ChangeId, prior_ptr: ArtifactPtr, candidate_ptr: ArtifactPtr, ts: LogicalTime, description: String }` where `ArtifactPtr` is an enum over config-cache key hash, HNSW graph path, quant-level record hash.
- [ ] `struct RollbackStore { snapshots: HashMap<ChangeId, ArtifactSnapshot>, live_ptrs: HashMap<ArtifactKey, ArtifactPtr> }` — stored in vault CF `anneal_rollback`.
- [ ] `fn prepare(key: ArtifactKey, candidate_ptr: ArtifactPtr) -> ChangeId` — reads current `live_ptr` for `key`, saves `ArtifactSnapshot`, returns `ChangeId`.
- [ ] `fn promote(change_id: ChangeId)` — atomically swaps `live_ptrs[key]` to `candidate_ptr`; snapshot retained.
- [ ] `fn rollback(change_id: ChangeId) -> Result<(), CalyxError>` — atomically swaps `live_ptrs[key]` back to `prior_ptr`; marks snapshot `reverted=true`; returns `CALYX_ANNEAL_UNKNOWN_CHANGE_ID` if not found.
- [ ] `fn commit(change_id: ChangeId)` — marks snapshot permanently committed; prior artifact may now be GC'd after a configurable retention window.
- [ ] All pointer swaps use `ArcSwap` or equivalent to be data-race-free; no `unsafe` blocks except in the swap primitive.
- [ ] Persist snapshot log to `anneal_rollback` CF via Aster WAL write; survives crash.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `prepare` → `promote` → `rollback` restores prior pointer exactly (compare `ArtifactPtr` byte equality).
- [ ] unit: double-rollback on a committed change → `CALYX_ANNEAL_CHANGE_COMMITTED`.
- [ ] proptest: sequence of `(prepare, promote, rollback)` operations never leaves `live_ptrs` in an inconsistent state (invariant: `live_ptrs[k]` is always either prior or candidate, never undefined).
- [ ] edge: `rollback` with unknown `change_id` → `CALYX_ANNEAL_UNKNOWN_CHANGE_ID`; concurrent `promote` + `rollback` on different keys → both succeed independently; empty store → `rollback` fails closed.
- [ ] fail-closed: WAL write failure during `prepare` → `CALYX_ASTER_WAL_SYNC` propagated; no partial snapshot recorded.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_rollback` CF row for the `ChangeId`; `live_ptrs` in-memory + persisted.
- **Readback:** `calyx readback anneal rollback --change-id <id>` (or `xxd` the `anneal_rollback` CF at the relevant key) — shows prior + candidate pointers + reverted flag.
- **Prove:** perform `prepare` → `promote` (live ptr = candidate) → `rollback` (live ptr = prior); `xxd` confirms the CF row has `reverted=true` and `live_ptrs` returns the prior pointer hash; byte-exact round-trip.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH43 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
