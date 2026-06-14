use calyx_anneal::{
    AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry, CALYX_ASTER_CF_UNAVAILABLE,
    CALYX_LEDGER_ENTRY_TOO_LARGE, ChangeId, MetricComparison, MetricSnapshot, TripwireMetric,
};
use calyx_core::{CalyxError, FixedClock, Result};
use calyx_ledger::{ActorId, LedgerAppender, LedgerCfStore, LedgerRow, MemoryLedgerStore};
use proptest::prelude::*;

const TEST_TS: u64 = 1_785_500_398;

#[test]
fn promote_entry_roundtrips_from_ledger_payload() {
    let mut ledger = memory_ledger();
    let entry = sample_entry(ChangeId(101), AnnealLedgerAction::Promote, Some([0; 32]));

    let reference = ledger.write(entry.clone()).expect("write promote");
    let readback = ledger.read_recent_with_refs(1).expect("read recent");

    assert_eq!(readback.len(), 1);
    assert_eq!(readback[0].ledger_ref, reference);
    assert_eq!(readback[0].entry, entry);
}

#[test]
fn promote_revert_read_in_order_and_find_by_change_id() {
    let mut ledger = memory_ledger();
    let promote = sample_entry(ChangeId(201), AnnealLedgerAction::Promote, Some([0; 32]));
    let revert = sample_entry(ChangeId(202), AnnealLedgerAction::Revert, None);

    let first_ref = ledger.write(promote.clone()).expect("write promote");
    let mut expected_revert = revert.clone();
    expected_revert.prev_hash = Some(first_ref.hash);
    let second_ref = ledger.write(revert).expect("write revert");

    let recent = ledger.read_recent_with_refs(2).expect("read recent");
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].entry, promote);
    assert_eq!(recent[1].entry, expected_revert);
    assert!(recent[0].ledger_ref.seq < recent[1].ledger_ref.seq);
    assert_eq!(
        ledger.find_by_change_id(ChangeId(201)).unwrap(),
        Some(recent[0].entry.clone())
    );
    assert_eq!(
        ledger.find_by_change_id_with_ref(ChangeId(202)).unwrap(),
        Some(recent[1].clone())
    );
    assert_eq!(recent[1].ledger_ref, second_ref);
}

#[test]
fn repeated_change_lookup_returns_latest_event() {
    let mut ledger = memory_ledger();
    let change_id = ChangeId(303);
    ledger
        .write(sample_entry(
            change_id,
            AnnealLedgerAction::Promote,
            Some([0; 32]),
        ))
        .expect("write promote");
    ledger
        .write(sample_entry(change_id, AnnealLedgerAction::Revert, None))
        .expect("write revert");

    let found = ledger
        .find_by_change_id(change_id)
        .expect("lookup")
        .expect("entry");

    assert_eq!(found.action, AnnealLedgerAction::Revert);
}

#[test]
fn empty_description_and_empty_read_are_allowed() {
    let mut ledger = memory_ledger();
    assert_eq!(ledger.read_recent(10).unwrap(), Vec::new());

    let mut entry = sample_entry(ChangeId(404), AnnealLedgerAction::Park, Some([0; 32]));
    entry.description.clear();

    ledger
        .write(entry.clone())
        .expect("write empty description");

    assert_eq!(ledger.read_recent(1).unwrap(), vec![entry]);
}

#[test]
fn cf_unavailable_error_propagates() {
    let appender = LedgerAppender::open(FailingStore, FixedClock::new(TEST_TS)).unwrap();
    let mut ledger =
        AnnealLedger::new(appender, ActorId::Service("calyx-anneal-test".to_string())).unwrap();

    let error = ledger
        .write(sample_entry(
            ChangeId(505),
            AnnealLedgerAction::Propose,
            Some([0; 32]),
        ))
        .unwrap_err();

    assert_eq!(error.code, CALYX_ASTER_CF_UNAVAILABLE);
}

#[test]
fn oversized_payload_fails_closed_without_truncation() {
    let mut ledger = memory_ledger();
    let mut entry = sample_entry(
        ChangeId(606),
        AnnealLedgerAction::Recalibrate,
        Some([0; 32]),
    );
    entry.description = "oversized ".repeat(4096);

    let error = ledger.write(entry).unwrap_err();

    assert_eq!(error.code, CALYX_LEDGER_ENTRY_TOO_LARGE);
    assert_eq!(ledger.read_recent(10).unwrap(), Vec::new());
}

