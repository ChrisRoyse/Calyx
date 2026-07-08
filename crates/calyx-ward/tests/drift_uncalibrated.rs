use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use calyx_core::SlotId;
use calyx_ward::{
    AnnealHook, DriftEvent, DriftMonitor, GuardId, GuardPolicy, GuardProfile, GuardVerdict,
    NoveltyAction, SlotVerdict, guard_health,
};

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn uncalibrated_profile_reports_rates_without_latching_drift() {
    let hook = RecordingHook::default();
    let hook_readback = hook.clone();
    let profile = uncalibrated_profile();
    let mut monitor = DriftMonitor::new(&profile, 3, Arc::new(hook));
    let before = guard_health(&monitor, guard_id());
    println!(
        "drift-before: last_calibrated={} drift={} far_slots={} rejection_slots={} hook_events={}",
        before.last_calibrated,
        before.drift,
        before.per_slot_calibrated_far_bound.len(),
        before.per_slot_rejection_rate.len(),
        hook_readback.events().len()
    );

    monitor.record_verdict(&failed_verdict());
    let health = guard_health(&monitor, guard_id());
    println!(
        "drift-after: last_calibrated={} drift={} far_slots={} rejection_rate_slot1={:?} hook_events={}",
        health.last_calibrated,
        health.drift,
        health.per_slot_calibrated_far_bound.len(),
        health.per_slot_rejection_rate.get(&slot()),
        hook_readback.events().len()
    );

    assert_eq!(health.last_calibrated, 0);
    assert!(!health.drift);
    assert_eq!(health.per_slot_calibrated_far_bound.len(), 0);
    assert_eq!(health.per_slot_frr.len(), 0);
    assert_eq!(health.per_slot_rejection_rate.get(&slot()), Some(&1.0));
    assert!(hook_readback.events().is_empty());
}

#[derive(Clone, Default)]
struct RecordingHook {
    events: Arc<Mutex<Vec<DriftEvent>>>,
}

impl RecordingHook {
    fn events(&self) -> Vec<DriftEvent> {
        self.events.lock().expect("events lock").clone()
    }
}

impl AnnealHook for RecordingHook {
    fn on_rejection_rate_drift(
        &self,
        guard_id: GuardId,
        slot: SlotId,
        current_rejection_rate: f32,
        calibrated_far_bound: f32,
    ) {
        self.events.lock().expect("events lock").push(DriftEvent {
            guard_id,
            slot,
            current_rejection_rate,
            calibrated_far_bound,
        });
    }
}

fn uncalibrated_profile() -> GuardProfile {
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "uncalibrated-drift".to_string(),
        tau: BTreeMap::from([(slot(), 0.70)]),
        required_slots: vec![slot()],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn failed_verdict() -> GuardVerdict {
    GuardVerdict {
        guard_id: guard_id(),
        overall_pass: false,
        provisional: true,
        per_slot: vec![SlotVerdict {
            slot: slot(),
            cos: 0.40,
            tau: 0.70,
            pass: false,
        }],
        action: Some(NoveltyAction::Quarantine),
    }
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

fn slot() -> SlotId {
    SlotId::new(1)
}
