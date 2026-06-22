//! The reactive engine: registry + bounded fired-event queue + audit log, with
//! a single [`ReactiveEngine::evaluate_post_ingest`] entry point invoked after
//! each ingest completes.

use std::collections::HashMap;
use std::sync::Arc;

use calyx_core::{Clock, CxId, LedgerRef, Result};
use uuid::Uuid;

use super::{
    AuditEntry, AuditLog, BoundedQueue, DEFAULT_MAX_AUDIT_ENTRIES, DEFAULT_MAX_QUEUE_DEPTH,
    DEFAULT_MAX_TRIGGERS, NoveltyVerdict, ReactiveSignals, SubscriptionStore, TriggerCondition,
    TriggerDef, TriggerFired, TriggerId, TriggerRegistry, queue_full_error,
};

/// A bounded, audited reactive trigger engine. Holds the registry of trigger
/// definitions, a depth-capped queue of fired events, and a ring audit log.
pub struct ReactiveEngine {
    pub(crate) registry: TriggerRegistry,
    pub(crate) queue: BoundedQueue<TriggerFired>,
    pub(crate) audit_log: AuditLog,
    pub(crate) clock: Arc<dyn Clock>,
    /// Last observed occurrence count per `EventRecurs` trigger, so a fire is
    /// raised only on the ingest that *increments* the count past the bar.
    pub(crate) last_count: HashMap<TriggerId, u64>,
    pub(crate) subscriptions: SubscriptionStore,
}

