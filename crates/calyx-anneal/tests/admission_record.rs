use std::collections::BTreeMap;

use calyx_anneal::{
    AdmissionRecord, AlgParams, AlgorithmicKind, AnnealLedger, AnnealLedgerAction,
    AnnealLedgerEntry, CALYX_LEDGER_WRITE_FAIL, CandidateLens, ChangeId, GateOutcome,
    LensAdmittedEntry, LensRejectedEntry, MetricSnapshot, ProposalOutcome, ProposalTerminalState,
    RejectReason, proposal_history, proposal_history_with_refs, record_admitted,
    record_proposal_outcome, record_rejected,
};
use calyx_core::{CalyxError, FixedClock, Result};
use calyx_ledger::{ActorId, LedgerAppender, LedgerCfStore, LedgerRow, MemoryLedgerStore};

const TEST_TS: u64 = 1_785_500_422;

#[test]
fn record_admitted_roundtrips_from_history() {
    let mut ledger = memory_ledger();
    let admitted = admitted_entry(ChangeId(422_001));

    let ledger_ref = record_admitted(&admitted, &mut ledger).expect("record admitted");
    let history = proposal_history_with_refs(&ledger, 1).expect("history");

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].ledger_ref, ledger_ref);
    assert_eq!(history[0].record, AdmissionRecord::LensAdmitted(admitted));
    assert_eq!(
        ledger.read_recent(1).unwrap()[0].action,
        AnnealLedgerAction::LensAdmitted
    );
}

#[test]
fn record_rejected_roundtrips_from_history() {
    let mut ledger = memory_ledger();
    let rejected = rejected_entry(0.02);

    record_rejected(&rejected, &mut ledger).expect("record rejected");
    let history = proposal_history(&ledger, 1).expect("history");

    assert_eq!(history, vec![AdmissionRecord::LensRejected(rejected)]);
    assert_eq!(
        ledger.read_recent(1).unwrap()[0].action,
        AnnealLedgerAction::LensRejected
    );
}

#[test]
fn no_sufficiency_gain_outcome_records_lens_rejected() {
    let mut ledger = memory_ledger();
    let outcome = ProposalOutcome {
        candidate: Some(candidate()),
        gate_outcome: Some(GateOutcome::Admitted {
            bits: 0.20,
            max_corr: 0.10,
            resource: None,
        }),
        sufficiency_before: 0.50,
        sufficiency_after: Some(0.50),
        admitted: false,
        change_id: Some(ChangeId(422_010)),
        hot_add: None,
        terminal_state: ProposalTerminalState::NoSufficiencyGain,
    };

    let ledger_ref = record_proposal_outcome(&outcome, &mut ledger, TEST_TS + 2, 0.40)
        .expect("record no-gain outcome")
        .expect("no-gain should write a rejection");
    let history = proposal_history_with_refs(&ledger, 1).expect("history");

    assert_eq!(history[0].ledger_ref, ledger_ref);
    match &history[0].record {
        AdmissionRecord::LensRejected(entry) => {
            assert_eq!(entry.deficit_gap, 0.40);
            assert_eq!(
                entry.reason,
                RejectReason::NoSufficiencyGain {
                    before: 0.50,
                    after: 0.50
                }
            );
        }
        other => panic!("expected LensRejected, got {other:?}"),
    }
}

#[test]
fn proposal_history_zero_empty_and_mixed_order() {
    let mut ledger = memory_ledger();
    ledger
        .write(non_proposal_entry(ChangeId(10)))
        .expect("write non proposal");
    record_admitted(&admitted_entry(ChangeId(11)), &mut ledger).unwrap();
    record_rejected(&rejected_entry(0.01), &mut ledger).unwrap();
    record_admitted(&admitted_entry(ChangeId(12)), &mut ledger).unwrap();

    assert!(proposal_history(&ledger, 0).unwrap().is_empty());
    let last_two = proposal_history(&ledger, 2).unwrap();

    assert!(matches!(last_two[0], AdmissionRecord::LensRejected(_)));
    assert!(matches!(last_two[1], AdmissionRecord::LensAdmitted(_)));
}

