# PH20 — Hot-swap add/retire/park + lazy backfill

**Stage:** S3 — Registry / Lenses  ·  **Crate:** `calyx-registry`  ·
**PRD roadmap:** P2  ·  **Axioms:** A5

## Objective

Implement the core ergonomic win: `add_lens`, `retire_lens`, `park_lens`,
and `unpark_lens` with **no global re-embed** and **no existing constellation
rewritten**. New lenses are searchable immediately for new constellations;
existing constellations are backfilled lazily in priority order (kernel/hot
first, then by query frequency, throttled, resumable). `retire_lens` is a
tombstone — history stays readable. Every operation bumps `panel_version`.

## Dependencies

- **Phases:** PH19 (all five runtimes working), PH09 (Aster slot CF +
  constellation CRUD — the backing store for the new slot columns)
- **Provides for:** PH21 (capability card profiling uses the registry's
  slot state), PH23 (Sextant searches per-slot ANN indexes), PH40 (temporal
  lens fusion depends on slot state flags)

## Current state (build off what exists)

`calyx-registry` has PH17–PH19: Registry, all runtimes, frozen contract.
`calyx-aster` has PH09: constellation CRUD, column families. Aster's
`slot_*/` CFs exist. `panel_version` field lives in `Constellation` struct.
Greenfield for `swap.rs` and backfill scheduler.

**aiwonder runtime endpoints:** `:8088` general GTE 768-d, `:8089` reranker,
`:8090` legal. `CALYX_HOME/.hf-cache`, `CALYX_HF_TOKEN` from env.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-registry/src/swap.rs` | `add_lens`, `retire_lens`, `park_lens`, `unpark_lens`, `panel_version` bump |
| `crates/calyx-registry/src/backfill.rs` | lazy backfill scheduler: priority queue (kernel/hot first), throttle, resumable state |
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
3. Run the backfill scheduler; observe slot columns filling over time
   (print before/after counts of `AbsentReason::Deferred` rows).
4. `retire_lens` → `SlotState::Retired`; historical constellations still
   readable from their slot columns; `panel_version` incremented.

Readback: `cargo test -p calyx-registry swap -- --include-ignored --nocapture`
on aiwonder; slot column counts before/after attached to PH20 GitHub issue.

## Risks / landmines

- **Backfill storm:** if the scheduler is not throttled, adding a lens to a
  large vault floods the TEI endpoint. Enforce a configurable `max_concurrent`
  (default 4) and `batch_size` (default 16) in `BackfillConfig`.
- **Resumable state:** backfill state must survive a process restart; persist
  the watermark (last `CxId` processed) to Aster or a simple JSON file.
- **panel_version monotonicity:** all four swap operations must bump the
  version; assert monotone increase in tests.
- **Retire ≠ delete:** columns for retired slots are kept until GC policy
  prunes them (PH58); fail loudly if any code path deletes slot CF data on
  retire.
