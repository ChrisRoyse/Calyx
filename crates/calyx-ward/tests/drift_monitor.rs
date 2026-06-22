use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use calyx_core::{FixedClock, SlotId};
use calyx_ward::{
    AnnealHook, CalibrationInput, CalibrationMeta, DriftEvent, DriftMonitor, GuardId, GuardPolicy,
    GuardProfile, GuardVerdict, NoveltyAction, SlotKind, SlotVerdict, calibrate, guard_health,
};
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";
const OTHER_GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c102";
const CALIBRATED_FAR: f32 = 0.05;

#[test]
fn rolling_rejection_rate_matches_known_ratio() {
    let hook = RecordingHook::default();
    let mut monitor = monitor_with_slots(1, 500, Arc::new(hook));

    record_repeated(&mut monitor, slot(1), true, 450);
    record_repeated(&mut monitor, slot(1), false, 50);

    let health = guard_health(&monitor, guard_id());
    assert_close(*health.per_slot_rejection_rate.get(&slot(1)).unwrap(), 0.10);
}

#[test]
fn rejection_rate_is_not_reported_as_calibration_far() {
    let hook = RecordingHook::default();
    let mut monitor = monitor_with_slots(1, 500, Arc::new(hook));

    record_repeated(&mut monitor, slot(1), true, 80);
    record_repeated(&mut monitor, slot(1), false, 20);

    let health = guard_health(&monitor, guard_id());
    let rejection_rate = *health.per_slot_rejection_rate.get(&slot(1)).unwrap();

    assert_close(rejection_rate, 0.20);
    assert!(rejection_rate > CALIBRATED_FAR * 3.0);
}

#[test]
fn guard_health_retains_distinct_per_slot_calibration_bounds() {
    let hook = RecordingHook::default();
    let profile = calibrated_profile_with_distinct_slot_bounds();
    let monitor = DriftMonitor::new(&profile, 100, Arc::new(hook));
    let health = guard_health(&monitor, guard_id());
    let meta = profile.calibration.as_ref().expect("profile calibration");
    let identity = meta.per_slot.get(&slot(1)).expect("identity slot");
    let stylistic = meta.per_slot.get(&slot(2)).expect("style slot");

    assert!(identity.far < stylistic.far);
    assert!(identity.frr > stylistic.frr);
    assert_eq!(
        health.per_slot_calibrated_far_bound.get(&slot(1)),
        Some(&identity.far)
    );
    assert_eq!(
        health.per_slot_calibrated_far_bound.get(&slot(2)),
        Some(&stylistic.far)
    );
    assert_eq!(health.per_slot_frr.get(&slot(1)), Some(&identity.frr));
    assert_eq!(health.per_slot_frr.get(&slot(2)), Some(&stylistic.frr));
}

#[test]
fn drift_uses_each_slots_own_calibrated_far_bound() {
    let hook = RecordingHook::default();
    let hook_readback = hook.clone();
    let profile = calibrated_profile_with_distinct_slot_bounds();
    let mut monitor = DriftMonitor::new(&profile, 100, Arc::new(hook));
    let slot1_far = profile
        .calibration
        .as_ref()
        .and_then(|meta| meta.per_slot.get(&slot(1)))
        .map(|meta| meta.far)
        .expect("slot 1 far");

    record_repeated(&mut monitor, slot(1), true, 99);
    record_repeated(&mut monitor, slot(1), false, 1);
    record_repeated(&mut monitor, slot(2), true, 99);
    record_repeated(&mut monitor, slot(2), false, 1);
    wait_for_events(&hook_readback, 1);

    let events = hook_readback.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].slot, slot(1));
    assert_close(events[0].calibrated_far_bound, slot1_far);
}

