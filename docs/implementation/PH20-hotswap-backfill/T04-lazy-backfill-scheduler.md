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

## Build (checklist of concrete, code-level steps)

- [ ] `BackfillPriority` enum: `Kernel`, `Hot`, `Normal`.
- [ ] `BackfillRequest` struct: `slot_id: SlotId`, `lens_id: LensId`,
  `priority: BackfillPriority`, `watermark: Option<CxId>` (last processed id,
  `None` = start from beginning).
- [ ] `BackfillConfig` struct: `max_concurrent: usize` (default 4),
  `batch_size: usize` (default 16), `throttle_ms: u64` (default 50 ms between
  batches).
- [ ] `BackfillScheduler` struct:
  - `queue: BinaryHeap<Reverse<(BackfillPriority, BackfillRequest)>>` (priority
    ordering: Kernel > Hot > Normal).
  - `active: HashSet<SlotId>` (slots currently being backfilled).
  - `config: BackfillConfig`.
- [ ] `BackfillScheduler::enqueue(req: BackfillRequest)`: push to heap; if
  `active.len() >= max_concurrent` → defer (heap holds it for next tick).
- [ ] `BackfillScheduler::tick(registry: &Registry, store: &dyn VaultStore) -> Result<BackfillStats>`:
  - pop up to `max_concurrent - active.len()` requests from heap.
  - for each request, fetch the next `batch_size` constellations from `store`
    starting after `watermark`.
  - for each constellation, call `registry.measure(lens_id, input)` → write
    result to slot CF (stub: log the write).
  - update `watermark` to last processed `CxId`.
  - if no more constellations → mark request complete; remove from `active`.
  - return `BackfillStats { slot_id, filled: usize, remaining: Option<usize> }`.
- [ ] Persisted watermark: serialize `HashMap<SlotId, CxId>` to
  `$CALYX_HOME/<vault_id>/backfill_watermark.json` after each batch.
- [ ] On scheduler init, reload watermarks from disk to resume interrupted
  backfill.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: enqueue 3 requests (Kernel, Normal, Hot); pop order is
  Kernel → Hot → Normal.
- [ ] unit: `tick` with 5 mock constellations and `batch_size=2` → fills 2
  per tick, `remaining=3` after first tick.
- [ ] unit: watermark persisted after first tick; on reinit, `BackfillScheduler`
  resumes from watermark rather than reprocessing already-filled rows.
- [ ] unit: `max_concurrent=1`; enqueue 3 requests; only 1 active after first
  tick.
- [ ] edge (≥3): (1) empty queue → `tick` is a no-op; (2) store returns 0
  constellations → request marked complete; (3) `measure` returns an error for
  one constellation → log the error, continue backfill (do not abort the whole
  run).
- [ ] fail-closed: measure error on one cx does not abort the batch; error is
  logged with the `CxId` and `CALYX_*` code.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `$CALYX_HOME/<vault>/backfill_watermark.json` on aiwonder filesystem
- **Readback:**
  `cargo test -p calyx-registry backfill -- --nocapture 2>&1` then
  `cat $CALYX_HOME/<vault>/backfill_watermark.json`
- **Prove:** watermark JSON shows the last processed `CxId` after a tick run;
  re-running scheduler from that watermark skips already-filled rows (assert
  `filled=0` on second run over same data); screenshot attached to PH20 GitHub
  issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH20 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
