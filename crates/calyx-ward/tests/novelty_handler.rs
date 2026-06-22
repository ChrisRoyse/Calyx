use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use calyx_core::{FixedClock, SlotId};
use calyx_ward::{
    CALYX_GUARD_ID_MISMATCH, CALYX_GUARD_NOT_A_FAILURE, CALYX_GUARD_NOVELTY_SINK, GuardId,
    GuardPolicy, GuardProfile, MatchedSlots, NoveltyAction, NoveltyHandler, NoveltyRecord,
    NoveltyStatus, ProducedSlots, VaultSink, WardError, guard, novel_regions,
};
use proptest::prelude::*;
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";
const FIXED_TS: u64 = 1_786_233_600_000;

#[test]
fn new_region_writes_awaiting_grounding_record() {
    let sink = MemorySink::default();
    let record = handle_action(&sink, NoveltyAction::NewRegion).expect("new region record");

    assert_eq!(record.status, NoveltyStatus::AwaitingGrounding);
    assert_eq!(record.action_taken, NoveltyAction::NewRegion);
    assert_eq!(record.guard_id, guard_id());
    assert_eq!(record.failing_verdicts.len(), 1);
    assert_eq!(record.novel_id.to_string().len(), 36);
    assert_eq!(sink.records().len(), 1);
    assert_eq!(
        novel_regions(&sink, Some(0)).expect("novel regions"),
        vec![record]
    );
}

#[test]
fn quarantine_writes_quarantined_record_without_trusting_it() {
    let sink = MemorySink::default();
    let record = handle_action(&sink, NoveltyAction::Quarantine).expect("quarantine record");

    assert_eq!(record.status, NoveltyStatus::Quarantined);
    assert_eq!(record.action_taken, NoveltyAction::Quarantine);
    assert!(record.failing_verdicts.iter().all(|slot| !slot.pass));
    assert!(
        novel_regions(&sink, Some(0))
            .expect("novel regions")
            .is_empty()
    );
}

#[test]
fn reject_closed_writes_rejected_record_then_returns_ood() {
    let sink = MemorySink::default();
    let error = handle_action(&sink, NoveltyAction::RejectClosed).expect_err("reject closed");
    let records = sink.records();

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, NoveltyStatus::Rejected);
    assert_eq!(records[0].action_taken, NoveltyAction::RejectClosed);
    assert_eq!(
        error,
        WardError::Ood {
            guard_id: guard_id(),
            failing: records[0].failing_verdicts.clone(),
        }
    );
}

proptest! {
    #[test]
    fn every_action_writes_exactly_one_record(action_index in 0u8..3) {
        let action = match action_index {
            0 => NoveltyAction::NewRegion,
            1 => NoveltyAction::Quarantine,
            _ => NoveltyAction::RejectClosed,
        };
        let sink = MemorySink::default();
        let _ = handle_action(&sink, action);

        prop_assert_eq!(sink.records().len(), 1);
    }
}

#[test]
fn sink_error_is_propagated_for_new_region_and_reject_closed() {
    let sink = FailingSink;
    let new_region = handle_action(&sink, NoveltyAction::NewRegion).expect_err("new region error");
    let reject = handle_action(&sink, NoveltyAction::RejectClosed).expect_err("reject error");

    assert_eq!(new_region.code(), CALYX_GUARD_NOVELTY_SINK);
    assert_eq!(reject.code(), CALYX_GUARD_NOVELTY_SINK);
}

#[test]
fn novel_regions_since_max_is_empty() {
    let sink = MemorySink::default();
    handle_action(&sink, NoveltyAction::NewRegion).expect("new region record");

    let records = novel_regions(&sink, Some(i64::MAX)).expect("novel regions");

    assert!(records.is_empty());
}