#[test]
fn injected_drift_sets_health_and_fires_hook_once() {
    let hook = RecordingHook::default();
    let hook_readback = hook.clone();
    let mut monitor = monitor_with_slots(1, 500, Arc::new(hook));

    record_repeated(&mut monitor, slot(1), true, 451);
    record_repeated(&mut monitor, slot(1), false, 50);
    wait_for_events(&hook_readback, 1);

    let health = guard_health(&monitor, guard_id());
    let events = hook_readback.events();

    assert!(health.drift);
    assert_close(*health.per_slot_rejection_rate.get(&slot(1)).unwrap(), 0.10);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].guard_id, guard_id());
    assert_eq!(events[0].slot, slot(1));
    assert!(events[0].current_rejection_rate >= CALIBRATED_FAR * 1.5);
    assert_eq!(events[0].calibrated_far_bound, CALIBRATED_FAR);
}

#[test]
fn all_pass_window_recovers_from_drift() {
    let hook = RecordingHook::default();
    let hook_readback = hook.clone();
    let mut monitor = monitor_with_slots(1, 500, Arc::new(hook));

    record_repeated(&mut monitor, slot(1), true, 451);
    record_repeated(&mut monitor, slot(1), false, 50);
    wait_for_events(&hook_readback, 1);
    record_repeated(&mut monitor, slot(1), true, 500);

    let health = guard_health(&monitor, guard_id());

    assert_close(*health.per_slot_rejection_rate.get(&slot(1)).unwrap(), 0.0);
    assert!(!health.drift);
}

#[test]
fn window_size_one_overwrites_previous_verdict() {
    let hook = RecordingHook::default();
    let mut monitor = monitor_with_slots(1, 1, Arc::new(hook));

    monitor.record_verdict(&verdict(slot(1), false));
    assert_close(
        *guard_health(&monitor, guard_id())
            .per_slot_rejection_rate
            .get(&slot(1))
            .unwrap(),
        1.0,
    );

    monitor.record_verdict(&verdict(slot(1), true));
    assert_close(
        *guard_health(&monitor, guard_id())
            .per_slot_rejection_rate
            .get(&slot(1))
            .unwrap(),
        0.0,
    );
}

#[test]
fn full_channel_drops_without_panic() {
    let hook = BlockingHook::default();
    let controls = hook.clone();
    let profile = profile_with_slots(34, 0.0, 0.0);
    let mut monitor = DriftMonitor::with_channel_capacity(&profile, 1, 32, Arc::new(hook));

    monitor.record_verdict(&verdict(slot(1), false));
    controls.wait_started();
    for id in 2..=34 {
        monitor.record_verdict(&verdict(slot(id), false));
    }

    assert!(monitor.dropped_events() >= 1);
    controls.release();
}

