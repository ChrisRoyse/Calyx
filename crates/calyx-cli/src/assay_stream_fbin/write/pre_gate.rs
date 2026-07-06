use std::collections::BTreeSet;
use std::fs;

use calyx_assay::{DEFAULT_MIN_POWER_RECOVERY_RATIO, MIN_INFORMATIVE_TARGET_ENTROPY_BITS};
use serde::Deserialize;

use crate::assay_anchor_audit::AnchorAudit;
use crate::error::CliResult;

use super::super::args::{Args, StreamMode};
use super::super::{MIN_A35_LENSES, io_error, local_error};
use super::evidence::PreEncodeGateEvidence;
use super::paths::display;

mod roster;

#[derive(Debug, Deserialize)]
struct BitsReport {
    anchor_entropy_bits: Option<f32>,
    min_informative_target_entropy_bits: Option<f32>,
    panel: Option<PanelGateRaw>,
    anchor_audit: Option<AnchorAudit>,
    anchor_leaks_into_input: Option<bool>,
    trivial_anchor: Option<bool>,
    grounded_gate_eligible: Option<bool>,
    report: Option<BitsReportInner>,
}

#[derive(Debug, Deserialize)]
struct BitsReportInner {
    anchor_entropy_bits: Option<f32>,
    min_informative_target_entropy_bits: Option<f32>,
    panel: Option<PanelGateRaw>,
    anchor_audit: Option<AnchorAudit>,
    anchor_leaks_into_input: Option<bool>,
    trivial_anchor: Option<bool>,
    grounded_gate_eligible: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PanelGateRaw {
    admitted_lenses: Option<Vec<String>>,
    estimate_bound: Option<String>,
    sufficiency_basis_bits: Option<f32>,
    power_calibration_status: Option<String>,
    power_recovery_ratio: Option<f32>,
    power_recovered_bits: Option<f32>,
    power_planted_bits: Option<f32>,
}

struct GateInputs {
    anchor_entropy_bits: f32,
    sufficiency_basis_bits: f32,
    estimate_bound: String,
    power_calibration_status: String,
    power_recovery_ratio: f32,
    admitted_lenses: Vec<String>,
    anchor_audit: AnchorAudit,
}

pub(super) fn validate_before_full_encode(args: &Args) -> CliResult<PreEncodeGateEvidence> {
    if args.a37_admission_cf_root.is_some() {
        return validate_db_admission_before_full_encode(args);
    }
    if args.diagnostic_bootstrap_without_admission() {
        return diagnostic_bootstrap_evidence(args);
    }
    if args.mode.requires_gate() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_REQUIRED",
            "--bits-report is diagnostic-only; gate mode must read A37 admission from Calyx/Aster",
            "write and read the A37 admission row through Calyx/Aster Graph CF before streaming",
        ));
    }
    let bits_report = args.bits_report.as_ref().ok_or_else(|| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_MISSING",
            "missing --bits-report",
            "pass a bits report or a DB-native A37 admission CF root",
        )
    })?;
    let report: BitsReport = serde_json::from_slice(
        &fs::read(bits_report).map_err(io_error)?,
    )
    .map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_INVALID",
            format!("parse {} failed: {error}", bits_report.display()),
            "pass assay_abundance.json or full bits-validate evidence with panel sufficiency metadata",
        )
    })?;
    let gate = gate_inputs(&report)?;
    let grounded_gate_eligible = is_grounded_gate_eligible(&gate.anchor_audit);
    if args.mode.requires_gate() {
        gate.anchor_audit
            .require_gate_eligible("assay stream-fbin pre-encode grounded anchor gate")?;
    }
    let streamed_lenses = roster::streamed_lens_names(args)?;
    validate_panel_identity(&gate.admitted_lenses, &streamed_lenses, args.mode)?;
    validate_target_entropy(&gate)?;
    validate_lower_bound(&gate)?;
    validate_power(&gate)?;
    let (sufficient, deficit_bits) = validate_sufficiency(&gate, args.mode)?;
    Ok(PreEncodeGateEvidence {
        mode: args.mode.as_str(),
        diagnostic_only: !args.mode.requires_gate() || !grounded_gate_eligible || !sufficient,
        bits_report: display(bits_report),
        anchor_entropy_bits: gate.anchor_entropy_bits,
        sufficiency_basis_bits: gate.sufficiency_basis_bits,
        power_adjusted_target_bits: power_adjusted_target_bits(&gate),
        deficit_bits,
        estimate_bound: gate.estimate_bound,
        power_calibration_status: gate.power_calibration_status,
        power_recovery_ratio: gate.power_recovery_ratio,
        min_power_recovery_ratio: DEFAULT_MIN_POWER_RECOVERY_RATIO,
        sufficient,
        grounded_gate_eligible,
        anchor_audit: gate.anchor_audit,
        admitted_lenses: gate.admitted_lenses,
        streamed_lenses,
    })
}

