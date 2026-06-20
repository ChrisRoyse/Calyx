use calyx_assay::{PanelSufficiency, TrustTag};
use calyx_core::{CalyxError, Clock, CxId, FixedClock, Panel};
use calyx_lodestar::{LodestarError, RecallReport};
use proptest::prelude::*;

use super::*;
use crate::honesty_gate::SufficiencyAssay;
use crate::super_intel::{KernelRecallSource, OracleConsistencySource};
use crate::{OracleSelfConsistency, ShortCircuit};

#[test]
fn tier4_calibrated_passes_when_error_is_below_oracle_ceiling() {
    let tier = measure_tier_calibrated(
        &CalFixture::ok(0.15),
        0.73,
        &domain(),
        &held_out(),
        &clock(),
    )
    .expect("tier 4");

    assert!(tier.passed);
    assert_eq!(tier.tier, Tier::Calibrated);
    assert_close(tier.measured_value, 0.15);
    assert_close(tier.threshold, 0.73);
}

#[test]
fn tier4_calibrated_fails_when_error_exceeds_oracle_ceiling() {
    let tier =
        measure_tier_calibrated(&CalFixture::ok(0.8), 0.73, &domain(), &held_out(), &clock())
            .expect("tier 4");

    assert!(!tier.passed);
    assert_eq!(tier.cheapest_fix.as_deref(), Some(CALIBRATION_FIX));
}

#[test]
fn tier5_goodhart_passes_at_or_above_threshold_and_fails_below() {
    let pass = measure_tier_goodhart_defended(
        &GoodFixture::ok(0.95, true, 0),
        &domain(),
        &held_out(),
        &clock(),
    )
    .expect("tier 5 pass");
    let fail = measure_tier_goodhart_defended(
        &GoodFixture::ok(0.85, true, 0),
        &domain(),
        &held_out(),
        &clock(),
    )
    .expect("tier 5 fail");

    assert!(pass.passed);
    assert!(!fail.passed);
    assert_eq!(fail.cheapest_fix.as_deref(), Some(GOODHART_FIX));
}

#[test]
fn tier5_goodhart_source_failure_returns_failed_tier() {
    let tier = measure_tier_goodhart_defended(
        &GoodFixture(Err(synthetic_error("CALYX_SYNTHETIC_GOODHART_DOWN"))),
        &domain(),
        &held_out(),
        &clock(),
    )
    .expect("tier 5 source failures are tier results");

    assert!(!tier.passed);
    assert_eq!(tier.tier, Tier::GoodhartDefended);
    assert!(
        tier.cheapest_fix
            .as_deref()
            .expect("goodhart source fix")
            .contains("CALYX_SYNTHETIC_GOODHART_DOWN")
    );
}

#[test]
fn tier6_mistake_closed_requires_zero_recurring_mistakes() {
    let pass = measure_tier_mistake_closed(&MistakeFixture::ok(0, 3), &domain(), &clock()).unwrap();
    let fail = measure_tier_mistake_closed(&MistakeFixture::ok(1, 3), &domain(), &clock()).unwrap();

    assert!(pass.passed);
    assert!(!fail.passed);
    assert_eq!(fail.cheapest_fix.as_deref(), Some(MISTAKE_FIX));
}

#[test]
fn all_six_tiers_pass_produces_overall_true_report() {
    let report = measure_report(FixtureSet::passing(held_out())).expect("report");

    assert!(report.overall);
    assert_eq!(report.failing_tier, None);
    assert_eq!(report.cheapest_fix, None);
    assert_eq!(report.tiers.len(), 6);
}

#[test]
fn tier3_failure_is_first_but_tiers_4_to_6_are_still_measured() {
    let mut fixtures = FixtureSet::passing(held_out());
    fixtures.kernel = KernelFixture::ok(0.5, 2);

    let report = measure_report(fixtures).expect("report");

    assert!(!report.overall);
    assert_eq!(report.failing_tier, Some(Tier::KernelExists));
    assert_eq!(report.tiers.len(), 6);
    assert_eq!(
        tier(&report, Tier::Calibrated).map(|tier| tier.passed),
        Some(true)
    );
    assert_eq!(
        tier(&report, Tier::GoodhartDefended).map(|tier| tier.passed),
        Some(true)
    );
    assert_eq!(
        tier(&report, Tier::MistakeClosed).map(|tier| tier.passed),
        Some(true)
    );
}

