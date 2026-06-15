# PH20 - T05 - No-re-embed invariant + FSV integration test

| Field | Value |
|---|---|
| **Phase** | PH20 - Hot-swap add/retire/park + lazy backfill |
| **Stage** | S3 - Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/tests/hot_swap_fsv.rs` (<=500) |
| **Depends on** | T01, T02, T03, T04 (this phase) |
| **Axioms** | A5 |
| **PRD** | `dbprdplans/05`, `13_STAGE3_REGISTRY.md` PH20 FSV gate |

## Goal

Prove the PH20 FSV gate on aiwonder: after `add_lens_durable` on a populated
durable vault, zero existing constellations are rewritten; durable scheduler state
orders, throttles, resumes, and completes lazy backfill; and `retire_lens`
tombstones the slot while historical data remains readable.

## Build (implemented)

- [x] The FSV creates a durable Aster vault under `CALYX_FSV_ROOT`, writes two
  seeded constellations, flushes them, and snapshots base CF bytes before
  `add_lens`.
- [x] `add_lens_durable` allocates the new slot, bumps `panel_version`, leaves
  the placeholder index unready, persists the scheduler request, and proves old
  base rows are unchanged.
- [x] Durable backfill state is stored in `backfill-watermark.json` via
  `BackfillScheduler::open/enqueue/claim_next_batch/complete_batch`.
- [x] Backfill writes deterministic dense slot vectors for both synthetic
  `CxId`s, reads each slot vector immediately, and prints scheduler watermarks
  after enqueue, first complete, and restart-resume completion.
- [x] `retire_lens` tombstones the slot while historical constellation and slot
  rows stay readable.
- [x] Final FSV flushes and reopens the vault, then reads both backfilled slot CF
  rows from disk.

## Tests (synthetic, deterministic)

- [x] Happy path: add lens, write deferred placeholder, backfill two rows,
  retire, and reopen-read persisted slot vectors.
- [x] Edge: duplicate live lens is rejected without panel version or queue
  mutation.
- [x] Edge: zero-size queue claim returns no work and preserves queue length.
- [x] Edge: missing-constellation slot write fails closed and does not advance
  the Aster snapshot.
- [x] Edge: scheduler claim inside `throttle_ms` returns a throttled batch and
  restart-resume claims the next real candidate only after the throttle window.

## FSV (read the bytes on aiwonder - the truth gate)

- **SoT:** durable Aster vault plus
  `/home/croyse/calyx/data/fsv-issue311-durable-add-lens-20260608/backfill-watermark.json`
  on aiwonder.
- **Readback:**
  `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue311-durable-add-lens-20260608 cargo test -p calyx-registry ph20_hot_swap_aiwonder_fsv -- --ignored --nocapture`
  then `cat $CALYX_FSV_ROOT/backfill-watermark.json` and
  `find $CALYX_FSV_ROOT/vault -type f`.
- **Prove:** output shows old base digests unchanged, scheduler
  processed/pending/in-flight completion, edge-case before/after state, retired
  slot still readable, and reopened dense slot vectors matching the expected
  synthetic values.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <=500 lines
- [x] FSV evidence attached to GitHub issue #311
- [x] no anti-pattern: no flatten / no unbounded `C(N,2)` materialization /
  no "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