fn validate_db_admission_before_full_encode(args: &Args) -> CliResult<PreEncodeGateEvidence> {
    let report = super::bits::load_a37_admission(args)?;
    let admitted_lenses = report
        .lenses
        .iter()
        .map(|lens| lens.name.clone())
        .collect::<Vec<_>>();
    let streamed_lenses = roster::streamed_lens_names(args)?;
    validate_panel_identity(&admitted_lenses, &streamed_lenses, args.mode)?;
    let sufficient = validate_db_gate(&report, args.mode)?;
    let deficit_bits = (report.min_marginal_bits - report.min_best_marginal_bits).max(0.0);
    Ok(PreEncodeGateEvidence {
        mode: args.mode.as_str(),
        diagnostic_only: !args.mode.requires_gate() || !sufficient,
        bits_report: format!(
            "aster-graph-cf:{}:{}",
            args.a37_admission_cf_root
                .as_ref()
                .expect("checked by caller")
                .display(),
            args.a37_admission_key
        ),
        anchor_entropy_bits: report.max_best_marginal_bits,
        sufficiency_basis_bits: report.min_best_marginal_bits,
        power_adjusted_target_bits: report.min_marginal_bits,
        deficit_bits,
        estimate_bound: "a37_multi_anchor_best_target".to_string(),
        power_calibration_status: "db_readback_passed".to_string(),
        power_recovery_ratio: 1.0,
        min_power_recovery_ratio: DEFAULT_MIN_POWER_RECOVERY_RATIO,
        sufficient,
        grounded_gate_eligible: report.gate_passed,
        anchor_audit: AnchorAudit {
            grounded_gate_eligible: report.gate_passed,
            audit_kind: Some("a37_admission_db".to_string()),
            source: Some("calyx/a37/admission/v1".to_string()),
            reason: Some(format!(
                "DB-native multi-anchor A37 admission status={} gate_passed={}",
                report.status, report.gate_passed
            )),
            ..AnchorAudit::default()
        },
        admitted_lenses,
        streamed_lenses,
    })
}

fn diagnostic_bootstrap_evidence(args: &Args) -> CliResult<PreEncodeGateEvidence> {
    Ok(PreEncodeGateEvidence {
        mode: args.mode.as_str(),
        diagnostic_only: true,
        bits_report: "none:diagnostic_bootstrap_without_a37_admission".to_string(),
        anchor_entropy_bits: 0.0,
        sufficiency_basis_bits: 0.0,
        power_adjusted_target_bits: args.min_bits,
        deficit_bits: args.min_bits,
        estimate_bound: "missing_db_admission".to_string(),
        power_calibration_status: "not_run".to_string(),
        power_recovery_ratio: 0.0,
        min_power_recovery_ratio: DEFAULT_MIN_POWER_RECOVERY_RATIO,
        sufficient: false,
        grounded_gate_eligible: false,
        anchor_audit: AnchorAudit {
            grounded_gate_eligible: false,
            audit_kind: Some("diagnostic_bootstrap_without_a37_admission".to_string()),
            source: Some("assay stream-fbin diagnostic bootstrap".to_string()),
            reason: Some(
                "no Calyx/Aster A37 admission row supplied; this encode is diagnostic-only"
                    .to_string(),
            ),
            ..AnchorAudit::default()
        },
        admitted_lenses: Vec::new(),
        streamed_lenses: roster::streamed_lens_names(args)?,
    })
}

fn validate_db_gate(
    report: &crate::assay_multi_anchor_card::model::MultiAnchorReport,
    mode: StreamMode,
) -> CliResult<bool> {
    if report.lens_count < MIN_A35_LENSES || report.lenses.len() < MIN_A35_LENSES {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_PANEL_TOO_SMALL",
            format!(
                "A37 admission lens_count={} lenses={}; A35 requires at least {MIN_A35_LENSES}",
                report.lens_count,
                report.lenses.len()
            ),
            "write a DB-native A37 admission for at least ten real content lenses",
        ));
    }
    let all_lenses_pass = report.passing_lens_count == report.lens_count
        && report.lenses.iter().all(|lens| lens.passed);
    let min_bits_pass = report.min_best_marginal_bits >= report.min_marginal_bits;
    let passed = report.gate_passed
        && report.family_span_pass
        && report.redundancy_bound_pass
        && report.no_collapse_pass
        && all_lenses_pass
        && min_bits_pass;
    if !passed && mode.requires_gate() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_REFUSED",
            format!(
                "A37 DB admission refused: status={} gate_passed={} family_span={} redundancy={} no_collapse={} passing_lenses={}/{} weakest_lens={} min_best_marginal_bits={:.6} required={:.6}",
                report.status,
                report.gate_passed,
                report.family_span_pass,
                report.redundancy_bound_pass,
                report.no_collapse_pass,
                report.passing_lens_count,
                report.lens_count,
                report.weakest_lens,
                report.min_best_marginal_bits,
                report.min_marginal_bits
            ),
            "pass a gate-passed DB-native multi-anchor A37 admission row for the exact manifest roster",
        ));
    }
    Ok(passed)
}

