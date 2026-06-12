//! PH50 full six-tier super-intelligence predicate.

use calyx_anneal::{GoodhartReport, RegressionReport, regression_rate};
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, LedgerRef, Panel, content_address};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use calyx_lodestar::LodestarError;
use calyx_ward::{GuardProfile, WardError};
use serde::{Deserialize, Serialize};

use crate::honesty_gate::SufficiencyAssay;
use crate::super_intel::{
    HeldOutSplit, KERNEL_RECALL_RATIO, KernelRecallSource, OracleConsistencySource, ShortCircuit,
    failed_tier, kernel_recall_tier, measure_tier_oracle_clean_from_result, oracle_error_fix,
    panel_sufficiency_tier, valid_measurement, validate_held_out,
};
use crate::{DomainId, OracleError, SuperIntelReport, Tier, TierResult};

pub const GOODHART_THRESHOLD: f32 = 0.9;
pub const CALIBRATION_CEILING_DELTA: f32 = 0.0;

const LEDGER_ACTOR: &str = "calyx-oracle";
const LEDGER_TAG: &str = "super_intelligence_v1";
const LABEL_HELD_OUT_ORACLE_FIX: &str = "label held-out oracle instances";
const CALIBRATION_FIX: &str = "run conformal calibration with more held-out instances";
const GOODHART_FIX: &str = "strengthen Gtau guard or add cross-lens anomaly detector";
const MISTAKE_FIX: &str = "trigger online head update for the recurring mistake pattern";
const KERNEL_PHASE_FIX: &str = "repair kernel recall source before super-intelligence measurement";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationMeasurement {
    pub calibration_error: f32,
    pub held_out_count: usize,
    pub calibrated_slots: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoodhartDefenseMeasurement {
    pub pass_rate: f32,
    pub held_out_count: usize,
    pub report_passed: bool,
    pub violation_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MistakeClosureMeasurement {
    pub recurring_mistakes: usize,
    pub replayed_mistakes: usize,
}

pub trait CalibrationSource {
    fn calibration_measurement(
        &self,
        domain: &DomainId,
        held_out: &HeldOutSplit,
        clock: &dyn Clock,
    ) -> Result<CalibrationMeasurement, OracleError>;
}

pub trait GoodhartDefenseSource {
    fn goodhart_defense_measurement(
        &self,
        domain: &DomainId,
        held_out: &HeldOutSplit,
        clock: &dyn Clock,
    ) -> Result<GoodhartDefenseMeasurement, OracleError>;
}

pub trait MistakeClosureSource {
    fn mistake_closure_measurement(
        &self,
        domain: &DomainId,
        clock: &dyn Clock,
    ) -> Result<MistakeClosureMeasurement, OracleError>;
}

impl CalibrationSource for GuardProfile {
    fn calibration_measurement(
        &self,
        domain: &DomainId,
        held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<CalibrationMeasurement, OracleError> {
        if self.domain != domain.as_str() {
            return Err(ward_error_to_oracle(WardError::InvalidDomain {
                reason: format!("guard domain {} != oracle domain {domain}", self.domain),
            }));
        }
        let calibration = self.calibration.as_ref().ok_or_else(|| {
            ward_error_to_oracle(WardError::Provisional {
                guard_id: self.guard_id,
            })
        })?;
        if !valid_calibration_profile(calibration) {
            return Err(ward_error_to_oracle(WardError::InvalidCalibrationInput {
                reason: "calibration FAR/FRR/confidence must be finite within [0, 1]",
            }));
        }
        let per_slot_far = calibration
            .per_slot
            .values()
            .map(|slot| slot.far)
            .max_by(f32::total_cmp);
        Ok(CalibrationMeasurement {
            calibration_error: per_slot_far.unwrap_or(calibration.far).max(calibration.far),
            held_out_count: held_out.held_out_count(),
            calibrated_slots: calibration
                .per_slot
                .len()
                .max(self.required_slots.len())
                .max(1),
        })
    }
}

impl GoodhartDefenseSource for GoodhartReport {
    fn goodhart_defense_measurement(
        &self,
        _domain: &DomainId,
        held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<GoodhartDefenseMeasurement, OracleError> {
        let pass_rate = self
            .in_region_frac
            .unwrap_or(if self.passed { 1.0 } else { 0.0 }) as f32;
        Ok(GoodhartDefenseMeasurement {
            pass_rate,
            held_out_count: held_out.held_out_count(),
            report_passed: self.passed,
            violation_count: self.violations.len(),
        })
    }
}

impl MistakeClosureSource for RegressionReport {
    fn mistake_closure_measurement(
        &self,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<MistakeClosureMeasurement, OracleError> {
        let _ = regression_rate(self).map_err(OracleError::from)?;
        Ok(MistakeClosureMeasurement {
            recurring_mistakes: self.regression_count,
            replayed_mistakes: self.results.len(),
        })
    }
}

pub struct SuperIntelligenceRequest<'a, O, A, K, C, G, M> {
    pub oracle: &'a O,
    pub assay: &'a A,
    pub kernel: &'a K,
    pub calibration: &'a C,
    pub goodhart: &'a G,
    pub mistakes: &'a M,
    pub panel: &'a Panel,
    pub domain: DomainId,
    pub held_out: &'a HeldOutSplit,
    pub clock: &'a dyn Clock,
    pub short_circuit: ShortCircuit,
}

pub fn measure_tier_calibrated<S>(
    source: &S,
    oracle_self_consistency_ceiling: f32,
    domain: &DomainId,
    held_out: &HeldOutSplit,
    clock: &dyn Clock,
) -> Result<TierResult, OracleError>
where
    S: CalibrationSource,
{
    let threshold = oracle_self_consistency_ceiling + CALIBRATION_CEILING_DELTA;
    if held_out.held_out_ids.is_empty() {
        return Ok(failed_tier(
            Tier::Calibrated,
            sanitized(threshold),
            LABEL_HELD_OUT_ORACLE_FIX.to_string(),
        ));
    }
    let measurement = source.calibration_measurement(domain, held_out, clock)?;
    let passed = valid_measurement(measurement.calibration_error, threshold)
        && measurement.calibration_error <= threshold;
    Ok(TierResult::new(
        Tier::Calibrated,
        passed,
        sanitized(measurement.calibration_error),
        sanitized(threshold),
        (!passed).then(|| CALIBRATION_FIX.to_string()),
    ))
}

pub fn measure_tier_goodhart_defended<S>(
    source: &S,
    domain: &DomainId,
    held_out: &HeldOutSplit,
    clock: &dyn Clock,
) -> Result<TierResult, OracleError>
where
    S: GoodhartDefenseSource,
{
    if held_out.held_out_ids.is_empty() {
        return Ok(failed_tier(
            Tier::GoodhartDefended,
            GOODHART_THRESHOLD,
            LABEL_HELD_OUT_ORACLE_FIX.to_string(),
        ));
    }
    let measurement = match source.goodhart_defense_measurement(domain, held_out, clock) {
        Ok(measurement) => measurement,
        Err(error) => {
            return Ok(failed_tier(
                Tier::GoodhartDefended,
                GOODHART_THRESHOLD,
                oracle_error_fix(&error),
            ));
        }
    };
    let passed = measurement.report_passed
        && valid_measurement(measurement.pass_rate, GOODHART_THRESHOLD)
        && measurement.pass_rate >= GOODHART_THRESHOLD;
    Ok(TierResult::new(
        Tier::GoodhartDefended,
        passed,
        sanitized(measurement.pass_rate),
        GOODHART_THRESHOLD,
        (!passed).then(|| GOODHART_FIX.to_string()),
    ))
}

pub fn measure_tier_mistake_closed<S>(
    source: &S,
    domain: &DomainId,
    clock: &dyn Clock,
) -> Result<TierResult, OracleError>
where
    S: MistakeClosureSource,
{
    let measurement = source.mistake_closure_measurement(domain, clock)?;
    let measured_value = measurement.recurring_mistakes as f32;
    let passed = measured_value.is_finite() && measurement.recurring_mistakes == 0;
    Ok(TierResult::new(
        Tier::MistakeClosed,
        passed,
        sanitized(measured_value),
        0.0,
        (!passed).then(|| MISTAKE_FIX.to_string()),
    ))
}

pub fn measure_super_intelligence_tiers<O, A, K, C, G, M>(
    request: SuperIntelligenceRequest<'_, O, A, K, C, G, M>,
) -> Result<SuperIntelReport, OracleError>
where
    O: OracleConsistencySource,
    A: SufficiencyAssay,
    K: KernelRecallSource,
    C: CalibrationSource,
    G: GoodhartDefenseSource,
    M: MistakeClosureSource,
{
    let domain = request.domain.clone();
    let oracle_result = request
        .oracle
        .oracle_self_consistency(domain.clone(), request.clock)?;
    let mut tiers = Vec::with_capacity(Tier::ORDER.len());

    tiers.push(measure_tier_oracle_clean_from_result(&oracle_result));
    if should_stop(&tiers, request.short_circuit) {
        return Ok(SuperIntelReport::new(domain, tiers));
    }

    let panel = request
        .assay
        .panel_sufficiency(request.panel, &domain, request.clock)?;
    tiers.push(panel_sufficiency_tier(request.panel, &panel));
    if should_stop(&tiers, request.short_circuit) {
        return Ok(SuperIntelReport::new(domain, tiers));
    }

    tiers.push(measure_kernel_tier(
        request.kernel,
        request.held_out,
        request.clock,
    )?);
    if should_stop(&tiers, request.short_circuit) {
        return Ok(SuperIntelReport::new(domain, tiers));
    }

    tiers.push(measure_tier_calibrated(
        request.calibration,
        oracle_result.ceiling,
        &domain,
        request.held_out,
        request.clock,
    )?);
    if should_stop(&tiers, request.short_circuit) {
        return Ok(SuperIntelReport::new(domain, tiers));
    }

    tiers.push(measure_tier_goodhart_defended(
        request.goodhart,
        &domain,
        request.held_out,
        request.clock,
    )?);
    if should_stop(&tiers, request.short_circuit) {
        return Ok(SuperIntelReport::new(domain, tiers));
    }

    tiers.push(measure_tier_mistake_closed(
        request.mistakes,
        &domain,
        request.clock,
    )?);
    Ok(SuperIntelReport::new(domain, tiers))
}

pub fn super_intelligence<C, O, A, K, Cal, G, M>(
    vault: &AsterVault<C>,
    request: SuperIntelligenceRequest<'_, O, A, K, Cal, G, M>,
) -> Result<SuperIntelReport, OracleError>
where
    C: Clock,
    O: OracleConsistencySource,
    A: SufficiencyAssay,
    K: KernelRecallSource,
    Cal: CalibrationSource,
    G: GoodhartDefenseSource,
    M: MistakeClosureSource,
{
    Ok(super_intelligence_with_ledger(vault, request)?.0)
}

pub fn super_intelligence_with_ledger<C, O, A, K, Cal, G, M>(
    vault: &AsterVault<C>,
    request: SuperIntelligenceRequest<'_, O, A, K, Cal, G, M>,
) -> Result<(SuperIntelReport, LedgerRef), OracleError>
where
    C: Clock,
    O: OracleConsistencySource,
    A: SufficiencyAssay,
    K: KernelRecallSource,
    Cal: CalibrationSource,
    G: GoodhartDefenseSource,
    M: MistakeClosureSource,
{
    let clock = request.clock;
    let report = measure_super_intelligence_tiers(request)?;
    let ledger_ref = write_super_intelligence_ledger(vault, &report, clock)?;
    Ok((report, ledger_ref))
}

pub fn write_super_intelligence_ledger<C>(
    vault: &AsterVault<C>,
    report: &SuperIntelReport,
    clock: &dyn Clock,
) -> Result<LedgerRef, OracleError>
where
    C: Clock,
{
    let payload = SuperIntelLedgerPayload {
        tag: LEDGER_TAG,
        domain: report.domain.as_str(),
        overall: report.overall,
        failing_tier: report.failing_tier,
        cheapest_fix: report.cheapest_fix.as_deref(),
        tiers: &report.tiers,
        ts: clock.now(),
    };
    let bytes = serde_json::to_vec(&payload).map_err(|_| OracleError::LedgerWriteFailure)?;
    vault
        .append_ledger_entry(
            EntryKind::Assay,
            SubjectId::Query(super_intel_subject(&report.domain).to_vec()),
            bytes,
            ActorId::Service(LEDGER_ACTOR.to_string()),
        )
        .map_err(|_| OracleError::LedgerWriteFailure)
}

fn measure_kernel_tier<K>(
    source: &K,
    held_out: &HeldOutSplit,
    clock: &dyn Clock,
) -> Result<TierResult, OracleError>
where
    K: KernelRecallSource,
{
    match validate_held_out(held_out) {
        Ok(()) => {}
        Err(LodestarError::RecallEmptyCorpus) => {
            return Ok(failed_tier(
                Tier::KernelExists,
                KERNEL_RECALL_RATIO,
                "label held-out instances".to_string(),
            ));
        }
        Err(LodestarError::RecallInvalidParams { detail }) => {
            return Ok(failed_tier(
                Tier::KernelExists,
                KERNEL_RECALL_RATIO,
                format!("CALYX_RECALL_INVALID_PARAMS: {detail}"),
            ));
        }
        Err(error) => return Err(lodestar_to_oracle(error)),
    }
    source
        .kernel_recall_report(held_out, clock)
        .map(|report| kernel_recall_tier(&report))
        .map_err(lodestar_to_oracle)
}

fn should_stop(tiers: &[TierResult], short_circuit: ShortCircuit) -> bool {
    matches!(short_circuit, ShortCircuit::Enabled) && tiers.last().is_some_and(|tier| !tier.passed)
}

fn valid_calibration_profile(meta: &calyx_ward::CalibrationMeta) -> bool {
    valid_fraction(meta.far)
        && valid_fraction(meta.frr)
        && valid_fraction(meta.confidence)
        && meta.per_slot.values().all(|slot| {
            valid_fraction(slot.far) && valid_fraction(slot.frr) && valid_fraction(slot.confidence)
        })
}

fn valid_fraction(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn sanitized(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn ward_error_to_oracle(error: WardError) -> OracleError {
    CalyxError {
        code: error.code(),
        message: error.to_string(),
        remediation: CALIBRATION_FIX,
    }
    .into()
}

fn lodestar_to_oracle(error: LodestarError) -> OracleError {
    CalyxError {
        code: error.code(),
        message: error.to_string(),
        remediation: KERNEL_PHASE_FIX,
    }
    .into()
}

fn super_intel_subject(domain: &DomainId) -> [u8; 16] {
    content_address([domain.as_str().as_bytes(), LEDGER_TAG.as_bytes()])
}

#[derive(Serialize)]
struct SuperIntelLedgerPayload<'a> {
    tag: &'static str,
    domain: &'a str,
    overall: bool,
    failing_tier: Option<Tier>,
    cheapest_fix: Option<&'a str>,
    tiers: &'a [TierResult],
    ts: u64,
}

#[cfg(test)]
#[path = "super_intel_full_tests.rs"]
mod tests;
