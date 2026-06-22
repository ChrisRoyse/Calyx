use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use calyx_core::SlotId;
use calyx_ward::{
    AnnealHook, CalibrationMeta, DriftEvent, DriftMonitor, GuardId, GuardPolicy, GuardProfile,
    GuardVerdict, NoveltyAction, SlotVerdict, guard_health,
};
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn full_channel_drift_notification_retries_after_recovery() {
    let hook = BlockingRecordingHook::default();
    let controls = hook.clone();
    let profile = profile_with_slots(3);
    let mut monitor = DriftMonitor::with_channel_capacity(&profile, 1, 1, Arc::new(hook));

    monitor.record_verdict(&verdict(slot(1), false));
    controls.wait_started();
    monitor.record_verdict(&verdict(slot(2), false));
    monitor.record_verdict(&verdict(slot(3), false));

    assert!(monitor.dropped_events() >= 1);
    assert!(!controls.has_event_for(slot(3)));
    assert!(guard_health(&monitor, guard_id()).drift);

    controls.release();
    controls.wait_for_event_for(slot(2));
    monitor.record_verdict(&verdict(slot(3), false));
    controls.wait_for_event_for(slot(3));

    assert!(controls.has_event_for(slot(3)));
    assert!(guard_health(&monitor, guard_id()).drift);
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_DRIFT_RETRY_FSV_DIR"]
fn drift_retry_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_WARD_DRIFT_RETRY_FSV_DIR")
        .expect("CALYX_WARD_DRIFT_RETRY_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");
    let hook = BlockingRecordingHook::default();
    let controls = hook.clone();
    let profile = profile_with_slots(3);
    let mut monitor = DriftMonitor::with_channel_capacity(&profile, 1, 1, Arc::new(hook));

    monitor.record_verdict(&verdict(slot(1), false));
    controls.wait_started();
    monitor.record_verdict(&verdict(slot(2), false));
    monitor.record_verdict(&verdict(slot(3), false));
    let before_retry_health = guard_health(&monitor, guard_id());
    let before_retry_events = controls.events();
    let dropped_before_retry = monitor.dropped_events();

    controls.release();
    controls.wait_for_event_for(slot(2));
    monitor.record_verdict(&verdict(slot(3), false));
    controls.wait_for_event_for(slot(3));
    let after_retry_health = guard_health(&monitor, guard_id());
    let after_retry_events = controls.events();

    write_json(&root, "before-retry-health.json", &before_retry_health);
    write_json(&root, "before-retry-events.json", &before_retry_events);
    write_json(&root, "after-retry-health.json", &after_retry_health);
    write_json(&root, "after-retry-events.json", &after_retry_events);
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "dropped_before_retry": dropped_before_retry,
            "dropped_after_retry": monitor.dropped_events(),
            "slot3_notified_before_retry": has_slot(&before_retry_events, slot(3)),
            "slot3_notified_after_retry": has_slot(&after_retry_events, slot(3)),
            "before_retry_drift": before_retry_health.drift,
            "after_retry_drift": after_retry_health.drift,
            "before_retry_event_count": before_retry_events.len(),
            "after_retry_event_count": after_retry_events.len(),
        }),
    );

    println!(
        "FSV_DRIFT_RETRY dropped_before_retry={} dropped_after_retry={} slot3_before={} slot3_after={} before_drift={} after_drift={}",
        dropped_before_retry,
        monitor.dropped_events(),
        has_slot(&before_retry_events, slot(3)),
        has_slot(&after_retry_events, slot(3)),
        before_retry_health.drift,
        after_retry_health.drift,
    );
}

#[derive(Clone, Default)]
struct BlockingRecordingHook {
    state: Arc<(Mutex<HookState>, Condvar)>,
}

#[derive(Default)]
struct HookState {
    started: bool,
    released: bool,
    events: Vec<DriftEvent>,
}

impl AnnealHook for BlockingRecordingHook {
    fn on_rejection_rate_drift(
        &self,
        guard_id: GuardId,
        slot: SlotId,
        current_rejection_rate: f32,
        calibrated_far_bound: f32,
    ) {
        let (lock, cv) = &*self.state;
        let mut state = lock.lock().expect("hook state");
        state.started = true;
        state.events.push(DriftEvent {
            guard_id,
            slot,
            current_rejection_rate,
            calibrated_far_bound,
        });
        cv.notify_all();
        while !state.released {
            state = cv.wait(state).expect("wait release");
        }
    }
}

impl BlockingRecordingHook {
    fn wait_started(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let (lock, cv) = &*self.state;
        let mut state = lock.lock().expect("hook state");
        while !state.started {
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for hook start");
            let timeout = deadline.saturating_duration_since(now);
            state = cv.wait_timeout(state, timeout).expect("wait started").0;
        }
    }

    fn release(&self) {
        let (lock, cv) = &*self.state;
        let mut state = lock.lock().expect("hook state");
        state.released = true;
        cv.notify_all();
    }

    fn events(&self) -> Vec<DriftEvent> {
        self.state.0.lock().expect("hook state").events.clone()
    }

    fn has_event_for(&self, slot: SlotId) -> bool {
        has_slot(&self.events(), slot)
    }

    fn wait_for_event_for(&self, slot: SlotId) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.has_event_for(slot) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for hook event for slot {slot}");
    }
}

fn profile_with_slots(slot_count: u16) -> GuardProfile {
    let mut tau = BTreeMap::new();
    let mut required_slots = Vec::new();
    for id in 1..=slot_count {
        let slot = slot(id);
        tau.insert(slot, 0.50);
        required_slots.push(slot);
    }
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic-drift-retry".to_string(),
        tau,
        required_slots,
        policy: GuardPolicy::AllRequired,
        calibration: Some(CalibrationMeta {
            corpus_hash: [7; 32],
            estimator: "unit".to_string(),
            far: 0.0,
            frr: 0.0,
            confidence: 0.99,
            ts: 1_786_233_600,
            per_slot: BTreeMap::new(),
        }),
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn verdict(slot_id: SlotId, pass: bool) -> GuardVerdict {
    GuardVerdict {
        guard_id: guard_id(),
        overall_pass: pass,
        provisional: false,
        per_slot: vec![SlotVerdict {
            slot: slot_id,
            cos: if pass { 0.91 } else { 0.40 },
            tau: 0.70,
            pass,
        }],
        action: None,
    }
}

fn has_slot(events: &[DriftEvent], slot: SlotId) -> bool {
    events.iter().any(|event| event.slot == slot)
}

fn write_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
    let path = std::path::Path::new(root).join(name);
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