#[test]
fn mismatched_prev_hash_fails_closed() {
    let mut ledger = memory_ledger();
    let error = ledger
        .write(sample_entry(
            ChangeId(707),
            AnnealLedgerAction::MistakeUpdate,
            Some([9; 32]),
        ))
        .unwrap_err();

    assert_eq!(error.code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert_eq!(ledger.read_recent(10).unwrap(), Vec::new());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn written_entries_remain_in_insertion_order(actions in prop::collection::vec(action_strategy(), 1..32)) {
        let mut ledger = memory_ledger();
        let mut expected_ids = Vec::new();

        for (index, action) in actions.into_iter().enumerate() {
            let change_id = ChangeId(10_000 + index as u64);
            ledger.write(sample_entry(change_id, action, None)).unwrap();
            expected_ids.push(change_id);
        }

        let readback = ledger.read_recent_with_refs(usize::MAX).unwrap();
        let actual_ids = readback
            .iter()
            .map(|entry| entry.entry.change_id)
            .collect::<Vec<_>>();
        prop_assert_eq!(actual_ids, expected_ids);
        prop_assert!(
            readback
                .windows(2)
                .all(|pair| pair[0].ledger_ref.seq < pair[1].ledger_ref.seq)
        );
    }
}

struct FailingStore;

impl LedgerCfStore for FailingStore {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        Ok(Vec::new())
    }

    fn put_new(&mut self, _seq: u64, _bytes: &[u8]) -> Result<()> {
        Err(CalyxError {
            code: CALYX_ASTER_CF_UNAVAILABLE,
            message: "injected ledger CF outage".to_string(),
            remediation: "restore Aster ledger CF availability",
        })
    }
}

fn memory_ledger() -> AnnealLedger<MemoryLedgerStore, FixedClock> {
    let appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(TEST_TS)).unwrap();
    AnnealLedger::new(appender, ActorId::Service("calyx-anneal-test".to_string())).unwrap()
}

fn sample_entry(
    change_id: ChangeId,
    action: AnnealLedgerAction,
    prev_hash: Option<[u8; 32]>,
) -> AnnealLedgerEntry {
    AnnealLedgerEntry {
        action,
        change_id,
        artifact_id: format!("artifact-{}", change_id.0),
        prior_ptr_hash: [1; 32],
        candidate_ptr_hash: [2; 32],
        metrics: MetricSnapshot {
            evaluated_at: TEST_TS,
            query_count: 8,
            metrics: vec![MetricComparison {
                metric: TripwireMetric::RecallAtK,
                candidate_value: 0.91,
                incumbent_value: 0.89,
            }],
        },
        ts: TEST_TS,
        description: "synthetic anneal ledger event".to_string(),
        fault: None,
        proposal: None,
        details: None,
        prev_hash,
    }
}

fn action_strategy() -> impl Strategy<Value = AnnealLedgerAction> {
    prop_oneof![
        Just(AnnealLedgerAction::Promote),
        Just(AnnealLedgerAction::Revert),
        Just(AnnealLedgerAction::Propose),
        Just(AnnealLedgerAction::LensAdmitted),
        Just(AnnealLedgerAction::LensRejected),
        Just(AnnealLedgerAction::Park),
        Just(AnnealLedgerAction::DegradeChange),
        Just(AnnealLedgerAction::FaultEvent),
        Just(AnnealLedgerAction::Rebuild),
        Just(AnnealLedgerAction::BaseCorruptAlert),
        Just(AnnealLedgerAction::BaseRestored),
        Just(AnnealLedgerAction::Recalibrate),
        Just(AnnealLedgerAction::TauRecalibrated),
        Just(AnnealLedgerAction::TauRecalibrationReverted),
        Just(AnnealLedgerAction::LensPark),
        Just(AnnealLedgerAction::LensUnpark),
        Just(AnnealLedgerAction::MistakeUpdate),
        Just(AnnealLedgerAction::HeadUpdate),
        Just(AnnealLedgerAction::HeadUpdateReverted),
        Just(AnnealLedgerAction::OperatorPromoted),
        Just(AnnealLedgerAction::OperatorReverted),
        Just(AnnealLedgerAction::SleepPassDeferred),
        Just(AnnealLedgerAction::OutcomeReward),
        Just(AnnealLedgerAction::OutcomeContradiction),
        Just(AnnealLedgerAction::AutotuneAB),
        Just(AnnealLedgerAction::AutotunePromote),
        Just(AnnealLedgerAction::GoodhartPassed),
        Just(AnnealLedgerAction::GoodhartFailed),
    ]
}