#[test]
fn empty_held_out_split_fails_tiers_4_and_5_with_label_fix() {
    let report = measure_report(FixtureSet::passing(empty_held_out())).expect("report");

    assert_eq!(
        tier(&report, Tier::Calibrated).and_then(|tier| tier.cheapest_fix.as_deref()),
        Some(LABEL_HELD_OUT_ORACLE_FIX)
    );
    assert_eq!(
        tier(&report, Tier::GoodhartDefended).and_then(|tier| tier.cheapest_fix.as_deref()),
        Some(LABEL_HELD_OUT_ORACLE_FIX)
    );
    assert!(!tier(&report, Tier::Calibrated).unwrap().passed);
    assert!(!tier(&report, Tier::GoodhartDefended).unwrap().passed);
}

#[test]
fn domain_not_found_propagates_as_oracle_error() {
    let mut fixtures = FixtureSet::passing(held_out());
    fixtures.oracle = OracleFixture(Err(OracleError::DomainNotFound));

    let error = measure_report(fixtures).expect_err("domain not found");

    assert_eq!(error, OracleError::DomainNotFound);
}

#[test]
fn first_failure_is_mistake_closed_when_only_tier6_fails() {
    let mut fixtures = FixtureSet::passing(held_out());
    fixtures.mistakes = MistakeFixture::ok(1, 4);

    let report = measure_report(fixtures).expect("report");

    assert!(!report.overall);
    assert_eq!(report.failing_tier, Some(Tier::MistakeClosed));
    assert_eq!(report.cheapest_fix.as_deref(), Some(MISTAKE_FIX));
}

#[test]
fn calibration_source_failure_propagates_without_silent_pass() {
    let mut fixtures = FixtureSet::passing(held_out());
    fixtures.calibration = CalFixture(Err(synthetic_error("CALYX_SYNTHETIC_CALIBRATION_DOWN")));

    let error = measure_report(fixtures).expect_err("calibration failure");

    assert_eq!(error.code(), "CALYX_SYNTHETIC_CALIBRATION_DOWN");
}

#[test]
fn kernel_source_failure_propagates_without_silent_pass() {
    let mut fixtures = FixtureSet::passing(held_out());
    fixtures.kernel = KernelFixture(Err(LodestarError::KernelIndexIo {
        detail: "synthetic index read failure".to_string(),
    }));

    let error = measure_report(fixtures).expect_err("kernel failure");

    assert_eq!(error.code(), "CALYX_KERNEL_INDEX_IO");
}

proptest! {
    #[test]
    fn six_tier_overall_matches_all_tier_results(mask in 0u8..64) {
        let tiers = Tier::predicate_order()
            .iter()
            .enumerate()
            .map(|(index, tier)| {
                let passed = (mask & (1 << index)) != 0;
                let measured = if passed { 1.0 } else { 0.0 };
                TierResult::new(*tier, passed, measured, 0.5, (!passed).then(|| format!("fix {tier}")))
            })
            .collect::<Vec<_>>();
        let report = SuperIntelReport::new(domain(), tiers.clone());

        prop_assert_eq!(report.overall, tiers.iter().all(|tier| tier.passed));
    }
}

struct FixtureSet {
    oracle: OracleFixture,
    assay: AssayFixture,
    kernel: KernelFixture,
    calibration: CalFixture,
    goodhart: GoodFixture,
    mistakes: MistakeFixture,
    held_out: HeldOutSplit,
}

impl FixtureSet {
    fn passing(held_out: HeldOutSplit) -> Self {
        Self {
            oracle: OracleFixture(Ok(OracleSelfConsistency::with_provenance(
                0.0, 0.73, false, None,
            ))),
            assay: AssayFixture(Ok(panel_sufficiency(1.0, 1.0))),
            kernel: KernelFixture::ok(1.0, held_out.held_out_count().max(1)),
            calibration: CalFixture::ok(0.15),
            goodhart: GoodFixture::ok(0.95, true, 0),
            mistakes: MistakeFixture::ok(0, 2),
            held_out,
        }
    }
}

#[derive(Clone, Debug)]
struct OracleFixture(Result<OracleSelfConsistency, OracleError>);

impl OracleConsistencySource for OracleFixture {
    fn oracle_self_consistency(
        &self,
        _domain: DomainId,
        _clock: &dyn Clock,
    ) -> Result<OracleSelfConsistency, OracleError> {
        self.0.clone()
    }
}

#[derive(Clone, Debug)]
struct AssayFixture(Result<PanelSufficiency, OracleError>);