impl ReactiveEngine {
    /// Creates an engine with the default A26 caps.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self::with_caps(
            clock,
            DEFAULT_MAX_TRIGGERS,
            DEFAULT_MAX_QUEUE_DEPTH,
            DEFAULT_MAX_AUDIT_ENTRIES,
        )
    }

    /// Creates an engine with explicit caps (all clamped to ≥ 1).
    pub fn with_caps(
        clock: Arc<dyn Clock>,
        max_triggers: usize,
        max_queue_depth: usize,
        max_audit_entries: usize,
    ) -> Self {
        Self {
            registry: TriggerRegistry::new(max_triggers),
            queue: BoundedQueue::new(max_queue_depth),
            audit_log: AuditLog::new(max_audit_entries),
            clock,
            last_count: HashMap::new(),
            subscriptions: SubscriptionStore::default(),
        }
    }

    /// Registers `condition` (owned by `owner`), assigning a fresh UUID v7 id and
    /// the current engine-clock `created_at`. Fails closed with
    /// [`crate::CALYX_REACTIVE_REGISTRY_FULL`] when the registry is full.
    pub fn register(
        &mut self,
        condition: TriggerCondition,
        owner: Option<String>,
    ) -> Result<TriggerId> {
        let def = TriggerDef {
            id: Uuid::now_v7(),
            condition,
            created_at: self.clock.now(),
            owner,
        };
        self.registry.register(def)
    }

    /// Removes the trigger with `id` and forgets its recurrence cursor.
    pub fn deregister(&mut self, id: TriggerId) -> bool {
        self.last_count.remove(&id);
        self.registry.deregister(id)
    }

    /// Evaluates every registered trigger against the post-ingest state exposed
    /// by `signals`, for the constellation `cx_id` whose ingest is recorded at
    /// `ingest_ledger_ref`. Appends exactly one [`AuditEntry`] per trigger
    /// (match or no-match) and enqueues a [`TriggerFired`] for each match.
    ///
    /// Returns the number of triggers that fired. Fails closed:
    /// - a signal-source error (e.g. an ungrounded constellation on `NewRegion`)
    ///   propagates and no fire is recorded for that trigger;
    /// - a queue overflow discards the oldest event, writes a
    ///   [`crate::CALYX_REACTIVE_QUEUE_FULL`] audit warning, and returns that
    ///   error after the batch.
    pub fn evaluate_post_ingest<S: ReactiveSignals>(
        &mut self,
        cx_id: CxId,
        ingest_ledger_ref: LedgerRef,
        signals: &S,
    ) -> Result<usize> {
        let now = self.clock.now();
        let mut fired = 0usize;
        let mut overflow: Option<calyx_core::CalyxError> = None;

        // Snapshot the defs so we can mutate queue/audit/cursor while iterating.
        for def in self.registry.defs().to_vec() {
            let matched = self.evaluate_condition(&def, cx_id, signals)?;
            self.audit_log.append(AuditEntry {
                eval_id: Uuid::now_v7(),
                trigger_id: def.id,
                cx_id,
                matched,
                ts: now,
                ledger_ref: ingest_ledger_ref.clone(),
                code: None,
            });
            if !matched {
                continue;
            }
            fired += 1;
            let event = TriggerFired {
                trigger_id: def.id,
                cx_id,
                fired_at: now,
                ledger_ref: ingest_ledger_ref.clone(),
                condition_snapshot: def.condition.clone(),
            };
            self.dispatch_to_subscriptions(&event);
            if let Some(dropped) = self.queue.push(event) {
                // A26: oldest event discarded — record the loss to the audit log
                // (the immutable warning the FSV reads) and remember to fail.
                self.audit_log.append(AuditEntry {
                    eval_id: Uuid::now_v7(),
                    trigger_id: def.id,
                    cx_id,
                    matched: true,
                    ts: now,
                    ledger_ref: ingest_ledger_ref.clone(),
                    code: Some(crate::CALYX_REACTIVE_QUEUE_FULL.to_string()),
                });
                overflow.get_or_insert_with(|| queue_full_error(&dropped));
            }
        }

        match overflow {
            Some(err) => Err(err),
            None => Ok(fired),
        }
    }

    /// Evaluates one condition. Returns `Ok(true)` on match, `Ok(false)` on
    /// no-match, and propagates any signal-source error (fail closed).
    pub(crate) fn evaluate_condition<S: ReactiveSignals>(
        &mut self,
        def: &TriggerDef,
        cx_id: CxId,
        signals: &S,
    ) -> Result<bool> {
        match &def.condition {
            TriggerCondition::NewRegion { tau_override } => {
                Ok(signals.novelty(cx_id, *tau_override)? == NoveltyVerdict::NewRegion)
            }
            TriggerCondition::EventRecurs {
                series,
                min_occurrences,
            } => {
                let current = signals.occurrence_count(*series)?;
                let last = self.last_count.insert(def.id, current).unwrap_or(0);
                let threshold = u64::from(*min_occurrences).max(1);
                Ok(current > last && last < threshold && current >= threshold)
            }
            TriggerCondition::DriftDetected {
                slot,
                drift_threshold,
            } => {
                let drift = signals.slot_drift(*slot)?;
                Ok(drift >= *drift_threshold)
            }
        }
    }

    /// Removes and returns all queued fired events, oldest first.
    pub fn drain_fired(&mut self) -> Vec<TriggerFired> {
        let mut out = Vec::with_capacity(self.queue.len());
        while let Some(event) = self.queue.pop() {
            out.push(event);
        }
        out
    }

    /// Read-only view of the registry.
    pub fn registry(&self) -> &TriggerRegistry {
        &self.registry
    }

    /// Read-only view of the fired-event queue.
    pub fn queue(&self) -> &BoundedQueue<TriggerFired> {
        &self.queue
    }

    /// Read-only view of the audit log (the engine's source of truth).
    pub fn audit_log(&self) -> &AuditLog {
        &self.audit_log
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CALYX_REACTIVE_QUEUE_FULL;
    use calyx_core::{CalyxError, FixedClock, SlotId};
    use std::cell::Cell;

    /// Deterministic signal stand-in. `occurrence_count` increments on every call
    /// (simulating one new occurrence per ingest); novelty/drift are fixed.
    struct ScriptedSignals {
        occ: Cell<u64>,
        novelty: NoveltyOutcome,
        drift: f32,
    }

    #[derive(Clone, Copy)]
    enum NoveltyOutcome {
        Verdict(NoveltyVerdict),
        Ungrounded,
    }

    impl ScriptedSignals {
        fn recurring() -> Self {
            Self {
                occ: Cell::new(0),
                novelty: NoveltyOutcome::Verdict(NoveltyVerdict::Grounded),
                drift: 0.0,
            }
        }
        fn with_novelty(outcome: NoveltyOutcome) -> Self {
            Self {
                occ: Cell::new(0),
                novelty: outcome,
                drift: 0.0,
            }
        }
        fn with_drift(drift: f32) -> Self {
            Self {
                occ: Cell::new(0),
                novelty: NoveltyOutcome::Verdict(NoveltyVerdict::Grounded),
                drift,
            }
        }
    }

    impl ReactiveSignals for ScriptedSignals {
        fn novelty(&self, _cx_id: CxId, _tau: Option<f32>) -> Result<NoveltyVerdict> {
            match self.novelty {
                NoveltyOutcome::Verdict(v) => Ok(v),
                NoveltyOutcome::Ungrounded => Err(CalyxError {
                    code: "CALYX_WARD_UNGROUNDED",
                    message: "constellation is ungrounded".to_string(),
                    remediation: "anchor the constellation before guarding",
                }),
            }
        }
        fn occurrence_count(&self, _series: CxId) -> Result<u64> {
            let next = self.occ.get() + 1;
            self.occ.set(next);
            Ok(next)
        }
        fn slot_drift(&self, _slot: SlotId) -> Result<f32> {
            Ok(self.drift)
        }
    }

    fn engine() -> ReactiveEngine {
        ReactiveEngine::new(Arc::new(FixedClock::new(1_000)))
    }

    fn cx() -> CxId {
        CxId::from_bytes([7u8; 16])
    }

    fn series() -> CxId {
        CxId::from_bytes([9u8; 16])
    }

    fn lref(seq: u64) -> LedgerRef {
        LedgerRef {
            seq,
            hash: [seq as u8; 32],
        }
    }

    #[test]
    fn event_recurs_fires_only_when_threshold_crossed() {
        let mut eng = engine();
        let id = eng
            .register(
                TriggerCondition::EventRecurs {
                    series: series(),
                    min_occurrences: 3,
                },
                None,
            )
            .unwrap();
        let signals = ScriptedSignals::recurring();

        // counts 1, 2 → no fire
        assert_eq!(
            eng.evaluate_post_ingest(cx(), lref(1), &signals).unwrap(),
            0
        );
        assert_eq!(
            eng.evaluate_post_ingest(cx(), lref(2), &signals).unwrap(),
            0
        );
        assert!(eng.queue().is_empty());
        // count 3 → exactly one fire
        assert_eq!(
            eng.evaluate_post_ingest(cx(), lref(3), &signals).unwrap(),
            1
        );
        assert_eq!(
            eng.evaluate_post_ingest(cx(), lref(4), &signals).unwrap(),
            0,
            "once the threshold has been crossed, later occurrences do not refire"
        );

        let fired: Vec<_> = eng.drain_fired();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].trigger_id, id);
        assert_eq!(
            fired[0].ledger_ref.seq, 3,
            "fired event carries the 3rd ingest ref"
        );
        // audit log: 4 evaluations recorded (3 no-match + 1 threshold-crossing match)
        let matched: Vec<bool> = eng.audit_log().entries().map(|e| e.matched).collect();
        assert_eq!(matched, vec![false, false, true, false]);
    }

    #[test]
    fn new_region_fires_on_novelty_only() {
        let mut eng = engine();
        eng.register(TriggerCondition::NewRegion { tau_override: None }, None)
            .unwrap();
        let novel =
            ScriptedSignals::with_novelty(NoveltyOutcome::Verdict(NoveltyVerdict::NewRegion));
        let grounded =
            ScriptedSignals::with_novelty(NoveltyOutcome::Verdict(NoveltyVerdict::Grounded));

        assert_eq!(eng.evaluate_post_ingest(cx(), lref(1), &novel).unwrap(), 1);
        assert_eq!(
            eng.evaluate_post_ingest(cx(), lref(2), &grounded).unwrap(),
            0
        );
        assert_eq!(eng.queue().len(), 1, "exactly one fire (the novel ingest)");
    }

    #[test]
    fn drift_fires_at_or_above_threshold() {
        let mut eng = engine();
        eng.register(
            TriggerCondition::DriftDetected {
                slot: SlotId::new(2),
                drift_threshold: 0.1,
            },
            None,
        )
        .unwrap();

        assert_eq!(
            eng.evaluate_post_ingest(cx(), lref(1), &ScriptedSignals::with_drift(0.05))
                .unwrap(),
            0,
            "below threshold → no fire"
        );
        assert_eq!(
            eng.evaluate_post_ingest(cx(), lref(2), &ScriptedSignals::with_drift(0.15))
                .unwrap(),
            1,
            "at/above threshold → fire"
        );
        let fired = eng.drain_fired();
        assert!(matches!(
            fired[0].condition_snapshot,
            TriggerCondition::DriftDetected { .. }
        ));
    }

    #[test]
    fn n_event_recurs_triggers_each_fire_once() {
        // proptest-style: 50 distinct min_occurrences=1 triggers, one ingest each.
        let mut eng = engine();
        for _ in 0..50 {
            eng.register(
                TriggerCondition::EventRecurs {
                    series: series(),
                    min_occurrences: 1,
                },
                None,
            )
            .unwrap();
        }
        // One evaluation: every trigger's first count is 1 ≥ 1 → fires once each.
        let fired = eng
            .evaluate_post_ingest(cx(), lref(1), &ScriptedSignals::recurring())
            .unwrap();
        assert_eq!(fired, 50);
        assert_eq!(eng.queue().len(), 50);
    }

    #[test]
    fn queue_overflow_discards_oldest_and_warns() {
        let mut eng = ReactiveEngine::with_caps(Arc::new(FixedClock::new(1_000)), 8, 2, 1024);
        eng.register(TriggerCondition::NewRegion { tau_override: None }, None)
            .unwrap();
        let novel =
            ScriptedSignals::with_novelty(NoveltyOutcome::Verdict(NoveltyVerdict::NewRegion));

        // Fill the queue to capacity (2).
        eng.evaluate_post_ingest(cx(), lref(1), &novel).unwrap();
        eng.evaluate_post_ingest(cx(), lref(2), &novel).unwrap();
        assert_eq!(eng.queue().len(), 2);

        // One more fire overflows: returns the error, queue stays at cap, and the
        // last audit row is the queue-full warning.
        let err = eng.evaluate_post_ingest(cx(), lref(3), &novel).unwrap_err();
        assert_eq!(err.code, CALYX_REACTIVE_QUEUE_FULL);
        assert_eq!(eng.queue().len(), 2, "queue bounded at capacity");
        let last = eng.audit_log().last().unwrap();
        assert_eq!(last.code.as_deref(), Some(CALYX_REACTIVE_QUEUE_FULL));
        // Oldest (seq=1) was discarded; queue now holds seq 2 and 3.
        let seqs: Vec<u64> = eng.queue().iter().map(|e| e.ledger_ref.seq).collect();
        assert_eq!(seqs, vec![2, 3]);
    }

    #[test]
    fn deregistered_trigger_is_skipped() {
        let mut eng = engine();
        let keep = eng
            .register(
                TriggerCondition::EventRecurs {
                    series: series(),
                    min_occurrences: 1,
                },
                None,
            )
            .unwrap();
        let drop = eng
            .register(TriggerCondition::NewRegion { tau_override: None }, None)
            .unwrap();
        assert!(eng.deregister(drop));
        // Only the kept EventRecurs trigger evaluates (count 1 ≥ 1 → fire).
        let fired = eng
            .evaluate_post_ingest(cx(), lref(1), &ScriptedSignals::recurring())
            .unwrap();
        assert_eq!(fired, 1);
        let fired_events = eng.drain_fired();
        assert_eq!(fired_events[0].trigger_id, keep);
        assert!(
            !eng.audit_log().entries().any(|e| e.trigger_id == drop),
            "no audit row for the deregistered trigger"
        );
    }

    #[test]
    fn ungrounded_novelty_fails_closed() {
        let mut eng = engine();
        eng.register(TriggerCondition::NewRegion { tau_override: None }, None)
            .unwrap();
        let err = eng
            .evaluate_post_ingest(
                cx(),
                lref(1),
                &ScriptedSignals::with_novelty(NoveltyOutcome::Ungrounded),
            )
            .unwrap_err();
        assert_eq!(err.code, "CALYX_WARD_UNGROUNDED");
        assert!(
            eng.queue().is_empty(),
            "no fire on a fail-closed evaluation"
        );
    }
}
