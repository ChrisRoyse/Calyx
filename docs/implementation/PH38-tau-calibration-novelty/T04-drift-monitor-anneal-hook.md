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

> STATUS: DONE / FSV-signed-off in #267. Implementation commit:
> `912b7072e56a679182ac77c9da6fc83bd5c25385`. Durable aiwonder evidence:
> `/home/croyse/calyx/data/fsv-issue267-ph38-t04-20260609-912b707`.

## Goal

Track rolling rejection/OOD rate per slot over a sliding window of recent guard
calls; when the rejection rate creeps above the calibrated `CalibrationMeta.far`
bound, fire the Anneal recalibration hook and emit a structured alert.
`guard_health()` returns current rejection rate, calibration FRR, drift flag, and
`last_calibrated` timestamp per guard. The drift monitor must not block the guard
hot path: it receives verdicts through a bounded channel.

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
- **Evidence:** `case-summary.json`
  `5b924a6349c0de34d88fc0611a70579fe42dc399e020bbdaeb97af9386b34403`,
  `after-drift-health.json`
  `3e28fcbf7a8af325accb7164064a7c7773f7473521575839e01d6423bd1947b1`,
  `after-recovery-health.json`
  `c690b561f2ea22b3948607866c1dabe6c65a7809ee41c38a1288ad6548ab9b1d`,
  `hook-events.json`
  `24941d0652d162550757cde449cc3191606875adf14940b3c685dde9e4a5a6b0`,
  `unknown-guard-health.json`
  `d39a56612a692ad30b861a9e5c43be8028ad9c52f015457bf50659105028db22`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] file(s) <= 500 lines (line-count gate).
- [x] FSV evidence (readback output / screenshot) attached to the PH38 GitHub
      issue.
- [x] no anti-pattern (DOCTRINE S9): no flatten / no `C(N,2)` past DPI /
      nothing "trusted" without grounding / no frozen-lens mutation / no
      harness-as-FSV.