fn gate_inputs(report: &BitsReport) -> CliResult<GateInputs> {
    let anchor_entropy_bits = report
        .anchor_entropy_bits
        .or_else(|| report.report.as_ref()?.anchor_entropy_bits)
        .ok_or_else(|| missing_metadata("anchor_entropy_bits"))?;
    let min_entropy = report
        .min_informative_target_entropy_bits
        .or_else(|| report.report.as_ref()?.min_informative_target_entropy_bits)
        .unwrap_or(MIN_INFORMATIVE_TARGET_ENTROPY_BITS);
    if !min_entropy.is_finite() || min_entropy < MIN_INFORMATIVE_TARGET_ENTROPY_BITS {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_INVALID",
            format!(
                "min_informative_target_entropy_bits={min_entropy} below required {MIN_INFORMATIVE_TARGET_ENTROPY_BITS}"
            ),
            "regenerate the bits report with the current calibrated assay writer",
        ));
    }
    let panel = report
        .panel
        .as_ref()
        .or_else(|| report.report.as_ref()?.panel.as_ref())
        .ok_or_else(|| missing_metadata("panel"))?;
    let admitted_lenses = panel
        .admitted_lenses
        .clone()
        .ok_or_else(|| missing_metadata("panel.admitted_lenses"))?;
    let estimate_bound = panel
        .estimate_bound
        .clone()
        .ok_or_else(|| missing_metadata("panel.estimate_bound"))?;
    let power_calibration_status = panel
        .power_calibration_status
        .clone()
        .ok_or_else(|| missing_metadata("panel.power_calibration_status"))?;
    let gate = GateInputs {
        anchor_entropy_bits,
        sufficiency_basis_bits: required_f32(
            panel.sufficiency_basis_bits,
            "panel.sufficiency_basis_bits",
        )?,
        estimate_bound,
        power_calibration_status,
        power_recovery_ratio: required_f32(
            panel.power_recovery_ratio,
            "panel.power_recovery_ratio",
        )?,
        admitted_lenses,
        anchor_audit: report_anchor_audit(report),
    };
    required_f32(panel.power_recovered_bits, "panel.power_recovered_bits")?;
    required_f32(panel.power_planted_bits, "panel.power_planted_bits")?;
    Ok(gate)
}

fn report_anchor_audit(report: &BitsReport) -> AnchorAudit {
    let inner = report.report.as_ref();
    AnchorAudit::from_parts(
        report
            .anchor_audit
            .clone()
            .or_else(|| inner.and_then(|value| value.anchor_audit.clone())),
        report
            .anchor_leaks_into_input
            .or_else(|| inner.and_then(|value| value.anchor_leaks_into_input)),
        report
            .trivial_anchor
            .or_else(|| inner.and_then(|value| value.trivial_anchor)),
        report
            .grounded_gate_eligible
            .or_else(|| inner.and_then(|value| value.grounded_gate_eligible)),
    )
}

fn validate_panel_identity(
    admitted_lenses: &[String],
    streamed_lenses: &[String],
    mode: StreamMode,
) -> CliResult {
    let admitted = names_set("panel.admitted_lenses", admitted_lenses)?;
    let streamed = names_set("streamed manifests", streamed_lenses)?;
    if admitted.len() < MIN_A35_LENSES && mode.requires_gate() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_PANEL_TOO_SMALL",
            format!(
                "panel gate has {} admitted lenses; A35 requires at least {MIN_A35_LENSES}",
                admitted.len()
            ),
            "run bits-validate on at least ten real frozen content lenses",
        ));
    }
    if admitted != streamed && mode.requires_gate() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_PANEL_MISMATCH",
            format!(
                "bits-report admitted_lenses={} streamed_manifests={}",
                joined(admitted.iter()),
                joined(streamed.iter())
            ),
            "stream exactly the same admitted panel that produced the calibrated sufficiency report",
        ));
    }
    Ok(())
}