#[test]
fn passing_verdict_is_not_a_novelty_failure() {
    let sink = MemorySink::default();
    let (mut profile, produced, mut matched) = scenario(NoveltyAction::NewRegion);
    profile.tau.insert(slot(1), 0.70);
    matched.insert(slot(1), vec![1.0, 0.0]);
    let verdict = guard(&profile, &produced, &matched, false).expect("passing verdict");
    let handler = handler_for(sink.clone());

    let error = handler
        .handle(&profile, &verdict, &produced)
        .expect_err("not a failure");

    assert_eq!(error.code(), CALYX_GUARD_NOT_A_FAILURE);
    assert!(sink.records().is_empty());
}

#[test]
fn guard_id_mismatch_fails_before_sink_write() {
    let sink = MemorySink::default();
    let error = guard_id_mismatch_error(&sink);

    assert_eq!(error.code(), CALYX_GUARD_ID_MISMATCH);
    assert!(sink.records().is_empty());
}

#[test]
fn novelty_error_constants_are_exported_from_crate_root() {
    assert_eq!(CALYX_GUARD_NOT_A_FAILURE, "CALYX_GUARD_NOT_A_FAILURE");
    assert_eq!(CALYX_GUARD_NOVELTY_SINK, "CALYX_GUARD_NOVELTY_SINK");
    assert_eq!(CALYX_GUARD_ID_MISMATCH, "CALYX_GUARD_ID_MISMATCH");
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_NOVELTY_FSV_DIR"]
fn novelty_handler_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_WARD_NOVELTY_FSV_DIR")
        .expect("CALYX_WARD_NOVELTY_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let new_sink = MemorySink::default();
    let awaiting = handle_action(&new_sink, NoveltyAction::NewRegion).expect("new region");
    let quarantined =
        handle_action(&MemorySink::default(), NoveltyAction::Quarantine).expect("quarantine");
    let reject_sink = MemorySink::default();
    let rejected_error =
        handle_action(&reject_sink, NoveltyAction::RejectClosed).expect_err("reject closed");
    let rejected = reject_sink.records().remove(0);
    let listed = novel_regions(&new_sink, Some(0)).expect("novel regions");
    let max_since = novel_regions(&new_sink, Some(i64::MAX)).expect("max since");
    let sink_new_region_error =
        handle_action(&FailingSink, NoveltyAction::NewRegion).expect_err("new region sink error");
    let sink_reject_error =
        handle_action(&FailingSink, NoveltyAction::RejectClosed).expect_err("reject sink error");
    let not_failure_sink = MemorySink::default();
    let not_failure_error = passing_verdict_error(&not_failure_sink);
    let mismatch_sink = MemorySink::default();
    let mismatch_error = guard_id_mismatch_error(&mismatch_sink);

    write_json(&root, "new-region-record.json", &awaiting);
    write_json(&root, "quarantine-record.json", &quarantined);
    write_json(&root, "reject-tombstone-record.json", &rejected);
    write_json(&root, "reject-error.json", &error_json(&rejected_error));
    write_json(&root, "novel-regions.json", &listed);
    write_json(&root, "novel-regions-max-since.json", &max_since);
    write_json(
        &root,
        "sink-error-new-region.json",
        &error_json(&sink_new_region_error),
    );
    write_json(
        &root,
        "sink-error-reject-closed.json",
        &error_json(&sink_reject_error),
    );
    write_json(
        &root,
        "not-failure-error.json",
        &json!({
            "records_after": not_failure_sink.records().len(),
            "error": error_json(&not_failure_error),
        }),
    );
    write_json(
        &root,
        "guard-id-mismatch-error.json",
        &json!({
            "records_after": mismatch_sink.records().len(),
            "error": error_json(&mismatch_error),
        }),
    );
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "new_region_status": awaiting.status,
            "quarantine_status": quarantined.status,
            "reject_status": rejected.status,
            "reject_error_code": rejected_error.code(),
            "listed_count": listed.len(),
            "max_since_count": max_since.len(),
            "sink_new_region_error_code": sink_new_region_error.code(),
            "sink_reject_error_code": sink_reject_error.code(),
            "not_failure_error_code": not_failure_error.code(),
            "not_failure_records_after": not_failure_sink.records().len(),
            "guard_id_mismatch_error_code": mismatch_error.code(),
            "guard_id_mismatch_records_after": mismatch_sink.records().len(),
            "new_region_uuid": awaiting.novel_id.to_string(),
        }),
    );

    println!(
        "FSV_PH38_T03 statuses={:?}/{:?}/{:?} reject_code={} listed={} max_since={}",
        awaiting.status,
        quarantined.status,
        rejected.status,
        rejected_error.code(),
        listed.len(),
        max_since.len(),
    );
    println!("FSV_PH38_T03_UUID {}", awaiting.novel_id);
}