impl SufficiencyAssay for AssayFixture {
    fn panel_sufficiency(
        &self,
        _panel: &Panel,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<PanelSufficiency, OracleError> {
        self.0.clone()
    }
}

#[derive(Clone, Debug)]
struct KernelFixture(Result<RecallReport, LodestarError>);

impl KernelFixture {
    fn ok(ratio: f32, n_queries_tested: usize) -> Self {
        Self(Ok(RecallReport {
            kernel_only: ratio,
            full: 1.0,
            ratio,
            n_queries_tested,
            held_out: held_out().held_out_ids,
            ..RecallReport::default()
        }))
    }
}

impl KernelRecallSource for KernelFixture {
    fn kernel_recall_report(
        &self,
        _held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<RecallReport, LodestarError> {
        self.0.clone()
    }
}

#[derive(Clone, Debug)]
struct CalFixture(Result<CalibrationMeasurement, OracleError>);

impl CalFixture {
    fn ok(calibration_error: f32) -> Self {
        Self(Ok(CalibrationMeasurement {
            calibration_error,
            held_out_count: 2,
            calibrated_slots: 1,
        }))
    }
}

impl CalibrationSource for CalFixture {
    fn calibration_measurement(
        &self,
        _domain: &DomainId,
        _held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<CalibrationMeasurement, OracleError> {
        self.0.clone()
    }
}

#[derive(Clone, Debug)]
struct GoodFixture(Result<GoodhartDefenseMeasurement, OracleError>);

impl GoodFixture {
    fn ok(pass_rate: f32, report_passed: bool, violation_count: usize) -> Self {
        Self(Ok(GoodhartDefenseMeasurement {
            pass_rate,
            held_out_count: 2,
            report_passed,
            violation_count,
        }))
    }
}

impl GoodhartDefenseSource for GoodFixture {
    fn goodhart_defense_measurement(
        &self,
        _domain: &DomainId,
        _held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<GoodhartDefenseMeasurement, OracleError> {
        self.0.clone()
    }
}

#[derive(Clone, Debug)]
struct MistakeFixture(Result<MistakeClosureMeasurement, OracleError>);

impl MistakeFixture {
    fn ok(recurring_mistakes: usize, replayed_mistakes: usize) -> Self {
        Self(Ok(MistakeClosureMeasurement {
            recurring_mistakes,
            replayed_mistakes,
        }))
    }
}

impl MistakeClosureSource for MistakeFixture {
    fn mistake_closure_measurement(
        &self,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<MistakeClosureMeasurement, OracleError> {
        self.0.clone()
    }
}

fn measure_report(fixtures: FixtureSet) -> Result<SuperIntelReport, OracleError> {
    measure_super_intelligence_tiers(SuperIntelligenceRequest {
        oracle: &fixtures.oracle,
        assay: &fixtures.assay,
        kernel: &fixtures.kernel,
        calibration: &fixtures.calibration,
        goodhart: &fixtures.goodhart,
        mistakes: &fixtures.mistakes,
        panel: &panel(),
        domain: domain(),
        held_out: &fixtures.held_out,
        clock: &clock(),
        short_circuit: ShortCircuit::MeasureAll,
    })
}

fn panel_sufficiency(panel_bits: f32, anchor_entropy_bits: f32) -> PanelSufficiency {
    PanelSufficiency {
        panel_bits,
        sufficiency_basis_bits: panel_bits,
        anchor_entropy_bits,
        sufficient: panel_bits >= anchor_entropy_bits,
        deficit_bits: (anchor_entropy_bits - panel_bits).max(0.0),
        deficits: Vec::new(),
        observation_scope: None,
        trust: TrustTag::Trusted,
        estimate_bound: calyx_assay::EstimateBound::LowerBound,
        power_calibration: None,
    }
}

fn panel() -> Panel {
    Panel {
        version: 1,
        slots: Vec::new(),
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn domain() -> DomainId {
    DomainId::from("ph50-t03-fixture")
}

fn clock() -> FixedClock {
    FixedClock::new(1_785_400_000)
}

fn held_out() -> HeldOutSplit {
    HeldOutSplit::new("held-out", vec![cx(1), cx(2)], vec![cx(3), cx(4)])
}

fn empty_held_out() -> HeldOutSplit {
    HeldOutSplit::new("empty", vec![cx(1), cx(2)], Vec::new())
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn tier(report: &SuperIntelReport, wanted: Tier) -> Option<&TierResult> {
    report.tiers.iter().find(|tier| tier.tier == wanted)
}

fn synthetic_error(code: &'static str) -> OracleError {
    CalyxError {
        code,
        message: "synthetic phase source unavailable".to_string(),
        remediation: "restore synthetic phase source",
    }
    .into()
}

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1.0e-6);
}
