use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    AnchorGap, CALYX_ANNEAL_OPERATOR_NO_GAIN, CALYX_ASSAY_INVALID_METRIC, ChangeId, ChangeOutcome,
    DeficitMap, HeadKind, OperatorPromotionGate, OperatorProposalStorage, OperatorShadowProposal,
    OperatorTerminalState, ProposeOperator, ProposeOperatorRequest, ProposedOperator,
    ShadowRevertReason, decode_operator_proposal_rows,
};
use calyx_core::{FixedClock, Modality, Result};

const TEST_TS: u64 = 1_785_500_582;

#[test]
fn online_head_deficit_promotes_and_persists_record() {
    let storage = MemoryOperatorStorage::default();
    let mut gate = ScriptedGate::promote();
    let clock = FixedClock::new(TEST_TS);
    let proposer = ProposeOperator::new(&clock);

    let outcome = proposer
        .propose_operator(request(&storage, &mut gate, online_deficit(), 0.20))
        .unwrap();

    assert!(matches!(
        outcome.terminal_state,
        OperatorTerminalState::Promoted
    ));
    assert_eq!(gate.ensured.load(Ordering::SeqCst), 1);
    assert_eq!(gate.proposed.load(Ordering::SeqCst), 1);
    let rows = decode_operator_proposal_rows(storage.scan_operator_proposals().unwrap()).unwrap();
    assert_eq!(rows.len(), 1);
    let record = &rows[0].record;
    assert_eq!(record.deficit_total_bits, 0.80);
    assert_eq!(record.refit_delta_j, 0.20);
    assert!((record.shadow_delta_j - 0.60).abs() < 1e-12);
    assert_eq!(record.change_id, Some(ChangeId(582_000)));
    assert_eq!(
        record.operator,
        ProposedOperator::OnlineHead {
            kind: HeadKind::Predictor,
            param_count: 2
        }
    );
}

#[test]
fn kernel_scope_deficit_uses_measured_recall_delta() {
    let storage = MemoryOperatorStorage::default();
    let mut gate = ScriptedGate::promote();
    let clock = FixedClock::new(TEST_TS + 1);
    let proposer = ProposeOperator::new(&clock);
    let mut req = request(&storage, &mut gate, kernel_deficit(), 0.05);
    req.kernel_recall_before = Some(0.40);
    req.kernel_recall_after = Some(0.72);

    proposer.propose_operator(req).unwrap();

    let rows = decode_operator_proposal_rows(storage.scan_operator_proposals().unwrap()).unwrap();
    let ProposedOperator::KernelScope {
        kernel_recall_before,
        kernel_recall_after,
        scope_hash,
        ..
    } = &rows[0].record.operator
    else {
        panic!("expected kernel scope proposal");
    };
    assert_eq!(*kernel_recall_before, 0.40);
    assert_eq!(*kernel_recall_after, 0.72);
    assert_ne!(*scope_hash, [0; 32]);
    assert!((rows[0].record.shadow_delta_j - 0.32).abs() < 1e-12);
}

#[test]
fn reverted_shadow_records_rollback_terminal_state() {
    let storage = MemoryOperatorStorage::default();
    let mut gate = ScriptedGate::revert();
    let clock = FixedClock::new(TEST_TS + 2);
    let proposer = ProposeOperator::new(&clock);

    let outcome = proposer
        .propose_operator(request(&storage, &mut gate, online_deficit(), 0.10))
        .unwrap();

    assert!(matches!(
        outcome.terminal_state,
        OperatorTerminalState::RolledBack {
            reason: ShadowRevertReason::InsufficientReplay
        }
    ));
    let rows = decode_operator_proposal_rows(storage.scan_operator_proposals().unwrap()).unwrap();
    assert!(matches!(
        rows[0].record.terminal_state,
        OperatorTerminalState::RolledBack { .. }
    ));
}

#[test]
fn refit_closed_and_no_deficit_do_not_write_or_gate() {
    let storage = MemoryOperatorStorage::default();
    let mut gate = ScriptedGate::promote();
    let clock = FixedClock::new(TEST_TS + 3);
    let proposer = ProposeOperator::new(&clock);

    let closed = proposer
        .propose_operator(request(&storage, &mut gate, online_deficit(), 0.80))
        .unwrap();
    let no_deficit = proposer
        .propose_operator(request(&storage, &mut gate, no_deficit(), 0.0))
        .unwrap();

    assert!(matches!(
        closed.terminal_state,
        OperatorTerminalState::RefitClosed
    ));
    assert!(matches!(
        no_deficit.terminal_state,
        OperatorTerminalState::NoDeficit
    ));
    assert_eq!(gate.proposed.load(Ordering::SeqCst), 0);
    assert!(storage.scan_operator_proposals().unwrap().is_empty());
}

