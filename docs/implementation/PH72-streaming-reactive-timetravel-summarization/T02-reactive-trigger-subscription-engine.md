# PH72 · T02 — Reactive trigger/subscription engine (NewRegion/Recurs/Drift), bounded + audited

| Field | Value |
|---|---|
| **Phase** | PH72 — Streaming + Reactive + Time-Travel + Universal Summarization |
| **Stage** | S20 — Critical Capabilities |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/reactive/mod.rs` (≤500), `crates/calyx-loom/src/reactive/engine.rs` (≤500) |
| **Depends on** | T01 (streaming ingest pipeline), PH37 (`Gτ` guard + novelty→new-region), PH41 (recurrence series + signature), PH38 (`τ` calibration) |
| **Axioms** | A26, A15, A16, A12 |
| **PRD** | `17 §8`, `18 §8` |

## Goal

Implement the reactive trigger and subscription engine: a bounded, audited subsystem
that evaluates `TriggerDef` conditions immediately after each `ingest_at` completes.
Three condition variants are supported — `NewRegion` (Ward novelty fires; the
constellation's Gτ guard reports novelty against the configured panel, i.e. it does
not match any existing region at calibrated τ), `EventRecurs` (the recurrence
signature fires for a known series), and `DriftDetected` (agreement-graph cosine
drift exceeds threshold). On match, a `TriggerFired` event is enqueued. The queue
is bounded (A26): on overflow, `CALYX_REACTIVE_QUEUE_FULL` is returned and the
oldest undelivered event is discarded with a Ledger warning written (A15).
An immutable audit log records every evaluation result (match or no-match) with
the Ledger reference of the ingest that triggered evaluation.

## Build (checklist of concrete, code-level steps)

- [ ] `TriggerCondition` enum: `NewRegion { panel_id: PanelId, tau_override: Option<f32> }` | `EventRecurs { series_id: CxId, min_occurrences: u32 }` | `DriftDetected { slot_id: SlotId, drift_threshold: f32 }`
- [ ] `TriggerDef { id: TriggerId, condition: TriggerCondition, created_at: Timestamp, owner: TenantId? }` where `TriggerId = Uuid v7`
- [ ] `TriggerFired { trigger_id: TriggerId, cx_id: CxId, fired_at: Timestamp, ledger_ref: LedgerRef, condition_snapshot: TriggerCondition }` — includes the Ledger ref of the ingest that caused the fire (A15)
- [ ] `TriggerRegistry { defs: HashMap<TriggerId, TriggerDef>, max_triggers: usize }` — `register(def) -> Result<TriggerId, CalyxError>` returns `CALYX_REACTIVE_REGISTRY_FULL` when `defs.len() >= max_triggers`; `deregister(id)`; `list() -> Vec<TriggerDef>`
- [ ] `ReactiveEngine { registry: TriggerRegistry, queue: BoundedQueue<TriggerFired>, audit_log: AuditLog, clock: Arc<dyn Clock> }` where `BoundedQueue` has hard capacity `max_queue_depth` (default 4096, A26)
- [ ] `ReactiveEngine::evaluate_post_ingest(cx_id, ingest_ledger_ref, vault_snapshot)` — iterates `registry.defs`; for each `TriggerCondition`, evaluates against the ingest result; on match enqueues `TriggerFired`; writes one `AuditEntry` per evaluation (match or no-match) regardless
- [ ] `NewRegion` evaluation: call `Ward::guard_cx(cx_id, panel_id, tau)` on the vault snapshot; if `GuardResult::Novelty` → fire; otherwise no-fire; never silently accepts ungrounded constellations
- [ ] `EventRecurs` evaluation: call `RecurrenceSeries::occurrence_count(series_id)` on the vault snapshot; if count incremented this ingest AND count ≥ `min_occurrences` → fire
- [ ] `DriftDetected` evaluation: compute the slot cosine delta between the current and previous snapshot for `slot_id`; if `|Δcosine| ≥ drift_threshold` → fire
- [ ] On `BoundedQueue` overflow: discard oldest `TriggerFired`, write a `CALYX_REACTIVE_QUEUE_FULL` warning entry to Ledger; return `CALYX_REACTIVE_QUEUE_FULL` to the caller
- [ ] `AuditLog` is an append-only ring buffer capped at `max_audit_entries` (default 65536, A26); `AuditEntry { eval_id: Uuid, trigger_id: TriggerId, cx_id: CxId, matched: bool, ts: Timestamp, ledger_ref: LedgerRef }`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: register one `EventRecurs { min_occurrences: 3 }` trigger; call `evaluate_post_ingest` 2× → no fire; call 3rd time → exactly one `TriggerFired` in queue with correct `trigger_id` and `ledger_ref`
- [ ] unit: register one `NewRegion` trigger; inject a `GuardResult::Novelty` mock for the test cx → fire; inject `GuardResult::Pass` → no fire; queue length reflects exactly
- [ ] unit: register one `DriftDetected { drift_threshold: 0.1 }` trigger; inject Δcosine = 0.05 → no fire; inject Δcosine = 0.15 → fire; assertion on `TriggerFired.condition_snapshot`
- [ ] proptest: `∀ n_triggers ∈ [1, 50]`, all `EventRecurs { min_occurrences: 1 }`: call `evaluate_post_ingest` once per trigger's series → exactly `n_triggers` `TriggerFired` events; queue length == n_triggers
- [ ] edge: register `max_triggers + 1` triggers → last `register` returns `CALYX_REACTIVE_REGISTRY_FULL`; existing triggers unchanged
- [ ] edge: fill queue to `max_queue_depth`; call `evaluate_post_ingest` causing one more fire → queue still at `max_queue_depth`; Ledger contains the `CALYX_REACTIVE_QUEUE_FULL` warning entry (verify via `audit_log.entries.last()`)
- [ ] edge: `deregister` a trigger mid-evaluation batch → subsequent evaluations skip the deregistered trigger without panic; no stale `TriggerFired` for deregistered id
- [ ] fail-closed: evaluate with an ungrounded constellation on a `NewRegion` trigger → `Ward::guard_cx` returns `CALYX_WARD_UNGROUNDED` → engine propagates error; no `TriggerFired` for that cx

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the `TriggerFired` queue entries and the `AuditLog` entries written after streaming ingestion of a recurring-event stream
- **Readback:** `calyx readback trigger-audit <sub_id> --vault $VAULT_PATH` → prints all `AuditEntry` rows; `calyx readback trigger-fired --vault $VAULT_PATH` → prints `TriggerFired` events; Ledger ref in each fired event is verifiable via `calyx readback ledger-entry <ledger_ref>`
- **Prove:** before: 0 triggers registered; register `EventRecurs { series_id: <known_id>, min_occurrences: 3 }` → ingest the recurring event 3 times → exactly one `TriggerFired` in the queue; the audit log shows 3 evaluation entries (2 no-match + 1 match); `ledger_ref` in the `TriggerFired` matches the WAL entry for the 3rd ingest (byte-compare the seq number); fill the queue → Ledger warning entry present

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH72 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
