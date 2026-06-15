# PH72 · T03 — `subscribe` / `observe_delta` stream API

| Field | Value |
|---|---|
| **Phase** | PH72 — Streaming + Reactive + Time-Travel + Universal Summarization |
| **Stage** | S20 — Critical Capabilities |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/reactive/subscription.rs` (<=500) plus `reactive/engine.rs` dispatch hooks |
| **Depends on** | T02 (reactive trigger/subscription engine) |
| **Axioms** | A26, A16, A15 |
| **PRD** | `17 §8` |

## Goal

Expose the public `subscribe` / `observe_delta` API over the `ReactiveEngine`: a
caller registers a `TriggerCondition` via `subscribe` and receives a
`SubscriptionId`; they then call `observe_delta(sub_id)` to drain all
`TriggerFired` events accumulated since the last drain. The API is synchronous and
pull-based (no async executor dependency in the core crate); a thin
`observe_delta_stream` adapter yields an iterator. Each subscription has its own
bounded drain buffer (A26). Subscription state is Ledger-provenanced (A15):
`subscribe` writes a `SUBSCRIPTION_CREATED` Ledger entry; `unsubscribe` writes
`SUBSCRIPTION_REMOVED`.

## Build (checklist of concrete, code-level steps)

- [ ] `SubscriptionId = Uuid v7` newtype; derive `Debug, Clone, Copy, PartialEq, Eq, Hash`
- [ ] `SubscriptionHandle { id: SubscriptionId, condition: TriggerCondition, drain_buf: VecDeque<TriggerFired>, max_drain_buf: usize }` — `max_drain_buf` default 1024; overflow drops oldest + returns `CALYX_REACTIVE_DRAIN_OVERFLOW` on next `observe_delta`
- [ ] `SubscriptionStore { handles: HashMap<SubscriptionId, SubscriptionHandle>, max_subscriptions: usize }` — separate from `TriggerRegistry` (subscriptions are user-facing; triggers are internal); `max_subscriptions` default 256
- [ ] `fn subscribe(engine: &mut ReactiveEngine, condition: TriggerCondition, clock: &dyn Clock) -> Result<SubscriptionId, CalyxError>` — creates a `TriggerDef` in the registry AND a `SubscriptionHandle`; writes `SUBSCRIPTION_CREATED` Ledger entry with `sub_id` and `TriggerCondition` snapshot; returns `CALYX_REACTIVE_REGISTRY_FULL` if at capacity
- [ ] `fn observe_delta(engine: &mut ReactiveEngine, sub_id: SubscriptionId) -> Result<Vec<TriggerFired>, CalyxError>` — drains `handle.drain_buf`, returns drained events; clears the buffer; returns `CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND` if `sub_id` unknown; returns `CALYX_REACTIVE_DRAIN_OVERFLOW` (with partial events) if overflow had occurred since last drain
- [ ] `fn observe_delta_stream<'a>(engine: &'a mut ReactiveEngine, sub_id: SubscriptionId) -> impl Iterator<Item=TriggerFired> + 'a` — thin wrapper that calls `observe_delta` once and yields the drained events; no background thread
- [ ] `fn unsubscribe(engine: &mut ReactiveEngine, sub_id: SubscriptionId, clock: &dyn Clock) -> Result<(), CalyxError>` — removes handle + deregisters the trigger; writes `SUBSCRIPTION_REMOVED` Ledger entry; returns `CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND` if not present
- [ ] `ReactiveEngine::dispatch_to_subscriptions(fired: &TriggerFired)` — called from `evaluate_post_ingest` when a trigger fires; finds all `SubscriptionHandle`s whose `trigger_id` matches and appends the `TriggerFired` to their `drain_buf`; on overflow sets `handle.overflow = true`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `subscribe(EventRecurs { min_occurrences: 1 })`; fire trigger via `evaluate_post_ingest`; `observe_delta` returns exactly one `TriggerFired`; second `observe_delta` immediately after returns empty vec
- [ ] unit: two subscriptions with different conditions; fire event matching only condition A → `observe_delta(sub_A)` returns 1 event; `observe_delta(sub_B)` returns 0 events; neither interferes with the other's drain buffer
- [ ] unit: `unsubscribe(sub_id)`; subsequent `observe_delta(sub_id)` returns `CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND`; Ledger contains `SUBSCRIPTION_REMOVED` entry (assert last ledger entry's kind field)
- [ ] proptest: `∀ n_fires ∈ [0, 512]`: fire trigger `n_fires` times; `observe_delta` returns exactly `min(n_fires, max_drain_buf)` events; if `n_fires > max_drain_buf`, result includes `CALYX_REACTIVE_DRAIN_OVERFLOW` error code
- [ ] edge: `subscribe` at `max_subscriptions` limit → returns `CALYX_REACTIVE_REGISTRY_FULL`; existing subscriptions unaffected
- [ ] edge: `observe_delta` on an empty buffer (no fires) → returns empty `Vec`; no error; no panic
- [ ] edge: `observe_delta_stream` exhausted → yields 0 items on second iteration without re-drain
- [ ] fail-closed: `observe_delta` with a `SubscriptionId` that was never registered → `CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND` (not a panic, not a zero-length result)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Ledger entries for `SUBSCRIPTION_CREATED` and `SUBSCRIPTION_REMOVED`; the `observe_delta` return value after a real streaming ingest that fires a recurring-event trigger
- **Readback:** `calyx readback ledger-tail --vault $VAULT_PATH --n 10` → shows `SUBSCRIPTION_CREATED` entry with `sub_id` and condition snapshot; after the recurring event fires: `calyx readback trigger-fired --sub-id <sub_id> --vault $VAULT_PATH` → prints the drained `TriggerFired` with Ledger ref; `calyx readback ledger-entry <ledger_ref>` → confirms the ingest that triggered it
- **Prove:** subscribe to `EventRecurs { series_id: S, min_occurrences: 2 }` → Ledger entry written (verify `sub_id` present); ingest series event once → `observe_delta` returns 0; ingest again → `observe_delta` returns exactly 1 `TriggerFired`; the `ledger_ref` inside `TriggerFired` points to the 2nd ingest WAL record (byte-compare seq numbers); `unsubscribe` → Ledger `SUBSCRIPTION_REMOVED` entry present; `observe_delta` after → `CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH72 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV

## #573 implementation note

- `SubscriptionId`, `SubscriptionHandle`, and `SubscriptionStore` live in
  `crates/calyx-loom/src/reactive/subscription.rs`.
- `ReactiveEngine::subscribe`, `observe_delta`, `observe_delta_report`,
  `observe_delta_stream`, and `unsubscribe` expose the synchronous pull API.
- `ReactiveEngine::subscribe_durable` and `unsubscribe_durable` append
  `SUBSCRIPTION_CREATED` / `SUBSCRIPTION_REMOVED` ledger entries tagged
  `reactive_subscription_v1`.
- Fired events are dispatched to the matching subscription buffer from both
  `evaluate_post_ingest` and `evaluate_post_ingest_durable`.
- `observe_delta` fails closed with
  `CALYX_REACTIVE_SUBSCRIPTION_NOT_FOUND` for unknown ids and
  `CALYX_REACTIVE_DRAIN_OVERFLOW` after a bounded-buffer overflow; callers can
  use `observe_delta_report` to drain the retained events and see the overflow
  flag.
