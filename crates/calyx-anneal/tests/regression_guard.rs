use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    CALYX_ANNEAL_REGRESSION_RECURRED, ChangeId, ChangeOutcome, HeadKind, HeadPromotionGate,
    HeadRegressionRollback, HeadShadowProposal, HeadStorage, MistakeLog, MistakeRef,
    MistakeStorage, OnlineHead, OnlineHeadState, RegressionConfig, RegressionContextSource,
    RegressionReport, ReplayEntry, ShadowRevertReason, decode_online_head,
};
use calyx_core::{
    AnchorKind, CalyxError, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef,
    Modality, Result,
};

mod fsv_support;
use fsv_support::vault_id;

const TEST_TS: u64 = 1_785_500_411;

#[test]
fn update_with_regression_promotes_after_no_recurrence_report() {
    let storage = MemoryHeadStorage::default();
    let mut state = OnlineHeadState::open(
        storage.clone(),
        ScriptedGate::promote(),
        Arc::new(FixedClock::new(TEST_TS)),
        [OnlineHead::new(HeadKind::Predictor, vec![0.0]).unwrap()],
    )
    .unwrap();
    let log = memory_log();
    let reference = log.append(cx_id(1), 0.0, 1.0, AnchorKind::Reward).unwrap();
    let batch = [replay(reference, 1)];
    let contexts = contexts([cx(1)]);

    let outcome = state
        .update_with_regression(
            &batch,
            &log,
            &contexts,
            1.0,
            0.0,
            RegressionConfig::strict(),
        )
        .unwrap();

    assert!(outcome.update.promoted);
    assert!(outcome.report.passed);
    assert_eq!(outcome.report.regression_count, 0);
    assert_eq!(state.head(HeadKind::Predictor).unwrap().params, vec![1.0]);
    let rows = storage.scan_heads().unwrap();
    assert_eq!(decode_online_head(&rows[0].1).unwrap().params, vec![1.0]);
}

#[test]
fn strict_regression_rolls_back_before_persisting_candidate_heads() {
    let storage = MemoryHeadStorage::default();
    let gate = ScriptedGate::promote();
    let rollback_count = gate.rollback_count.clone();
    let mut state = OnlineHeadState::open(
        storage.clone(),
        gate,
        Arc::new(FixedClock::new(TEST_TS)),
        [OnlineHead::new(HeadKind::Predictor, vec![1.0]).unwrap()],
    )
    .unwrap();
    let log = memory_log();
    let reference = log.append(cx_id(2), 1.1, 1.0, AnchorKind::Reward).unwrap();
    let batch = [replay(reference, 2)];
    let contexts = contexts([cx(2)]);

    let err = state
        .update_with_regression(
            &batch,
            &log,
            &contexts,
            1.0,
            0.0,
            RegressionConfig::strict(),
        )
        .unwrap_err();

    assert_eq!(err.code, CALYX_ANNEAL_REGRESSION_RECURRED);
    assert_eq!(rollback_count.load(Ordering::SeqCst), 1);
    assert_eq!(state.head(HeadKind::Predictor).unwrap().params, vec![1.0]);
    assert!(storage.scan_heads().unwrap().is_empty());
}

#[derive(Clone, Default)]
struct MemoryHeadStorage {
    rows: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl HeadStorage for MemoryHeadStorage {
    fn load_head(&self, kind: HeadKind) -> Result<Option<Vec<u8>>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(&calyx_anneal::head_key(kind))
            .cloned())
    }

    fn save_heads(&self, rows: Vec<(HeadKind, Vec<u8>)>) -> Result<()> {
        let mut inner = self.rows.lock().unwrap();
        for (kind, value) in rows {
            inner.insert(calyx_anneal::head_key(kind), value);
        }
        Ok(())
    }

    fn scan_heads(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

#[derive(Clone, Default)]
struct MemoryMistakeStorage {
    rows: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl MistakeStorage for MemoryMistakeStorage {
    fn put_new(&self, seq: u64, value: &[u8]) -> Result<()> {
        self.rows
            .lock()
            .unwrap()
            .insert(calyx_anneal::mistake_key(seq), value.to_vec());
        Ok(())
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

#[derive(Default)]
struct MemoryContexts {
    rows: BTreeMap<CxId, Constellation>,
}

impl RegressionContextSource for MemoryContexts {
    fn regression_constellation(&self, cx_id: CxId) -> Result<Constellation> {
        self.rows.get(&cx_id).cloned().ok_or_else(|| CalyxError {
            code: "TEST_CONTEXT_MISS",
            message: format!("missing {cx_id}"),
            remediation: "insert context",
        })
    }
}

#[derive(Clone)]
struct ScriptedGate {
    next_id: Arc<AtomicU64>,
    revert: Arc<AtomicBool>,
    rollback_count: Arc<AtomicUsize>,
}

impl ScriptedGate {
    fn promote() -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(410_000)),
            revert: Arc::new(AtomicBool::new(false)),
            rollback_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl HeadPromotionGate for ScriptedGate {
    fn ensure_head_prior(
        &mut self,
        _key: calyx_anneal::ArtifactKey,
        _ptr: calyx_anneal::ArtifactPtr,
    ) -> Result<()> {
        Ok(())
    }

    fn propose_head_change(
        &mut self,
        _key: calyx_anneal::ArtifactKey,
        _candidate_ptr: calyx_anneal::ArtifactPtr,
        _candidate: &HeadShadowProposal,
        _incumbent: &HeadShadowProposal,
        _description: &str,
    ) -> Result<ChangeOutcome> {
        if self.revert.load(Ordering::SeqCst) {
            return Ok(ChangeOutcome::Reverted {
                reason: ShadowRevertReason::InsufficientReplay,
                change_id: ChangeId(410_999),
            });
        }
        Ok(ChangeOutcome::Promoted(ChangeId(
            self.next_id.fetch_add(1, Ordering::SeqCst),
        )))
    }
}

impl HeadRegressionRollback for ScriptedGate {
    fn rollback_regressed_head_update(
        &mut self,
        _change_id: ChangeId,
        _report: &RegressionReport,
    ) -> Result<()> {
        self.rollback_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn memory_log() -> MistakeLog<MemoryMistakeStorage> {
    MistakeLog::open(
        MemoryMistakeStorage::default(),
        16,
        Arc::new(FixedClock::new(TEST_TS)),
    )
    .unwrap()
}

fn contexts(items: impl IntoIterator<Item = Constellation>) -> MemoryContexts {
    let mut contexts = MemoryContexts::default();
    for cx in items {
        contexts.rows.insert(cx.cx_id, cx);
    }
    contexts
}

fn replay(reference: MistakeRef, seed: u8) -> ReplayEntry {
    ReplayEntry::new(cx_id(seed), reference.surprise, reference, TEST_TS).unwrap()
}

fn cx_id(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn cx(seed: u8) -> Constellation {
    Constellation {
        cx_id: cx_id(seed),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: TEST_TS,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}
