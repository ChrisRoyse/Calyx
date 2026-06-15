# PH20 — Hot-swap add/retire/park + lazy backfill

**Stage:** S3 — Registry / Lenses  ·  **Crate:** `calyx-registry`  ·
**PRD roadmap:** P2  ·  **Axioms:** A5

## Objective

Implement the core ergonomic win: `add_lens`, `retire_lens`, `park_lens`,
and `unpark_lens` with **no global re-embed** and **no existing constellation
rewritten**. Successful state-changing operations bump `panel_version`; exact
repeat adds and repeated lifecycle calls are idempotent no-ops. New lenses are
searchable immediately for new constellations; existing constellations are
backfilled lazily in priority order (kernel/hot first, then by query frequency,
throttled, resumable). `retire_lens` is a tombstone — history stays readable.

## Dependencies

- **Phases:** PH19 (all five runtimes working), PH09 (Aster slot CF +
  constellation CRUD — the backing store for the new slot columns)
- **Provides for:** PH21 (capability card profiling uses the registry's
  slot state), PH23 (Sextant searches per-slot ANN indexes), PH40 (temporal
  lens fusion depends on slot state flags)

## Current state

`calyx-registry` has PH17-PH19 plus PH20: Registry, all runtimes, frozen
contract, `swap.rs` hot-swap mechanics, and `backfill.rs` durable scheduler
state. `calyx-aster` has PH09 constellation CRUD and slot column families, so
PH20 FSV writes real slot vectors, flushes the vault, reopens it, and reads the
slot CF rows back from disk. The old queue-only scheduler proof is superseded by
#300; #311 wires durable scheduler enqueue into the public hot-swap API with
`SwapController::add_lens_durable`. #314 requires every hot-swap add path to
receive a `Registry` and prove the lens id is registered with a frozen contract
matching the requested slot shape/modality before panel, queue, or scheduler
state can mutate. #315 persists scheduler JSON through temp-file/fsync/rename
and fails closed on corrupt persisted scheduler state. #321 makes scheduler
mutations transactional, so a persist failure after the scheduler rename restores
the prior scheduler JSON before durable hot-swap rollback returns an error.
#327 makes PH20 lifecycle semantics explicit: identical live `add_lens` calls
return the existing slot without panel-version or backfill mutation; repeated
park/unpark/retire calls are no-ops; park/retire cancels pending backfill for the
slot; and Sextant excludes parked/retired slots from default search while
returning `CALYX_SEXTANT_SLOT_INACTIVE` for explicit inactive-slot searches.

**aiwonder runtime endpoints:** `:8088` general GTE 768-d, `:8089` reranker,
`:8090` legal. `CALYX_HOME/.hf-cache`, `CALYX_HF_TOKEN` from env.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-registry/src/swap.rs` | `add_lens` in-memory queue path, `add_lens_durable` production scheduler path, `retire_lens`, `park_lens`, `unpark_lens`, `panel_version` bump |
| `crates/calyx-registry/src/backfill.rs` | lazy backfill scheduler: kernel/hot/normal priority, persisted JSON watermarks, throttle, restart resume |
| `crates/calyx-registry/src/slot_alloc.rs` | `SlotId` allocation, slot CF column creation stub (wires to Aster in PH23) |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | add_lens: slot allocation + panel_version bump | PH19 |
| T02 | retire_lens: tombstone + keep history | T01 |
| T03 | park_lens / unpark_lens | T01 |
| T04 | Lazy backfill scheduler (priority-ordered, throttled, resumable) | T01 |
| T05 | No-re-embed invariant + FSV integration test | T02, T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. Add a new `TeiHttpLens` to a populated vault (or mock store with N
   pre-existing constellations); assert zero existing constellation is
   rewritten (`slot_*/` CF rows for old constellations are unchanged).
2. The new slot is searchable immediately for a freshly ingested constellation.
3. Call `add_lens_durable`; read `backfill-watermark.json` immediately after
   add to prove durable scheduler enqueue happened inside the hot-swap API
   call.
4. Run the durable backfill scheduler; read `backfill-watermark.json` after
   enqueue, after first batch, and after restart-resume to prove priority order,
   throttle, and completion state.
5. `retire_lens` → `SlotState::Retired`; historical constellations still
   readable from their slot columns; `panel_version` increments only on the
   first state-changing retire. Reopen the durable Aster vault and read the
   backfilled slot CF rows again.
6. Attempt `add_lens_durable` with an unregistered `LensId`; read panel version,
   slot count, queue length, and scheduler JSON bytes before/after to prove no
   mutation and `CALYX_LENS_FROZEN_VIOLATION`.
7. Persist a backfill scheduler request, reopen it, then try to open a corrupt
   scheduler JSON file; read the good/corrupt file bytes and error artifact to
   prove atomic-state readback and fail-closed corruption handling.
8. Inject a post-rename scheduler persist failure during `add_lens_durable`;
   read panel version, slot count, queue length, scheduler JSON bytes, reopened
   watermarks, marker file state, and temp files to prove disk and memory rollback
   stay consistent.

Readback: `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue311-durable-add-lens-20260608 cargo test -p calyx-registry ph20_hot_swap_aiwonder_fsv -- --ignored --nocapture`
on aiwonder, followed by `cat $CALYX_FSV_ROOT/backfill-watermark.json` and vault
file readback. Evidence is attached to GitHub issue #311.

#314 readback: `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue314-registered-hot-swap-20260608 cargo test -p calyx-registry ph20_unregistered_hot_swap_fails_closed_aiwonder_fsv -- --ignored --nocapture`
on aiwonder, followed by `cat $CALYX_FSV_ROOT/hot-swap-registered-readback.json`.

#315 readback: `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue315-backfill-atomic-persist-20260608 cargo test -p calyx-registry ph20_backfill_atomic_persist_aiwonder_fsv -- --ignored --nocapture`
on aiwonder, followed by `cat $CALYX_FSV_ROOT/backfill-atomic-readback.json`.

#321 readback: `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue321-durable-rollback-20260608 cargo test -p calyx-registry ph20_backfill_atomic_persist_aiwonder_fsv -- --ignored --nocapture`
on aiwonder, followed by `cat $CALYX_FSV_ROOT/backfill-atomic-readback.json`.

## Risks / landmines

- **Backfill storm:** if the scheduler is not throttled, adding a lens to a
  large vault floods the TEI endpoint. Enforce a configurable `max_concurrent`
  (default 4) and `batch_size` (default 16) in `BackfillConfig`.
- **Resumable state:** backfill state must survive a process restart; persist
  the watermark (last `CxId` processed) to Aster or an atomic JSON file. Corrupt
  scheduler JSON must fail closed, not fabricate an empty queue. Persist errors
  during mutation must restore previous scheduler bytes before caller rollback
  proceeds.
- **panel_version monotonicity:** all four swap operations must bump the version
  only when they change state; idempotent repeats must leave the version
  unchanged. Assert monotone/no-op behavior in tests.
- **Retire ≠ delete:** columns for retired slots are kept until GC policy
  prunes them (PH58); fail loudly if any code path deletes slot CF data on
  retire.
