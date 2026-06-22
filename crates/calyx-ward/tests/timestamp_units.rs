use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use calyx_core::{FixedClock, SlotId};
use calyx_ward::{
    AnnealHook, CalibrationInput, CalibrationMeta, DriftMonitor, GuardId, GuardPolicy,
    GuardProfile, MatchedSlots, NoveltyAction, NoveltyHandler, NoveltyRecord, ProducedSlots,
    SlotKind, VaultSink, WardError, calibrate_slot, guard, guard_health,
};
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c357";
const CLOCK_MS: u64 = 1_786_233_600_123;

#[test]
fn ward_timestamp_surfaces_use_clock_millis() {
    let readback = timestamp_readback();

    assert_eq!(readback.calibration_ts, readback.expected_ms);
    assert_eq!(readback.novelty_ts, readback.expected_ms);
    assert_eq!(readback.guard_health_last_calibrated, readback.expected_ms);
}

#[test]
fn timestamp_conversion_edges_are_saturating_millis() {
    assert_eq!(meta_ts(0), 0);
    assert_eq!(meta_ts(i64::MAX as u64), i64::MAX);
    assert_eq!(meta_ts(i64::MAX as u64 + 1), i64::MAX);
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_TIMESTAMP_FSV_DIR"]
fn timestamp_units_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_WARD_TIMESTAMP_FSV_DIR")
        .expect("CALYX_WARD_TIMESTAMP_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let readback = timestamp_readback();
    let edge_cases = json!({
        "zero_clock_ts": meta_ts(0),
        "max_i64_clock_ts": meta_ts(i64::MAX as u64),
        "over_i64_clock_ts": meta_ts(i64::MAX as u64 + 1),
    });
    let summary = json!({
        "expected_clock_ms": readback.expected_ms,
        "calibration_ts": readback.calibration_ts,
        "novelty_ts": readback.novelty_ts,
        "guard_health_last_calibrated": readback.guard_health_last_calibrated,
        "all_match": readback.all_match(),
    });

    write_json(&root, "calibration-meta.json", &readback.calibration);
    write_json(&root, "novelty-record.json", &readback.novelty);
    write_json(&root, "guard-health.json", &readback.guard_health);
    write_json(&root, "timestamp-edge-cases.json", &edge_cases);
    write_json(&root, "case-summary.json", &summary);

    println!(
        "FSV_PH38_TS expected_ms={} calibration_ts={} novelty_ts={} health_ts={} all_match={}",
        readback.expected_ms,
        readback.calibration_ts,
        readback.novelty_ts,
        readback.guard_health_last_calibrated,
        readback.all_match()
    );
}

#[derive(Clone, Debug)]
struct TimestampReadback {
    expected_ms: i64,
    calibration_ts: i64,
    novelty_ts: i64,
    guard_health_last_calibrated: i64,
    calibration: CalibrationMeta,
    novelty: NoveltyRecord,
    guard_health: calyx_ward::GuardHealth,
}

impl TimestampReadback {
    fn all_match(&self) -> bool {
        self.calibration_ts == self.expected_ms
            && self.novelty_ts == self.expected_ms
            && self.guard_health_last_calibrated == self.expected_ms
    }
}

fn timestamp_readback() -> TimestampReadback {
    let clock = FixedClock::new(CLOCK_MS);
    let (tau, calibration) =
        calibrate_slot(&calibration_input(), 0.05, &clock).expect("calibration");
    let profile = profile(tau, Some(calibration.clone()));
    let produced = produced_slots();
    let matched = matched_slots();
    let verdict = guard(&profile, &produced, &matched, false).expect("failing verdict");
    let sink = MemorySink::default();
    let handler = NoveltyHandler::new(Arc::new(sink), Arc::new(FixedClock::new(CLOCK_MS)));
    let novelty = handler
        .handle(&profile, &verdict, &produced)
        .expect("novelty record");
    let monitor = DriftMonitor::new(&profile, 500, Arc::new(NoopHook));
    let guard_health = guard_health(&monitor, guard_id());

    TimestampReadback {
        expected_ms: i64::try_from(CLOCK_MS).expect("clock ms fits i64"),
        calibration_ts: calibration.ts,
        novelty_ts: novelty.ts,
        guard_health_last_calibrated: guard_health.last_calibrated,
        calibration,
        novelty,
        guard_health,
    }
}

fn meta_ts(clock_ms: u64) -> i64 {
    CalibrationMeta::new(
        [1; 32],
        "synthetic",
        0.0,
        0.0,
        0.95,
        &FixedClock::new(clock_ms),
    )
    .ts
}

fn calibration_input() -> CalibrationInput {
    CalibrationInput {
        slot: slot(1),
        good_scores: (0..100).map(|i| 0.80 + i as f32 * 0.001).collect(),
        bad_scores: (0..100).map(|i| 0.30 + i as f32 * 0.003).collect(),
        slot_kind: SlotKind::Identity,
        target_far: 0.01,
    }
}

fn profile(tau: f32, calibration: Option<CalibrationMeta>) -> GuardProfile {
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 38_357,
        domain: "synthetic-timestamp".to_string(),
        tau: BTreeMap::from([(slot(1), tau)]),
        required_slots: vec![slot(1)],
        policy: GuardPolicy::AllRequired,
        calibration,
        novelty_action: NoveltyAction::NewRegion,
    }
}

fn produced_slots() -> ProducedSlots {
    ProducedSlots::from([(slot(1), vec![1.0, 0.0])])
}

fn matched_slots() -> MatchedSlots {
    MatchedSlots::from([(slot(1), vec![0.40, (1.0_f32 - 0.40 * 0.40).sqrt()])])
}

#[derive(Clone, Default)]
struct MemorySink {
    records: Arc<Mutex<Vec<NoveltyRecord>>>,
}

impl VaultSink for MemorySink {
    fn write_novel(&self, record: &NoveltyRecord) -> Result<(), WardError> {
        self.records
            .lock()
            .expect("records lock")
            .push(record.clone());
        Ok(())
    }

    fn novel_records(&self) -> Result<Vec<NoveltyRecord>, WardError> {
        Ok(self.records.lock().expect("records lock").clone())
    }
}

struct NoopHook;

impl AnnealHook for NoopHook {
    fn on_rejection_rate_drift(
        &self,
        _guard_id: GuardId,
        _slot: SlotId,
        _current_rejection_rate: f32,
        _calibrated_far_bound: f32,
    ) {
    }
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
