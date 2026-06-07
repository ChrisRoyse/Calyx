# PH38 · T04 — `DriftMonitor` + Anneal hook + `guard_health()`

| Field | Value |
|---|---|
| **Phase** | PH38 — τ Calibration (Conformal) + Novelty → New Region |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/drift.rs` (≤500) |
| **Depends on** | T03 (this phase) · PH48 (Anneal — stub hook until live) |
| **Axioms** | A12, A14 |
| **PRD** | `dbprdplans/09 §3`, `09 §6` |

## Goal

Track rolling FAR per slot over a sliding window of recent guard calls; when
FAR creeps above the calibrated `CalibrationMeta.far` bound, fire the Anneal
recalibration hook and emit a structured alert. `guard_health()` returns current
FAR, FRR, drift flag, and `last_calibrated` timestamp per guard. The drift
monitor must not block the guard hot path — it receives verdicts through a
bounded channel.

## Build (checklist of concrete, code-level steps)

- [ ] Define `AnnealHook` trait (sync, object-safe):
      `fn on_far_drift(&self, guard_id: GuardId, slot: SlotId, current_far: f32,
      calibrated_far: f32)` — the real impl calls Anneal's recalibration queue
      (PH48); the test impl records calls in a `Vec`
- [ ] Define `DriftMonitor` struct:
      `guard_id: GuardId`, `window_size: usize` (rolling window, default 500),
      `per_slot_results: BTreeMap<SlotId, VecDeque<bool>>` (true=pass, false=fail),
      `calibrated_far: BTreeMap<SlotId, f32>`,
      `anneal_hook: Arc<dyn AnnealHook>`,
      `hook_channel: SyncSender<DriftEvent>` (bounded, capacity 32)
- [ ] Implement `DriftMonitor::record_verdict(&mut self, verdict: &GuardVerdict)`:
      - For each `SlotVerdict` in `verdict.per_slot`:
        - Push `v.pass` into `per_slot_results[slot]`; pop front if `> window_size`
      - After each update, compute rolling `far_k = fail_count_k / window_k`
        for each slot
      - If `far_k > calibrated_far_k * 1.5` (50% relative creep): send
        `DriftEvent` on the channel; non-blocking (`try_send`; drop on full)
- [ ] Spawn a background thread in `DriftMonitor::new()` that reads from the
      channel and calls `anneal_hook.on_far_drift(..)`; the thread exits when
      sender is dropped
- [ ] Implement `guard_health(monitor: &DriftMonitor, guard_id: GuardId)
      -> GuardHealth`:
      `GuardHealth { guard_id, per_slot_far: BTreeMap<SlotId,f32>,
      per_slot_frr: BTreeMap<SlotId,f32>, drift: bool, last_calibrated: i64 }`
      where `drift = any slot's rolling_far > calibrated_far * 1.5`
- [ ] Wire `drift.rs` into `lib.rs`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: inject 500 verdicts with known pass rates (seed=42); assert rolling
      FAR matches expected ratio within ±0.01
- [ ] unit: inject 501 verdicts where last 50 are all fails (drift scenario);
      assert `guard_health().drift == true` and hook was called once
- [ ] unit: hook call count via test impl; after 501st verdict above, hook fired
      ≥ 1 time; `guard_id` and `slot` passed correctly
- [ ] unit: window resets correctly — after a window of all-pass verdicts (1000),
      FAR drops to 0.0; `drift == false`
- [ ] edge: `window_size = 1` — each verdict overwrites the window; rolling FAR
      is either 0.0 or 1.0
- [ ] edge: channel full (32 events pending) — 33rd `try_send` drops silently
      (no panic, no block)
- [ ] fail-closed: `guard_health()` on an unknown `guard_id` returns all zeros;
      does not panic

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test stdout showing `GuardHealth` struct output
- **Readback:**
  `cargo test -p calyx-ward drift -- --nocapture 2>&1 | grep -E "drift|far|hook"`
- **Prove:** output shows `drift: true` after the injected drift scenario;
  hook was called; `far` value ≥ calibrated bound * 1.5; after full window
  of passes, `drift: false`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH38 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