#[derive(Clone, Default)]
struct MemorySink {
    records: Arc<Mutex<Vec<NoveltyRecord>>>,
}

impl MemorySink {
    fn records(&self) -> Vec<NoveltyRecord> {
        self.records.lock().expect("records lock").clone()
    }
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
        Ok(self.records())
    }
}

#[derive(Clone)]
struct FailingSink;

impl VaultSink for FailingSink {
    fn write_novel(&self, _record: &NoveltyRecord) -> Result<(), WardError> {
        Err(WardError::NoveltySink {
            reason: "synthetic sink failure".to_string(),
        })
    }

    fn novel_records(&self) -> Result<Vec<NoveltyRecord>, WardError> {
        Ok(Vec::new())
    }
}

fn handle_action<S>(sink: &S, action: NoveltyAction) -> Result<NoveltyRecord, WardError>
where
    S: VaultSink + Clone + 'static,
{
    let (profile, produced, matched) = scenario(action);
    let verdict = guard(&profile, &produced, &matched, false).expect("failing verdict");
    handler_for(sink.clone()).handle(&profile, &verdict, &produced)
}

fn handler_for<S>(sink: S) -> NoveltyHandler
where
    S: VaultSink + 'static,
{
    NoveltyHandler::new(Arc::new(sink), Arc::new(FixedClock::new(FIXED_TS)))
}

fn scenario(action: NoveltyAction) -> (GuardProfile, ProducedSlots, MatchedSlots) {
    let profile = GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic-novelty".to_string(),
        tau: [(slot(1), 0.70)].into_iter().collect(),
        required_slots: vec![slot(1)],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: action,
    };
    let produced = BTreeMap::from([(slot(1), vec![1.0, 0.0])]);
    let matched = BTreeMap::from([(slot(1), vec![0.45, (1.0_f32 - 0.45 * 0.45).sqrt()])]);
    (profile, produced, matched)
}

fn passing_verdict_error(sink: &MemorySink) -> WardError {
    let (mut profile, produced, mut matched) = scenario(NoveltyAction::NewRegion);
    profile.tau.insert(slot(1), 0.70);
    matched.insert(slot(1), vec![1.0, 0.0]);
    let verdict = guard(&profile, &produced, &matched, false).expect("passing verdict");

    handler_for(sink.clone())
        .handle(&profile, &verdict, &produced)
        .expect_err("not a failure")
}

fn guard_id_mismatch_error(sink: &MemorySink) -> WardError {
    let (profile, produced, matched) = scenario(NoveltyAction::NewRegion);
    let mut verdict = guard(&profile, &produced, &matched, false).expect("failing verdict");
    verdict.guard_id = other_guard_id();

    handler_for(sink.clone())
        .handle(&profile, &verdict, &produced)
        .expect_err("guard id mismatch")
}

fn error_json(error: &WardError) -> serde_json::Value {
    json!({
        "code": error.code(),
        "message": error.to_string(),
    })
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
    "118f48a4-9a79-74d2-8a5c-9ad7f6b8c101"
        .parse()
        .expect("other guard id")
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