#[test]
fn unknown_guard_health_returns_zero_snapshot() {
    let hook = RecordingHook::default();
    let monitor = monitor_with_slots(1, 500, Arc::new(hook));

    let health = guard_health(&monitor, other_guard_id());

    assert_eq!(health.guard_id, other_guard_id());
    assert!(health.per_slot_rejection_rate.is_empty());
    assert!(health.per_slot_calibrated_far_bound.is_empty());
    assert!(health.per_slot_frr.is_empty());
    assert!(!health.drift);
    assert_eq!(health.last_calibrated, 0);
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_DRIFT_FSV_DIR"]
fn drift_monitor_fsv_fixture_writes_readback_artifacts() {
    let root =
        std::env::var("CALYX_WARD_DRIFT_FSV_DIR").expect("CALYX_WARD_DRIFT_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");
    let hook = RecordingHook::default();
    let hook_readback = hook.clone();
    let mut monitor = monitor_with_slots(1, 500, Arc::new(hook));

    let before = guard_health(&monitor, guard_id());
    record_repeated(&mut monitor, slot(1), true, 451);
    record_repeated(&mut monitor, slot(1), false, 50);
    wait_for_events(&hook_readback, 1);
    let after_drift = guard_health(&monitor, guard_id());
    record_repeated(&mut monitor, slot(1), true, 500);
    let after_recovery = guard_health(&monitor, guard_id());
    let unknown = guard_health(&monitor, other_guard_id());
    let events = hook_readback.events();

    write_json(&root, "before-health.json", &before);
    write_json(&root, "after-drift-health.json", &after_drift);
    write_json(&root, "after-recovery-health.json", &after_recovery);
    write_json(&root, "hook-events.json", &events);
    write_json(&root, "unknown-guard-health.json", &unknown);
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "before_drift": before.drift,
            "after_drift": after_drift.drift,
            "runtime_metric": "rejection_rate",
            "calibration_metric": "false_accept_rate_bound",
            "after_drift_rejection_rate": after_drift.per_slot_rejection_rate.get(&slot(1)),
            "calibrated_far_bound": CALIBRATED_FAR,
            "required_rejection_rate": CALIBRATED_FAR * 1.5,
            "hook_event_count": events.len(),
            "after_recovery_drift": after_recovery.drift,
            "after_recovery_rejection_rate": after_recovery.per_slot_rejection_rate.get(&slot(1)),
            "unknown_guard_slots": unknown.per_slot_rejection_rate.len(),
        }),
    );

    println!(
        "FSV_PH38_T04 drift_before={} drift_after={} rejection_rate_after={:?} hook_events={} recovery_drift={}",
        before.drift,
        after_drift.drift,
        after_drift.per_slot_rejection_rate.get(&slot(1)),
        events.len(),
        after_recovery.drift,
    );
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_PER_SLOT_CALIBRATION_FSV_DIR"]
fn per_slot_calibration_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_WARD_PER_SLOT_CALIBRATION_FSV_DIR")
        .expect("CALYX_WARD_PER_SLOT_CALIBRATION_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");
    let hook = RecordingHook::default();
    let hook_readback = hook.clone();
    let profile = calibrated_profile_with_distinct_slot_bounds();
    let mut monitor = DriftMonitor::new(&profile, 100, Arc::new(hook));

    record_repeated(&mut monitor, slot(1), true, 99);
    record_repeated(&mut monitor, slot(1), false, 1);
    record_repeated(&mut monitor, slot(2), true, 99);
    record_repeated(&mut monitor, slot(2), false, 1);
    wait_for_events(&hook_readback, 1);
    let health = guard_health(&monitor, guard_id());
    let events = hook_readback.events();
    let calibration = profile.calibration.as_ref().expect("calibration");

    write_json(&root, "calibrated-profile.json", &profile);
    write_json(&root, "guard-health.json", &health);
    write_json(&root, "hook-events.json", &events);
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "profile_far_summary": calibration.far,
            "profile_frr_summary": calibration.frr,
            "slot1_far": calibration.per_slot.get(&slot(1)).map(|meta| meta.far),
            "slot2_far": calibration.per_slot.get(&slot(2)).map(|meta| meta.far),
            "slot1_frr": calibration.per_slot.get(&slot(1)).map(|meta| meta.frr),
            "slot2_frr": calibration.per_slot.get(&slot(2)).map(|meta| meta.frr),
            "health_slot1_far": health.per_slot_calibrated_far_bound.get(&slot(1)),
            "health_slot2_far": health.per_slot_calibrated_far_bound.get(&slot(2)),
            "health_slot1_frr": health.per_slot_frr.get(&slot(1)),
            "health_slot2_frr": health.per_slot_frr.get(&slot(2)),
            "hook_event_count": events.len(),
            "hook_event_slot": events.first().map(|event| event.slot),
            "hook_event_far_bound": events.first().map(|event| event.calibrated_far_bound),
        }),
    );

    println!(
        "FSV_PH38_PER_SLOT slot1_far={:?} slot2_far={:?} slot1_frr={:?} slot2_frr={:?} health_slot1_far={:?} health_slot2_far={:?} hook_events={}",
        calibration.per_slot.get(&slot(1)).map(|meta| meta.far),
        calibration.per_slot.get(&slot(2)).map(|meta| meta.far),
        calibration.per_slot.get(&slot(1)).map(|meta| meta.frr),
        calibration.per_slot.get(&slot(2)).map(|meta| meta.frr),
        health.per_slot_calibrated_far_bound.get(&slot(1)),
        health.per_slot_calibrated_far_bound.get(&slot(2)),
        events.len(),
    );
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

