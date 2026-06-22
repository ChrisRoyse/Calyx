use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    CALYX_ANNEAL_REGRESSION_NAN_PREDICTION, CALYX_ANNEAL_REGRESSION_RECURRED,
    CALYX_ANNEAL_REGRESSION_SOURCE_UNAVAILABLE, MistakeLog, MistakeRef, MistakeStorage,
    RegressionConfig, RegressionContextSource, RegressionPredictor, ReplayEntry,
    assert_no_regression, record_regression, regression_rate,
};
use calyx_core::{
    AnchorKind, CalyxError, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef,
    Modality, Result,
};
use proptest::prelude::*;

mod fsv_support;
use fsv_support::vault_id;

const TEST_TS: u64 = 1_785_500_410;

#[test]
fn improved_prediction_does_not_recur() {
    let (log, batch, contexts) = single_case(0.8, 0.0);

    let report = assert_no_regression(&FixedPredictor::new(0.1), &batch, &log, &contexts).unwrap();

    assert!(report.passed);
    assert_eq!(report.regression_count, 0);
    assert_eq!(report.results[0].old_surprise, 0.8);
    assert_eq!(report.results[0].new_surprise, 0.1);
    assert!(!report.results[0].recurred);
}

#[test]
fn still_wrong_prediction_recurs_and_can_be_relogged() {
    let (log, batch, contexts) = single_case(0.8, 0.0);
    let report = assert_no_regression(&FixedPredictor::new(0.9), &batch, &log, &contexts).unwrap();

    assert!(!report.passed);
    assert_eq!(regression_rate(&report).unwrap(), 1.0);
    assert!(RegressionConfig::strict().exceeds(&report).unwrap());
    let err = calyx_anneal::regression_recurred(&report);
    assert_eq!(err.code, CALYX_ANNEAL_REGRESSION_RECURRED);

    let relogged = record_regression(&report.results[0], &log)
        .unwrap()
        .expect("recurrence should be relogged");
    assert!(relogged.surprise > report.results[0].old_surprise);
    assert_eq!(log.readback_recent(2).unwrap().len(), 2);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn all_improvements_have_zero_regression_rate(
        old in 0.01_f64..10.0,
        factor in 0.0_f64..0.99,
    ) {
        let new_surprise = old * factor;
        let (log, batch, contexts) = single_case(old, 0.0);
        let report = assert_no_regression(
            &FixedPredictor::new(new_surprise),
            &batch,
            &log,
            &contexts,
        ).unwrap();

        prop_assert!(report.passed);
        prop_assert_eq!(regression_rate(&report).unwrap(), 0.0);
    }
}

#[test]
fn empty_all_recur_and_nan_edges_are_fail_closed() {
    let log = memory_log();
    let contexts = MemoryContexts::default();
    let empty = assert_no_regression(&FixedPredictor::new(0.0), &[], &log, &contexts).unwrap();
    assert!(empty.passed);
    assert_eq!(regression_rate(&empty).unwrap(), 0.0);

    let (log, batch, contexts) = multi_case(&[(0.1, 0.0), (0.2, 0.0), (0.3, 0.0)]);
    let all = assert_no_regression(&FixedPredictor::new(1.0), &batch, &log, &contexts).unwrap();
    assert_eq!(all.regression_count, 3);
    assert_eq!(regression_rate(&all).unwrap(), 1.0);

    let (log, batch, contexts) = single_case(0.8, 0.0);
    let nan = assert_no_regression(&NanPredictor, &batch, &log, &contexts).unwrap();
    assert_eq!(nan.regression_count, 1);
    assert_eq!(nan.results[0].new_surprise, f64::MAX);
    assert_eq!(
        nan.results[0].prediction_error.as_deref(),
        Some(CALYX_ANNEAL_REGRESSION_NAN_PREDICTION)
    );
}

#[test]
fn missing_context_fails_closed_with_exact_code() {
    let log = memory_log();
    let row = append(&log, 1, 0.0, 1.0);
    let batch = vec![replay(row, 1)];
    let err = assert_no_regression(
        &FixedPredictor::new(1.0),
        &batch,
        &log,
        &MemoryContexts::default(),
    )
    .unwrap_err();

    assert_eq!(err.code, CALYX_ANNEAL_REGRESSION_SOURCE_UNAVAILABLE);
}

#[derive(Default)]
struct MemoryContexts {
    rows: BTreeMap<CxId, Constellation>,
}

impl MemoryContexts {
    fn insert(&mut self, cx: Constellation) {
        self.rows.insert(cx.cx_id, cx);
    }
}

impl RegressionContextSource for MemoryContexts {
    fn regression_constellation(&self, cx_id: CxId) -> Result<Constellation> {
        self.rows.get(&cx_id).cloned().ok_or_else(|| CalyxError {
            code: "TEST_CONTEXT_MISS",
            message: format!("missing {cx_id}"),
            remediation: "insert the synthetic context",
        })
    }
}

struct FixedPredictor {
    value: f64,
}

impl FixedPredictor {
    const fn new(value: f64) -> Self {
        Self { value }
    }
}

impl RegressionPredictor for FixedPredictor {
    fn predict_regression(&self, _cx: &Constellation) -> f64 {
        self.value
    }
}

struct NanPredictor;

impl RegressionPredictor for NanPredictor {
    fn predict_regression(&self, _cx: &Constellation) -> f64 {
        f64::NAN
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

fn single_case(
    old_prediction: f64,
    observed: f64,
) -> (
    MistakeLog<MemoryMistakeStorage>,
    Vec<ReplayEntry>,
    MemoryContexts,
) {
    let log = memory_log();
    let row = append(&log, 1, old_prediction, observed);
    let mut contexts = MemoryContexts::default();
    contexts.insert(cx(1));
    (log, vec![replay(row, 1)], contexts)
}

fn multi_case(
    values: &[(f64, f64)],
) -> (
    MistakeLog<MemoryMistakeStorage>,
    Vec<ReplayEntry>,
    MemoryContexts,
) {
    let log = memory_log();
    let mut batch = Vec::with_capacity(values.len());
    let mut contexts = MemoryContexts::default();
    for (index, (old_prediction, observed)) in values.iter().copied().enumerate() {
        let seed = index as u8 + 1;
        let row = append(&log, seed, old_prediction, observed);
        batch.push(replay(row, seed));
        contexts.insert(cx(seed));
    }
    (log, batch, contexts)
}

fn memory_log() -> MistakeLog<MemoryMistakeStorage> {
    MistakeLog::open(
        MemoryMistakeStorage::default(),
        16,
        Arc::new(FixedClock::new(TEST_TS)),
    )
    .unwrap()
}

fn append(
    log: &MistakeLog<MemoryMistakeStorage>,
    seed: u8,
    predicted: f64,
    observed: f64,
) -> MistakeRef {
    log.append(cx_id(seed), predicted, observed, AnchorKind::Reward)
        .unwrap()
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
