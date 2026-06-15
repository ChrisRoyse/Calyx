# PH38 - T04 - `DriftMonitor` + Anneal hook + `guard_health()`

| Field | Value |
|---|---|
| **Phase** | PH38 - tau Calibration (Conformal) + Novelty -> New Region |
| **Stage** | S8 - Ward Gtau Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/drift.rs` (<=500) |
| **Depends on** | T03 (this phase), PH48 (Anneal - stub hook until live) |
| **Axioms** | A12, A14 |
| **PRD** | `dbprdplans/09 S3`, `09 S6` |

> STATUS: DONE / FSV-signed-off in #267; metric semantics hardening signed off
> in #351; per-slot calibration-bound hardening signed off in #354; legacy health
> JSON serde compatibility signed off in #358; retry after hook backpressure
> signed off in #355. Latest implementation commit: `bd544a5`. Durable aiwonder
> evidence: `/home/croyse/calyx/data/fsv-issue355-drift-retry-20260609-bd544a5`.

## Goal

Track rolling rejection/OOD rate per slot over a sliding window of recent guard
calls; when the rejection rate creeps above that slot's calibrated FAR bound,
fire the Anneal recalibration hook and emit a structured alert. `guard_health()`
returns current rejection rate, per-slot calibrated FAR bound, calibration FRR,
drift flag, and `last_calibrated` timestamp per guard. The drift monitor must
not block the guard hot path: it receives verdicts through a bounded channel.

## Build (checklist of concrete, code-level steps)

- [x] Define `AnnealHook` trait (sync, object-safe):
      `fn on_rejection_rate_drift(&self, guard_id: GuardId, slot: SlotId,
      current_rejection_rate: f32, calibrated_far_bound: f32)`; the real impl
      calls Anneal's recalibration queue (PH48); the test impl records calls in
      a `Vec`.
- [x] Define `DriftMonitor` struct:
      `guard_id: GuardId`, `window_size: usize` (rolling window, default 500),
      `per_slot_results: BTreeMap<SlotId, VecDeque<bool>>`
      (true=pass, false=fail),
      `calibrated_far_bound: BTreeMap<SlotId, f32>`,
      `anneal_hook: Arc<dyn AnnealHook>`,
      `hook_channel: SyncSender<DriftEvent>` (bounded, capacity 32).
- [x] Implement `DriftMonitor::record_verdict(&mut self, verdict: &GuardVerdict)`:
      - For each `SlotVerdict` in `verdict.per_slot`:
        - Push `v.pass` into `per_slot_results[slot]`; pop front if
          `> window_size`.
      - After each update, compute rolling `rejection_rate_k =
        fail_count_k / window_k` for each slot.
      - If `rejection_rate_k > calibrated_far_bound_k * 1.5` (50% relative
        creep): send `DriftEvent` on the channel; non-blocking (`try_send`);
        drop on full.
- [x] Spawn a background thread in `DriftMonitor::new()` that reads from the
      channel and calls `anneal_hook.on_rejection_rate_drift(..)`; the thread
      exits when sender is dropped.
- [x] Implement `guard_health(monitor: &DriftMonitor, guard_id: GuardId)
      -> GuardHealth`:
      `GuardHealth { guard_id, per_slot_rejection_rate: BTreeMap<SlotId,f32>,
      per_slot_calibrated_far_bound: BTreeMap<SlotId,f32>,
      per_slot_frr: BTreeMap<SlotId,f32>, drift: bool, last_calibrated: i64 }`
      where `drift = any slot's rolling_rejection_rate >
      calibrated_far_bound * 1.5`.
- [x] Wire `drift.rs` into `lib.rs`.

## Tests (synthetic, deterministic: known input -> known bytes/number)

- [x] unit: inject 500 verdicts with known pass/reject rates (seed=42); assert
      rolling rejection rate matches expected ratio within +/-0.01.
- [x] unit: inject 501 verdicts where last 50 are all fails (drift scenario);
      assert `guard_health().drift == true` and hook was called once.
- [x] unit: hook call count via test impl; after the 501st verdict above, hook
      fired at least once; `guard_id` and `slot` passed correctly.
- [x] unit: window resets correctly: after a window of all-pass verdicts (1000),
      rejection rate drops to 0.0; `drift == false`.
- [x] edge: `window_size = 1`: each verdict overwrites the window; rolling
      rejection rate is either 0.0 or 1.0.
- [x] edge: channel full (32 events pending): 33rd `try_send` drops silently
      (no panic, no block).
- [x] fail-closed: `guard_health()` on an unknown `guard_id` returns all zeros;
      does not panic.
- [x] regression: health exposes distinct per-slot calibrated FAR bounds and
      drift hook comparison uses each slot's own bound.

## FSV (read the bytes on aiwonder: the truth gate)

- **SoT:** durable aiwonder evidence root containing `GuardHealth` JSON before
  drift, after injected drift, after recovery, hook event readback JSON, and a
  SHA-256 manifest.
- **Readback:** run the manual FSV fixture with `CALYX_WARD_DRIFT_FSV_DIR=$root`,
  then separately inspect the JSON/log artifacts with `xxd`, `sha256sum`, and
  parsed JSON.
- **Prove:** durable readback shows `drift=true` after the injected drift
  scenario, a recorded hook event,
  `runtime_rejection_rate >= calibrated_far_bound * 1.5`, and `drift=false`
  after a full window of passes.
- **Per-slot-bound evidence:** #354 readback shows
  `per_slot_calibrated_far_bound = {"1":0.01,"2":0.05}`, per-slot FRR
  `{"1":1.0,"2":0.0}`, and a hook event for slot 1 using the slot 1 FAR bound.
- **Serde compatibility evidence:** #358 readback shows legacy health JSON
  without `per_slot_calibrated_far_bound` deserializes with an empty bound map
  and reserializes with the new field present.
- **Retry evidence:** #355 readback shows `dropped_before_retry=1`,
  `slot3_notified_before_retry=false`, `slot3_notified_after_retry=true`, and
  drift true both before and after retry.
- **Evidence:** `case-summary.json`
  `805d5d32accb704caa2b22c5f268621e38f8fbd42f2bbb770d8b0501189b6c52`,
  `before-health.json`
  `32db9e8167c45e3a840d5fe1f93d165a31663d6988f4438aeef160b9967f6a50`,
  `after-drift-health.json`
  `6c92a79d98e42a51fe7af1c03c8b8305f526899f54168eb3c5a7619f49c881ee`,
  `after-recovery-health.json`
  `32db9e8167c45e3a840d5fe1f93d165a31663d6988f4438aeef160b9967f6a50`,
  `hook-events.json`
  `3ccc7666288288316718c909729ab6b02a5b89822f884452654efc0fe8b123af`,
  `unknown-guard-health.json`
  `4dc50a1e951fb81db402cb9ae7677e187cd442ba59a8e8b8b73d9108e5b527f3`,
  log
  `48f8c27c7798f930a897e5f134833860b9dd452adc80d7de59a33ad2e70a1899`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] file(s) <= 500 lines (line-count gate).
- [x] FSV evidence (readback output / screenshot) attached to the PH38 GitHub
      issue.
- [x] no anti-pattern (DOCTRINE S9): no flatten / no `C(N,2)` past DPI /
      nothing "trusted" without grounding / no frozen-lens mutation / no
      harness-as-FSV.
