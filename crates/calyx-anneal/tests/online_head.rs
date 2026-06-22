use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    CALYX_ANNEAL_HEAD_TOO_LARGE, CALYX_ANNEAL_HEAD_UPDATE_REVERTED, ChangeOutcome, FrozenLensCheck,
    HeadKind, HeadPromotionGate, HeadShadowProposal, HeadStorage, MistakeRef, OnlineHead,
    OnlineHeadState, ReplayEntry, ShadowRevertReason, decode_online_head,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, FixedClock, Result};
use proptest::prelude::*;

mod fsv_support;
use fsv_support::vault_id;

const TEST_TS: u64 = 1_785_500_408;

#[test]
fn first_predictor_update_has_exact_delta_and_persists() {
    let storage = MemoryHeadStorage::default();
    let mut state = state_with_heads(storage.clone(), ScriptedGate::promote(), [zero_predictor()]);
    let outcome = state.update(&[entry(1.0, 1)], 0.01, 0.0).unwrap();

    let head = state.head(HeadKind::Predictor).unwrap();
    assert!(outcome.promoted);
    assert_eq!(head.version, 1);
    assert!((head.params[0] - 0.01).abs() < 1e-7);
    assert!((head.fisher_diag[0] - 1.0).abs() < 1e-7);

    let rows = storage.scan_heads().unwrap();
    assert_eq!(rows.len(), 1);
    let persisted = decode_online_head(&rows[0].1).unwrap();
    assert_eq!(persisted.version, 1);
    assert_eq!(persisted.params, head.params);
}

#[test]
fn ewc_regularizer_bounds_task_a_loss_increase() {
    let mut state = state_with_heads(
        MemoryHeadStorage::default(),
        ScriptedGate::promote(),
        [zero_predictor()],
    );
    state.update(&[entry(1.0, 1)], 0.1, 0.0).unwrap();
    let task_a_param = state.head(HeadKind::Predictor).unwrap().params[0];
    let task_a_loss = squared_loss(task_a_param, 1.0);

    state.update(&[entry(0.0, 2)], 0.001, 5.0).unwrap();

    let head = state.head(HeadKind::Predictor).unwrap();
    let loss_increase = squared_loss(head.params[0], 1.0) - task_a_loss;
    let ewc_bound = 5.0 * head.fisher_diag[0] * head.params[0].powi(2);
    assert!(loss_increase <= ewc_bound + 1e-6);
    assert!(head.params[0] > 0.09);
    assert!(head.params[0] < task_a_param);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn param_len_remains_constant(surprises in prop::collection::vec(0.0_f64..2.0, 1..24)) {
        let mut state = state_with_heads(
            MemoryHeadStorage::default(),
            ScriptedGate::promote(),
            [OnlineHead::new(HeadKind::Predictor, vec![0.0, 0.0, 0.0]).unwrap()],
        );
        for (index, surprise) in surprises.into_iter().enumerate() {
            state.update(&[entry(surprise, index as u64 + 1)], 0.005, 0.5).unwrap();
            prop_assert_eq!(state.head(HeadKind::Predictor).unwrap().params.len(), 3);
            prop_assert_eq!(state.head(HeadKind::Predictor).unwrap().fisher_diag.len(), 3);
        }
    }
}

#[test]
fn empty_zero_lr_and_too_large_edges_are_fail_closed() {
    let storage = MemoryHeadStorage::default();
    let mut state = state_with_heads(storage.clone(), ScriptedGate::promote(), [zero_predictor()]);

    state.update(&[], 0.01, 1.0).unwrap();
    assert_eq!(state.head(HeadKind::Predictor).unwrap().params, vec![0.0]);
    assert!(storage.scan_heads().unwrap().is_empty());

    state.update(&[entry(1.0, 1)], 0.0, 1.0).unwrap();
    assert_eq!(state.head(HeadKind::Predictor).unwrap().version, 0);
    assert!(storage.scan_heads().unwrap().is_empty());

    let err = OnlineHead::new(HeadKind::Predictor, vec![0.0; 1025]).unwrap_err();
    assert_eq!(err.code, CALYX_ANNEAL_HEAD_TOO_LARGE);
}