fn validate_target_entropy(gate: &GateInputs) -> CliResult {
    if !gate.anchor_entropy_bits.is_finite()
        || gate.anchor_entropy_bits < MIN_INFORMATIVE_TARGET_ENTROPY_BITS
    {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_TARGET_UNINFORMATIVE",
            format!(
                "anchor_entropy_bits={} below required {}",
                gate.anchor_entropy_bits, MIN_INFORMATIVE_TARGET_ENTROPY_BITS
            ),
            "use a grounded target class with enough binary outcome entropy",
        ));
    }
    Ok(())
}

fn validate_lower_bound(gate: &GateInputs) -> CliResult {
    if gate.estimate_bound != "lower_bound" {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_NOT_LOWER_BOUND",
            format!("panel.estimate_bound={}", gate.estimate_bound),
            "regenerate the bits report with a lower-bound sufficiency estimate",
        ));
    }
    Ok(())
}

fn validate_power(gate: &GateInputs) -> CliResult {
    if gate.power_calibration_status != "passed"
        || !gate.power_recovery_ratio.is_finite()
        || gate.power_recovery_ratio < DEFAULT_MIN_POWER_RECOVERY_RATIO
    {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_UNPOWERED",
            format!(
                "panel power status={} recovery_ratio={} min={}",
                gate.power_calibration_status,
                gate.power_recovery_ratio,
                DEFAULT_MIN_POWER_RECOVERY_RATIO
            ),
            "increase the cheap sample or replace the panel before full encode",
        ));
    }
    Ok(())
}

fn validate_sufficiency(gate: &GateInputs, mode: StreamMode) -> CliResult<(bool, f32)> {
    if !gate.sufficiency_basis_bits.is_finite() || gate.sufficiency_basis_bits < 0.0 {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_INVALID",
            format!(
                "panel.sufficiency_basis_bits={} must be finite and non-negative",
                gate.sufficiency_basis_bits
            ),
            "regenerate the bits report with a valid panel lower bound",
        ));
    }
    // #1140: the sufficiency target is power-adjusted. The same estimator that
    // measured the panel also measured a planted perfect signal on identical
    // data/dims (power calibration); its recovery ratio is the estimator's
    // ceiling. Comparing the raw lower bound against the full anchor entropy
    // is unreachable even for a perfect panel. validate_power (which runs
    // first) has already proven the ratio is finite and above the floor.
    let target_bits = power_adjusted_target_bits(gate);
    let deficit_bits = (target_bits - gate.sufficiency_basis_bits).max(0.0);
    if deficit_bits > 0.0 && mode.requires_gate() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_REFUSED",
            format!(
                "panel lower-bound sufficiency {:.6} bits is below the power-adjusted target {:.6} bits (anchor entropy {:.6} x power recovery {:.6}); deficit {:.6}",
                gate.sufficiency_basis_bits,
                target_bits,
                gate.anchor_entropy_bits,
                gate.power_recovery_ratio,
                deficit_bits
            ),
            "acquire or admit stronger grounded content lenses before full encode",
        ));
    }
    Ok((deficit_bits == 0.0, deficit_bits))
}

fn power_adjusted_target_bits(gate: &GateInputs) -> f32 {
    gate.anchor_entropy_bits * gate.power_recovery_ratio.min(1.0)
}

fn is_grounded_gate_eligible(audit: &AnchorAudit) -> bool {
    audit.grounded_gate_eligible
        && !audit.anchor_leaks_into_input
        && !audit.trivial_anchor
        && !audit.label_recoverable_from_input
}

fn names_set(label: &str, names: &[String]) -> CliResult<BTreeSet<String>> {
    let mut set = BTreeSet::new();
    for name in names {
        if name.trim().is_empty() {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_INVALID",
                format!("{label} contains an empty lens name"),
                "regenerate the bits report and manifests with stable non-empty lens names",
            ));
        }
        if !set.insert(name.clone()) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_INVALID",
                format!("{label} contains duplicate lens {name}"),
                "regenerate the bits report and manifests with a unique panel roster",
            ));
        }
    }
    Ok(set)
}

fn required_f32(value: Option<f32>, name: &'static str) -> CliResult<f32> {
    let value = value.ok_or_else(|| missing_metadata(name))?;
    if !value.is_finite() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_INVALID",
            format!("{name} is non-finite: {value}"),
            "regenerate the bits report with finite calibrated assay metadata",
        ));
    }
    Ok(value)
}

fn missing_metadata(name: &'static str) -> crate::error::CliError {
    local_error(
        "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_MISSING",
        format!("bits report missing {name}"),
        "rerun assay bits-validate with the current calibrated assay writer",
    )
}

fn joined<'a>(items: impl Iterator<Item = &'a String>) -> String {
    items.cloned().collect::<Vec<_>>().join(",")
}
