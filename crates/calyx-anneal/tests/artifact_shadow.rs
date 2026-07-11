use std::sync::{Arc, Mutex};

use calyx_anneal::{
    ActionMetricSnapshot, AnnealLedgerAction, ArtifactKey, ArtifactPtr, ArtifactReplayMeasurer,
    CALYX_ANNEAL_SHADOW_MEASUREMENT_MISSING, ChangeOutcome, MetricSide, ReplayQuery,
    ShadowRevertReason, TripwireMetric,
};
use calyx_core::{CalyxError, FixedClock, Result};
use calyx_ledger::MemoryLedgerStore;

#[allow(dead_code)]
#[path = "support/fsv_bad_change.rs"]
mod support;
use support::{
    TEST_TS, artifact_key, budget_config, candidate_ptr, install_prior, memory_substrate, prior_ptr,
};

const CALYX_TEST_MEASUREMENT_FAILED: &str = "CALYX_TEST_MEASUREMENT_FAILED";

#[test]
fn artifact_gate_measures_candidate_and_incumbent_pointers_independently() {
    let clock = FixedClock::new(TEST_TS);
    let measurer = RecordingMeasurer::new(0.96, 0.95, None);
    let calls = measurer.calls.clone();
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default())
        .with_replay_measurer(Arc::new(measurer));
    install_prior(&substrate.rollback);

    let outcome = substrate
        .propose_artifact_change_with_description(
            artifact_key(),
            candidate_ptr(),
            "artifact-bound measurement",
        )
        .unwrap();

    assert!(matches!(outcome, ChangeOutcome::Promoted(_)));
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            (artifact_key(), candidate_ptr(), 1),
            (artifact_key(), prior_ptr(), 1),
        ]
    );
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(candidate_ptr())
    );
    let entry = substrate.status().unwrap().recent_changes.pop().unwrap();
    assert_eq!(entry.action, AnnealLedgerAction::Promote);
    let recall = entry
        .metrics
        .metrics
        .iter()
        .find(|metric| metric.metric == TripwireMetric::RecallAtK)
        .unwrap();
    assert_eq!(recall.candidate_value, 0.96);
    assert_eq!(recall.incumbent_value, 0.95);
}

#[test]
fn artifact_gate_reverts_measured_candidate_regression() {
    let clock = FixedClock::new(TEST_TS);
    let measurer = RecordingMeasurer::new(0.94, 0.95, None);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default())
        .with_replay_measurer(Arc::new(measurer));
    install_prior(&substrate.rollback);

    let outcome = substrate
        .propose_artifact_change_with_description(
            artifact_key(),
            candidate_ptr(),
            "measured candidate regression",
        )
        .unwrap();

    assert!(matches!(
        outcome,
        ChangeOutcome::Reverted {
            reason: ShadowRevertReason::MetricRegression(TripwireMetric::RecallAtK),
            ..
        }
    ));
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(prior_ptr())
    );
    let entry = substrate.status().unwrap().recent_changes.pop().unwrap();
    assert_eq!(entry.action, AnnealLedgerAction::Revert);
    let recall = entry
        .metrics
        .metrics
        .iter()
        .find(|metric| metric.metric == TripwireMetric::RecallAtK)
        .unwrap();
    assert_eq!(recall.candidate_value, 0.94);
    assert_eq!(recall.incumbent_value, 0.95);
}

#[test]
fn artifact_gate_without_measurer_reverts_and_preserves_incumbent() {
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    install_prior(&substrate.rollback);

    let outcome = substrate
        .propose_artifact_change_with_description(
            artifact_key(),
            candidate_ptr(),
            "missing measurement",
        )
        .unwrap();

    assert!(matches!(
        outcome,
        ChangeOutcome::Reverted {
            reason: ShadowRevertReason::MeasurementFailed {
                side: MetricSide::Candidate,
                ref code,
            },
            ..
        } if code == CALYX_ANNEAL_SHADOW_MEASUREMENT_MISSING
    ));
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(prior_ptr())
    );
    let entry = substrate.status().unwrap().recent_changes.pop().unwrap();
    assert_eq!(entry.action, AnnealLedgerAction::Revert);
    assert_eq!(entry.metrics.query_count, 0);
}

#[test]
fn incumbent_measurement_error_is_attributed_after_candidate_measurement() {
    let clock = FixedClock::new(TEST_TS);
    let measurer = RecordingMeasurer::new(0.96, 0.95, Some(prior_ptr()));
    let calls = measurer.calls.clone();
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default())
        .with_replay_measurer(Arc::new(measurer));
    install_prior(&substrate.rollback);

    let outcome = substrate
        .propose_artifact_change_with_description(
            artifact_key(),
            candidate_ptr(),
            "incumbent measurement error",
        )
        .unwrap();

    assert!(matches!(
        outcome,
        ChangeOutcome::Reverted {
            reason: ShadowRevertReason::MeasurementFailed {
                side: MetricSide::Incumbent,
                ref code,
            },
            ..
        } if code == CALYX_TEST_MEASUREMENT_FAILED
    ));
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(prior_ptr())
    );
}

#[derive(Clone)]
struct RecordingMeasurer {
    candidate_recall: f64,
    incumbent_recall: f64,
    fail_ptr: Option<ArtifactPtr>,
    calls: Arc<Mutex<Vec<(ArtifactKey, ArtifactPtr, u64)>>>,
}

impl RecordingMeasurer {
    fn new(candidate_recall: f64, incumbent_recall: f64, fail_ptr: Option<ArtifactPtr>) -> Self {
        Self {
            candidate_recall,
            incumbent_recall,
            fail_ptr,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ArtifactReplayMeasurer for RecordingMeasurer {
    fn measure(
        &self,
        key: &ArtifactKey,
        artifact: &ArtifactPtr,
        query: &ReplayQuery,
    ) -> Result<ActionMetricSnapshot> {
        self.calls
            .lock()
            .unwrap()
            .push((key.clone(), artifact.clone(), query.query_id));
        if self.fail_ptr.as_ref() == Some(artifact) {
            return Err(CalyxError {
                code: CALYX_TEST_MEASUREMENT_FAILED,
                message: "scripted artifact measurement failure".to_string(),
                remediation: "inspect the test measurement fixture",
            });
        }
        let recall = if *artifact == candidate_ptr() {
            self.candidate_recall
        } else {
            self.incumbent_recall
        };
        Ok(ActionMetricSnapshot::from_values([
            (TripwireMetric::RecallAtK, recall),
            (TripwireMetric::GuardFAR, 0.001),
            (TripwireMetric::GuardFRR, 0.001),
            (TripwireMetric::SearchP99, 50.0),
            (TripwireMetric::IngestP95, 80.0),
        ]))
    }
}