#[test]
fn ledger_write_failure_fails_closed_without_row() {
    let appender = LedgerAppender::open(FailingStore, FixedClock::new(TEST_TS)).unwrap();
    let mut ledger =
        AnnealLedger::new(appender, ActorId::Service("calyx-anneal-test".to_string())).unwrap();

    let error = record_admitted(&admitted_entry(ChangeId(99)), &mut ledger).unwrap_err();

    assert_eq!(error.code, CALYX_LEDGER_WRITE_FAIL);
    assert!(ledger.read_recent(10).unwrap().is_empty());
}

#[test]
fn invalid_admitted_metric_is_rejected_before_ledger_write() {
    let mut ledger = memory_ledger();
    let mut admitted = admitted_entry(ChangeId(422_009));
    admitted.sufficiency_after = admitted.sufficiency_before;

    let error = record_admitted(&admitted, &mut ledger).unwrap_err();

    assert_eq!(error.code, "CALYX_ASSAY_INVALID_METRIC");
    assert!(ledger.read_recent(10).unwrap().is_empty());
}

#[test]
fn missing_structured_payload_fails_history_decode() {
    let mut ledger = memory_ledger();
    ledger
        .write(AnnealLedgerEntry {
            action: AnnealLedgerAction::LensAdmitted,
            change_id: ChangeId(777),
            artifact_id: "missing-structured-payload".to_string(),
            prior_ptr_hash: [0; 32],
            candidate_ptr_hash: [1; 32],
            metrics: MetricSnapshot::empty(TEST_TS),
            ts: TEST_TS,
            description: "bad proposal row".to_string(),
            fault: None,
            proposal: None,
            details: None,
            prev_hash: None,
        })
        .expect("write malformed proposal");

    let error = proposal_history(&ledger, 1).unwrap_err();

    assert_eq!(error.code, "CALYX_ANNEAL_LEDGER_INVALID_ENTRY");
}

struct FailingStore;

impl LedgerCfStore for FailingStore {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        Ok(Vec::new())
    }

    fn put_new(&mut self, _seq: u64, _bytes: &[u8]) -> Result<()> {
        Err(CalyxError {
            code: "CALYX_ASTER_CF_UNAVAILABLE",
            message: "injected ledger CF outage".to_string(),
            remediation: "restore Aster ledger CF",
        })
    }
}

fn memory_ledger() -> AnnealLedger<MemoryLedgerStore, FixedClock> {
    let appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(TEST_TS)).unwrap();
    AnnealLedger::new(appender, ActorId::Service("calyx-anneal-test".to_string())).unwrap()
}

fn admitted_entry(change_id: ChangeId) -> LensAdmittedEntry {
    LensAdmittedEntry {
        candidate_desc: "Algorithmic PCA lens for anchor 'quality' over 1 corpus rows (seed 42)"
            .to_string(),
        bits_gain: 0.12,
        max_corr: 0.45,
        sufficiency_before: 0.20,
        sufficiency_after: 0.80,
        change_id,
        ts: TEST_TS,
    }
}

fn rejected_entry(bits: f64) -> LensRejectedEntry {
    LensRejectedEntry {
        candidate_desc: "Algorithmic PCA lens for anchor 'quality' over 1 corpus rows (seed 42)"
            .to_string(),
        reason: RejectReason::InsufficientBits {
            bits,
            threshold: 0.05,
        },
        deficit_gap: 0.80,
        ts: TEST_TS + 1,
    }
}

fn candidate() -> CandidateLens {
    CandidateLens::Algorithmic {
        kind: AlgorithmicKind::Pca,
        params: AlgParams {
            target_anchor: "quality".to_string(),
            sample_count: 64,
            seed: 42,
            features: BTreeMap::new(),
        },
    }
}

fn non_proposal_entry(change_id: ChangeId) -> AnnealLedgerEntry {
    AnnealLedgerEntry {
        action: AnnealLedgerAction::Promote,
        change_id,
        artifact_id: "non-proposal".to_string(),
        prior_ptr_hash: [1; 32],
        candidate_ptr_hash: [2; 32],
        metrics: MetricSnapshot::empty(TEST_TS),
        ts: TEST_TS,
        description: "non proposal event".to_string(),
        fault: None,
        proposal: None,
        details: None,
        prev_hash: None,
    }
}
