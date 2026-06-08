# PH20 · T04 — Lazy backfill scheduler (priority-ordered, throttled, resumable)

| Field | Value |
|---|---|
| **Phase** | PH20 — Hot-swap add/retire/park + lazy backfill |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/backfill.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A5 |
| **PRD** | `dbprdplans/05 §3`, `dbprdplans/17 §7.4` (backfill storm bounded) |

## Goal

Implement the lazy backfill scheduler that fills new slot columns for existing
constellations after `add_lens`. Priority: kernel constellations first, then
hot (high query-frequency), then the rest. Throttled to avoid VRAM/TEI
contention (`17 §7.4`). Resumable across process restarts via a persisted
watermark.

## Build (implemented)

- [x] `BackfillPriority` enum: `Normal`, `Hot`, `Kernel`; scheduler ranks
  `Kernel > Hot > Normal`.
- [x] `BackfillRequest` struct: `slot_id`, `lens_id`, `priority`, and an
  ordered candidate list. The persisted `next_index`, `in_flight`, and
  `last_processed` fields are the watermark state.
- [x] `BackfillConfig` struct: `max_concurrent` default 4, `batch_size` default
  16, `throttle_ms` default 50.
- [x] `BackfillScheduler::open(path, config)`: loads JSON state, swaps in the
  current config, and clears any persisted in-flight batch so interrupted work
  is retried after restart.
- [x] `BackfillScheduler::enqueue(req)`: persists the request without rewriting
  existing scheduler state for duplicate keys.
- [x] `BackfillScheduler::claim_next_batch(now_ms)`: returns the next
  non-throttled batch by priority, enforces `max_concurrent`, persists
  `in_flight`, and does not advance the watermark until completion.
- [x] `BackfillScheduler::complete_batch(slot_id, lens_id, now_ms)`: advances
  `next_index`, records `last_processed`, clears `in_flight`, marks complete
  when all candidates are filled, and persists `next_allowed_ms`.
- [x] `BackfillScheduler::watermarks()`: exposes processed/pending/in-flight,
  completion, and last processed `CxId` for readback.
- [x] `SwapController::add_lens_durable`: calls the hot-swap add path and
  persists the durable `BackfillRequest` in the same public API call; restores
  controller/scheduler objects before returning an enqueue error.

## Tests (synthetic, deterministic)

- [x] Unit: kernel request beats normal request, batch sizing is enforced,
  completion persists, and throttle blocks early claims.
- [x] Unit: a claimed but uncompleted batch is retried after reopening the
  scheduler file.
- [x] PH20 FSV: duplicate lens is rejected without panel/queue mutation.
- [x] PH20 FSV: zero-size queue claim is a no-op.
- [x] PH20 FSV: missing-constellation slot backfill fails closed without
  advancing the Aster snapshot.
- [x] PH20 FSV: scheduler JSON is read after enqueue, after first complete, and
  after restart-resume completion.
- [x] PH20 FSV: the durable Aster vault is reopened and both backfilled slot CF
  rows are read after final flush.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/home/croyse/calyx/data/fsv-issue311-durable-add-lens-20260608/backfill-watermark.json`
  plus the durable Aster vault under the same root.
- **Readback:** run
  `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue311-durable-add-lens-20260608 cargo test -p calyx-registry ph20_hot_swap_aiwonder_fsv -- --ignored --nocapture`,
  then read `backfill-watermark.json`, vault files, and the printed reopened
  slot-vector rows.
- **Prove:** scheduler JSON shows priority, processed/pending/in-flight,
  throttle-resume, and final completion; reopened Aster reads show the expected
  dense slot vectors for both synthetic `CxId`s; old base rows remain byte-stable.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence attached to GitHub issue #311
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI /
      nothing "trusted" without grounding / no frozen-lens mutation /
      no harness-as-FSV