#[test]
fn no_gain_and_invalid_metric_fail_closed_without_rows() {
    let storage = MemoryOperatorStorage::default();
    let mut gate = ScriptedGate::promote();
    let clock = FixedClock::new(TEST_TS + 4);
    let proposer = ProposeOperator::new(&clock);
    let mut req = request(&storage, &mut gate, kernel_deficit(), 0.05);
    req.kernel_recall_before = Some(0.42);
    req.kernel_recall_after = Some(0.42);

    let no_gain = proposer.propose_operator(req).unwrap_err();
    let invalid = proposer
        .propose_operator(request(&storage, &mut gate, invalid_deficit(), 0.0))
        .unwrap_err();

    assert_eq!(no_gain.code, CALYX_ANNEAL_OPERATOR_NO_GAIN);
    assert_eq!(invalid.code, CALYX_ASSAY_INVALID_METRIC);
    assert!(storage.scan_operator_proposals().unwrap().is_empty());
    assert_eq!(gate.proposed.load(Ordering::SeqCst), 0);
}

fn request<'a>(
    storage: &'a MemoryOperatorStorage,
    gate: &'a mut ScriptedGate,
    deficit: DeficitMap,
    refit_delta_j: f64,
) -> ProposeOperatorRequest<'a> {
    ProposeOperatorRequest {
        deficit: Box::leak(Box::new(deficit)),
        refit_delta_j,
        storage,
        gate,
        kernel_recall_before: None,
        kernel_recall_after: None,
    }
}

fn online_deficit() -> DeficitMap {
    deficit(
        "oracle_prediction_quality",
        1.0,
        0.20,
        vec![Modality::Structured],
    )
}

fn kernel_deficit() -> DeficitMap {
    deficit("kernel_recall_window", 1.0, 0.35, Vec::new())
}

fn no_deficit() -> DeficitMap {
    deficit("oracle_prediction_quality", 1.0, 0.90, Vec::new())
}

fn invalid_deficit() -> DeficitMap {
    deficit("oracle_prediction_quality", f64::NAN, 0.0, Vec::new())
}

fn deficit(
    anchor: &str,
    entropy_h: f64,
    mutual_info_i: f64,
    modalities: Vec<Modality>,
) -> DeficitMap {
    let gap = (entropy_h - mutual_info_i).max(0.0);
    DeficitMap {
        computed_at: TEST_TS,
        top_gaps: vec![AnchorGap {
            anchor_class: anchor.to_string(),
            entropy_h,
            mutual_info_i,
            gap,
        }],
        underrepresented_modalities: modalities,
        total_bits_deficit: gap,
    }
}

#[derive(Clone, Default)]
struct MemoryOperatorStorage {
    rows: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl OperatorProposalStorage for MemoryOperatorStorage {
    fn save_operator_proposal(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        self.rows.lock().unwrap().insert(key, value);
        Ok(())
    }

    fn load_operator_proposal(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.rows.lock().unwrap().get(key).cloned())
    }

    fn scan_operator_proposals(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

struct ScriptedGate {
    mode: GateMode,
    next_id: AtomicU64,
    ensured: AtomicUsize,
    proposed: AtomicUsize,
}

impl ScriptedGate {
    fn promote() -> Self {
        Self {
            mode: GateMode::Promote,
            next_id: AtomicU64::new(582_000),
            ensured: AtomicUsize::new(0),
            proposed: AtomicUsize::new(0),
        }
    }

    fn revert() -> Self {
        Self {
            mode: GateMode::Revert,
            ..Self::promote()
        }
    }
}

impl OperatorPromotionGate for ScriptedGate {
    fn ensure_operator_prior(
        &mut self,
        _key: calyx_anneal::ArtifactKey,
        _ptr: calyx_anneal::ArtifactPtr,
    ) -> Result<()> {
        self.ensured.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn propose_operator_change(
        &mut self,
        _key: calyx_anneal::ArtifactKey,
        _candidate_ptr: calyx_anneal::ArtifactPtr,
        _candidate: &OperatorShadowProposal,
        _incumbent: &OperatorShadowProposal,
        _description: &str,
    ) -> Result<ChangeOutcome> {
        self.proposed.fetch_add(1, Ordering::SeqCst);
        let change_id = ChangeId(self.next_id.fetch_add(1, Ordering::SeqCst));
        Ok(match self.mode {
            GateMode::Promote => ChangeOutcome::Promoted(change_id),
            GateMode::Revert => ChangeOutcome::Reverted {
                reason: ShadowRevertReason::InsufficientReplay,
                change_id,
            },
        })
    }
}

enum GateMode {
    Promote,
    Revert,
}
