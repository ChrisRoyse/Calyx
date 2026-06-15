use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use calyx_anneal::{
    CALYX_ASSAY_INVALID_METRIC, CALYX_REGISTRY_PROFILE_TIMEOUT, CandidateLens,
    DIFFERENTIATION_MAX_CORR, DIFFERENTIATION_MIN_BITS, DifferentiationGate, GateOutcome,
    LensProfiler, PairNMI, RejectReason,
};
use calyx_core::{CalyxError, Clock, Constellation, LensId, Result};
use calyx_registry::{
    CapabilityCard, CostMetrics, CoverageMetrics, LensHealth, MetricSource, SeparationMetrics,
    SpreadMetrics,
};
use proptest::prelude::*;

#[test]
fn low_bits_are_rejected() {
    let outcome = gate_with(0.04, &[(lens(1), 0.10)], 0).unwrap();

    assert_eq!(
        outcome,
        GateOutcome::Rejected {
            reason: RejectReason::InsufficientBits {
                bits: f64::from(0.04_f32),
                threshold: DIFFERENTIATION_MIN_BITS
            }
        }
    );
}

#[test]
fn high_correlation_is_rejected_with_offending_lens() {
    let offending = lens(8);
    let outcome = gate_with(0.10, &[(lens(7), 0.55), (offending, 0.65)], 0).unwrap();

    assert_eq!(
        outcome,
        GateOutcome::Rejected {
            reason: RejectReason::TooCorrelated {
                corr: f64::from(0.65_f32),
                offending_lens: offending,
                threshold: DIFFERENTIATION_MAX_CORR
            }
        }
    );
}

#[test]
fn sufficient_bits_and_low_corr_are_admitted() {
    let outcome = gate_with(0.10, &[(lens(3), 0.55)], 0).unwrap();

    assert_eq!(
        outcome,
        GateOutcome::Admitted {
            bits: f64::from(0.10_f32),
            max_corr: f64::from(0.55_f32)
        }
    );
}

#[test]
fn empty_panel_and_exact_boundary_are_admitted() {
    let outcome = gate_with(0.05, &[], 0).unwrap();

    assert_eq!(
        outcome,
        GateOutcome::Admitted {
            bits: f64::from(0.05_f32),
            max_corr: 0.0
        }
    );
}

#[test]
fn nonfinite_bits_fail_closed() {
    let error = gate_with(f32::NAN, &[(lens(4), 0.10)], 0).unwrap_err();

    assert_eq!(error.code, CALYX_ASSAY_INVALID_METRIC);
}

#[test]
fn profile_timeout_from_elapsed_clock_rejects() {
    let outcome = gate_with(0.20, &[(lens(5), 0.10)], 30_001).unwrap();

    assert_eq!(
        outcome,
        GateOutcome::Rejected {
            reason: RejectReason::ProfileTimeout
        }
    );
}

#[test]
fn profiler_timeout_error_rejects_without_panic() {
    let clock = StepClock::new(1_785_500_420);
    let profiler = StaticProfiler::timeout();
    let nmi = StaticNmi::from_pairs(&[(lens(6), 0.10)]);
    let gate = DifferentiationGate::new(&clock);
    let outcome = gate
        .gate(&candidate(), &[lens(6)], &profiler, &nmi, &[])
        .unwrap();

    assert_eq!(
        outcome,
        GateOutcome::Rejected {
            reason: RejectReason::ProfileTimeout
        }
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn valid_bits_and_corr_admit_iff_contract_passes(bits in 0.0f32..1.0, corr in 0.0f32..1.0) {
        let outcome = gate_with(bits, &[(lens(9), f64::from(corr))], 0).unwrap();
        let expected = f64::from(bits) >= DIFFERENTIATION_MIN_BITS
            && f64::from(corr) <= DIFFERENTIATION_MAX_CORR;

        prop_assert_eq!(matches!(outcome, GateOutcome::Admitted { .. }), expected);
    }
}

fn gate_with(bits: f32, correlations: &[(LensId, f64)], elapsed_ms: u64) -> Result<GateOutcome> {
    let clock = StepClock::new(1_785_500_420);
    let profiler = StaticProfiler::new(lens(200), bits, clock.inner(), elapsed_ms);
    let nmi = StaticNmi::from_pairs(correlations);
    let panel = correlations
        .iter()
        .map(|(lens_id, _)| *lens_id)
        .collect::<Vec<_>>();
    DifferentiationGate::new(&clock).gate(&candidate(), &panel, &profiler, &nmi, &[])
}

fn candidate() -> CandidateLens {
    CandidateLens::Commission {
        spec: calyx_anneal::CommissionSpec {
            target_modality: calyx_core::Modality::Audio,
            endpoint: None,
            model_id: None,
            axis: "speaker_identity".to_string(),
            suggested_targets: Vec::new(),
            description: "fixture candidate".to_string(),
        },
    }
}

fn lens(byte: u8) -> LensId {
    LensId::from_bytes([byte; 16])
}

#[derive(Clone)]
struct StepClock {
    now: Arc<AtomicU64>,
}

impl StepClock {
    fn new(ts: u64) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(ts)),
        }
    }

    fn inner(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.now)
    }
}

