# PH44 · T03 — Background rebuild (ANN + kernel + guard from base+slots)

| Field | Value |
|---|---|
| **Phase** | PH44 — Self-Heal (Rebuild Derived, Degrade Flags) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/heal/rebuild.rs` (≤500) |
| **Depends on** | T01 (DegradeRegistry), T02 (FaultDetectors trigger rebuild) |
| **Axioms** | A16, A15 |
| **PRD** | `dbprdplans/12 §2`, `dbprdplans/24 §7` rows 12, 16 |

## Goal

Implement `RebuildScheduler`: when a fault detector marks a derived structure
`Degraded`, the scheduler queues a background rebuild sourced exclusively from
base+slots (the WAL-proven ground truth, never from the corrupt derived
artifact). Each rebuild runs within the background budget (PH43 T04), uses the
current MVCC snapshot from Aster (PH08) so concurrent writes are safe, and
transitions the component back to `Ok` only after the rebuilt artifact passes a
tripwire check. The prior artifact is kept until the rebuild is proven (rollback
if tripwires cross).

## Build (checklist of concrete, code-level steps)

- [ ] `enum RebuildTarget { AnnIndex { slot_id: SlotId }, KernelIndex { scope: ScopeId }, GuardProfile { slot_id: SlotId } }` — only derived targets; base shards are handled by T04.
- [ ] `trait Rebuilder: Send + Sync { fn rebuild(&self, target: &RebuildTarget, snapshot: MvccSnapshot, budget: BudgetHandle) -> Result<ArtifactPtr, CalyxError>; }` — one impl per target type: `AnnIndexRebuilder` (reads slot vectors from base+slots CF), `KernelIndexRebuilder` (reads base+lodestar CF), `GuardProfileRebuilder` (reads anchors CF).
- [ ] `struct RebuildScheduler { queue: PriorityQueue<RebuildJob>, rebuilders: HashMap<DiscriminantOf<RebuildTarget>, Box<dyn Rebuilder>>, substrate: Arc<AnnealSubstrate>, registry: Arc<DegradeRegistry> }`.
- [ ] `fn enqueue(&mut self, target: RebuildTarget, priority: RebuildPriority)` — inserts into priority queue; higher priority for actively-queried slots.
- [ ] `fn run_next(&mut self) -> Result<RebuildOutcome, CalyxError>` — pops highest-priority job, acquires budget, calls appropriate `Rebuilder`, runs through `AnnealSubstrate::propose_change` (shadow+tripwire check), updates `DegradeRegistry` to `Ok` on success.
- [ ] `RebuildOutcome::Completed { change_id, prior_ptr, new_ptr }` logged to Ledger; `RebuildOutcome::Failed { reason }` leaves component `Degraded`.
- [ ] Guarantee: the `Rebuilder` MUST read only from `base` and `slots` CFs on the given `MvccSnapshot`; accessing a derived CF during a rebuild → `CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION`.
- [ ] Clock-injected; no `SystemTime::now()`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: enqueue an ANN rebuild, run it; `DegradeRegistry` transitions to `Ok`; Ledger has `Rebuild` entry; `ArtifactPtr` points to new artifact (different hash from corrupt one).
- [ ] unit: `AnnIndexRebuilder` accesses derived CF → `CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION`; rebuild fails, registry stays `Degraded`.
- [ ] proptest: any sequence of `enqueue` + `run_next` leaves `DegradeRegistry` consistent (component is `Ok` iff a successful rebuild completed).
- [ ] edge: rebuild target not in `Degraded` state → `run_next` skips it; empty queue → `run_next` returns `RebuildOutcome::NothingQueued`; budget exhausted → rebuild yields, re-queues.
- [ ] fail-closed: MVCC snapshot read fails during rebuild → `CALYX_ASTER_SNAPSHOT_UNAVAILABLE`; component stays `Degraded`; Ledger records failure.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** base+slots CF (byte-intact), Ledger `Rebuild` entry, `DegradeRegistry` state.
- **Readback:** `calyx readback ledger --kind Anneal --action Rebuild --last 1`; `calyx anneal status --health`.
- **Prove:** flip a byte in the HNSW index file → `Degraded` detected → `enqueue` → `run_next` → Ledger shows `action=Rebuild, outcome=Completed`; `status --health` shows `AnnIndex(slot_0): Ok`; `xxd` the base CF confirms base bytes unchanged (no data loss); rebuilt artifact checksum stored in `ChecksumDetector` matches the new file.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH44 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