#[test]
fn reverted_substrate_does_not_update_params_version_or_storage() {
    let storage = MemoryHeadStorage::default();
    let mut state = state_with_heads(storage.clone(), ScriptedGate::revert(), [zero_predictor()]);

    let err = state.update(&[entry(1.0, 1)], 0.01, 0.0).unwrap_err();

    assert_eq!(err.code, CALYX_ANNEAL_HEAD_UPDATE_REVERTED);
    let head = state.head(HeadKind::Predictor).unwrap();
    assert_eq!(head.version, 0);
    assert_eq!(head.params, vec![0.0]);
    assert!(storage.scan_heads().unwrap().is_empty());
}

#[test]
fn aster_storage_writes_cbor_head_row_to_anneal_heads_cf() {
    let root = TestRoot::new("aster-head");
    let vault = AsterVault::new_durable(
        root.path(),
        vault_id(),
        b"issue408-head-storage".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let storage = calyx_anneal::AsterHeadStorage::new(&vault);
    let mut state = state_with_heads(storage, ScriptedGate::promote(), [zero_predictor()]);

    state.update(&[entry(1.0, 1)], 0.01, 0.0).unwrap();
    vault.flush().unwrap();

    let rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealHeads)
        .unwrap();
    assert_eq!(rows.len(), 1);
    let head = decode_online_head(&rows[0].1).unwrap();
    assert_eq!(head.kind, HeadKind::Predictor);
    assert_eq!(head.version, 1);
    assert!(head.param_norm() > 0.0);
}

#[test]
fn online_head_update_checks_frozen_guard_before_and_after() {
    let guard = CountingFrozenGuard::default();
    let calls = guard.calls.clone();
    let mut state = OnlineHeadState::open_with_guard(
        MemoryHeadStorage::default(),
        ScriptedGate::promote(),
        Arc::new(FixedClock::new(TEST_TS)),
        [zero_predictor()],
        guard,
    )
    .unwrap();

    state.update(&[entry(1.0, 1)], 0.01, 0.0).unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

fn state_with_heads<I, S, G>(storage: S, gate: G, heads: I) -> OnlineHeadState<S, G>
where
    I: IntoIterator<Item = OnlineHead>,
    S: HeadStorage,
    G: HeadPromotionGate,
{
    OnlineHeadState::open(storage, gate, Arc::new(FixedClock::new(TEST_TS)), heads).unwrap()
}

fn zero_predictor() -> OnlineHead {
    OnlineHead::new(HeadKind::Predictor, vec![0.0]).unwrap()
}

fn entry(surprise: f64, seq: u64) -> ReplayEntry {
    ReplayEntry::new(
        CxId::from_bytes([seq as u8; 16]),
        surprise,
        MistakeRef { seq, surprise },
        TEST_TS,
    )
    .unwrap()
}

fn squared_loss(param: f32, target: f32) -> f32 {
    0.5 * (param - target).powi(2)
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
struct CountingFrozenGuard {
    calls: Arc<AtomicUsize>,
}

impl FrozenLensCheck for CountingFrozenGuard {
    fn assert_no_violation(&self) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Clone)]
struct ScriptedGate {
    next_id: Arc<AtomicU64>,
    revert: Arc<AtomicBool>,
    ensured: Arc<AtomicUsize>,
}

impl ScriptedGate {
    fn promote() -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(408_000)),
            revert: Arc::new(AtomicBool::new(false)),
            ensured: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn revert() -> Self {
        let gate = Self::promote();
        gate.revert.store(true, Ordering::SeqCst);
        gate
    }
}

impl HeadPromotionGate for ScriptedGate {
    fn ensure_head_prior(
        &mut self,
        _key: calyx_anneal::ArtifactKey,
        _ptr: calyx_anneal::ArtifactPtr,
    ) -> Result<()> {
        self.ensured.fetch_add(1, Ordering::SeqCst);
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
                change_id: calyx_anneal::ChangeId(408_999),
            });
        }
        Ok(ChangeOutcome::Promoted(calyx_anneal::ChangeId(
            self.next_id.fetch_add(1, Ordering::SeqCst),
        )))
    }
}

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("calyx-online-head-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