#[derive(Clone, Default)]
struct BlockingHook {
    events: Arc<Mutex<Vec<DriftEvent>>>,
    started: Arc<(Mutex<bool>, Condvar)>,
    release_gate: Arc<(Mutex<bool>, Condvar)>,
}

impl BlockingHook {
    fn wait_started(&self) {
        let (lock, cvar) = &*self.started;
        let mut started = lock.lock().expect("started lock");
        while !*started {
            started = cvar.wait(started).expect("started wait");
        }
    }

    fn release(&self) {
        let (lock, cvar) = &*self.release_gate;
        *lock.lock().expect("release lock") = true;
        cvar.notify_all();
    }
}

impl AnnealHook for BlockingHook {
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
        let (started_lock, started_cvar) = &*self.started;
        *started_lock.lock().expect("started lock") = true;
        started_cvar.notify_all();

        let (release_lock, release_cvar) = &*self.release_gate;
        let mut released = release_lock.lock().expect("release lock");
        while !*released {
            released = release_cvar.wait(released).expect("release wait");
        }
    }
}

fn monitor_with_slots(
    slot_count: u16,
    window_size: usize,
    hook: Arc<dyn AnnealHook>,
) -> DriftMonitor {
    let profile = profile_with_slots(slot_count, CALIBRATED_FAR, 0.02);
    DriftMonitor::new(&profile, window_size, hook)
}

fn profile_with_slots(slot_count: u16, far: f32, frr: f32) -> GuardProfile {
    let mut tau = BTreeMap::new();
    for id in 1..=slot_count {
        tau.insert(slot(id), 0.70);
    }
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic-drift".to_string(),
        tau,
        required_slots: (1..=slot_count).map(slot).collect(),
        policy: GuardPolicy::AllRequired,
        calibration: Some(CalibrationMeta::new(
            [4; 32],
            "conformal",
            far,
            frr,
            0.95,
            &FixedClock::new(1_786_233_600),
        )),
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn calibrated_profile_with_distinct_slot_bounds() -> GuardProfile {
    let mut identity = calibration_input(slot(1), SlotKind::Identity, 0.01);
    let mut stylistic = calibration_input(slot(2), SlotKind::Stylistic, 0.05);
    identity.good_scores = vec![0.59; 100];
    stylistic.good_scores = vec![0.70; 100];
    calibrate(
        profile_template(),
        vec![identity, stylistic],
        0.05,
        &FixedClock::new(1_786_233_600),
    )
    .expect("calibrate profile")
}

fn calibration_input(slot: SlotId, slot_kind: SlotKind, target_far: f32) -> CalibrationInput {
    CalibrationInput {
        slot,
        good_scores: (0..100).map(|i| 0.80 + i as f32 * 0.001).collect(),
        bad_scores: (0..100).map(|i| 0.30 + i as f32 * 0.003).collect(),
        slot_kind,
        target_far,
    }
}

fn profile_template() -> GuardProfile {
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic-drift".to_string(),
        tau: BTreeMap::new(),
        required_slots: Vec::new(),
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn record_repeated(monitor: &mut DriftMonitor, slot_id: SlotId, pass: bool, count: usize) {
    for _ in 0..count {
        monitor.record_verdict(&verdict(slot_id, pass));
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

fn wait_for_events(hook: &RecordingHook, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if hook.events().len() >= expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for hook events");
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.01,
        "actual={actual} expected={expected}"
    );
}

fn write_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
    let path = std::path::Path::new(root).join(name);
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

fn other_guard_id() -> GuardId {
    OTHER_GUARD_UUID.parse().expect("other guard id")
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