impl Clock for StepClock {
    fn now(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

struct StaticProfiler {
    lens_id: LensId,
    bits: f32,
    clock: Option<Arc<AtomicU64>>,
    elapsed_ms: u64,
    error: Option<CalyxError>,
}

impl StaticProfiler {
    fn new(lens_id: LensId, bits: f32, clock: Arc<AtomicU64>, elapsed_ms: u64) -> Self {
        Self {
            lens_id,
            bits,
            clock: Some(clock),
            elapsed_ms,
            error: None,
        }
    }

    fn timeout() -> Self {
        Self {
            lens_id: lens(201),
            bits: 0.0,
            clock: None,
            elapsed_ms: 0,
            error: Some(CalyxError {
                code: CALYX_REGISTRY_PROFILE_TIMEOUT,
                message: "fixture profile timeout".to_string(),
                remediation: "retry profile with budget or repair lens runtime",
            }),
        }
    }
}

impl LensProfiler for StaticProfiler {
    fn profile(
        &self,
        _candidate: &CandidateLens,
        corpus_sample: &[Constellation],
    ) -> Result<CapabilityCard> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if let Some(clock) = &self.clock {
            clock.fetch_add(self.elapsed_ms, Ordering::SeqCst);
        }
        Ok(card(self.lens_id, self.bits, corpus_sample.len()))
    }
}

struct StaticNmi {
    correlations: BTreeMap<LensId, f64>,
}

impl StaticNmi {
    fn from_pairs(pairs: &[(LensId, f64)]) -> Self {
        Self {
            correlations: pairs.iter().copied().collect(),
        }
    }
}

impl PairNMI for StaticNmi {
    fn lens_embeddings(
        &self,
        lens: &LensId,
        _corpus_sample: &[Constellation],
    ) -> Result<Vec<Vec<f32>>> {
        let corr = *self.correlations.get(lens).ok_or_else(|| CalyxError {
            code: CALYX_ASSAY_INVALID_METRIC,
            message: format!("missing fixture correlation for lens {lens}"),
            remediation: "repair differentiation gate fixture",
        })?;
        Ok(vec![vec![corr as f32]])
    }

    fn nmi(&self, _lens_a: &LensId, lens_b_embeddings: &[Vec<f32>]) -> Result<f64> {
        lens_b_embeddings
            .first()
            .and_then(|row| row.first())
            .copied()
            .map(f64::from)
            .ok_or_else(|| CalyxError {
                code: CALYX_ASSAY_INVALID_METRIC,
                message: "empty fixture NMI embeddings".to_string(),
                remediation: "repair differentiation gate fixture",
            })
    }
}

fn card(lens_id: LensId, bits: f32, probe_count: usize) -> CapabilityCard {
    CapabilityCard {
        lens_id,
        probe_count,
        signal: Some(bits),
        signal_source: MetricSource::AssayStore,
        proxy_signal: bits,
        differentiation: None,
        differentiation_source: MetricSource::AssayPending,
        proxy_differentiation: 0.0,
        spread: SpreadMetrics {
            participation_ratio: 1.0,
            normalized_participation_ratio: 1.0,
            stable_rank: 1.0,
            total_variance: 1.0,
            mean_pairwise_distance: 1.0,
        },
        separation: SeparationMetrics {
            score: bits,
            silhouette: bits,
            mean_pairwise_distance: 1.0,
            labeled_groups: 2,
            used_labels: true,
        },
        cost: CostMetrics {
            total_ms: 1.0,
            ms_per_input: 1.0,
            vram_bytes: 0,
            ram_bytes: 0,
            batch_ceiling: 1_000,
        },
        coverage: CoverageMetrics {
            requested: probe_count,
            measured: probe_count,
            failed: 0,
            rate: 1.0,
        },
        health: LensHealth::Loaded,
        low_spread: false,
    }
}
