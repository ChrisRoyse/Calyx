use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use calyx_assay::{DEFAULT_MIN_POWER_RECOVERY_RATIO, MIN_INFORMATIVE_TARGET_ENTROPY_BITS};
use calyx_registry::LensForgeManifest;
use serde::Deserialize;

use crate::assay_anchor_audit::AnchorAudit;
use crate::error::CliResult;

use super::super::args::{Args, StreamMode};
use super::super::{MIN_A35_LENSES, io_error, local_error};
use super::evidence::PreEncodeGateEvidence;
use super::paths::display;

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
    let report: BitsReport = serde_json::from_slice(
        &fs::read(&args.bits_report).map_err(io_error)?,
    )
    .map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_INVALID",
            format!("parse {} failed: {error}", args.bits_report.display()),
            "pass assay_abundance.json or full bits-validate evidence with panel sufficiency metadata",
        )
    })?;
    let gate = gate_inputs(&report)?;
    let grounded_gate_eligible = is_grounded_gate_eligible(&gate.anchor_audit);
    if args.mode.requires_gate() {
        gate.anchor_audit
            .require_gate_eligible("assay stream-fbin pre-encode grounded anchor gate")?;
    }
    let streamed_lenses = streamed_manifest_names(args)?;
    validate_panel_identity(&gate.admitted_lenses, &streamed_lenses)?;
    validate_target_entropy(&gate)?;
    validate_lower_bound(&gate)?;
    validate_power(&gate)?;
    let (sufficient, deficit_bits) = validate_sufficiency(&gate, args.mode)?;
    Ok(PreEncodeGateEvidence {
        mode: args.mode.as_str(),
        diagnostic_only: !args.mode.requires_gate() || !grounded_gate_eligible || !sufficient,
        bits_report: display(&args.bits_report),
        anchor_entropy_bits: gate.anchor_entropy_bits,
        sufficiency_basis_bits: gate.sufficiency_basis_bits,
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

fn validate_panel_identity(admitted_lenses: &[String], streamed_lenses: &[String]) -> CliResult {
    let admitted = names_set("panel.admitted_lenses", admitted_lenses)?;
    let streamed = names_set("streamed manifests", streamed_lenses)?;
    if admitted.len() < MIN_A35_LENSES {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_PANEL_TOO_SMALL",
            format!(
                "panel gate has {} admitted lenses; A35 requires at least {MIN_A35_LENSES}",
                admitted.len()
            ),
            "run bits-validate on at least ten real frozen content lenses",
        ));
    }
    if admitted != streamed {
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
    let deficit_bits = (gate.anchor_entropy_bits - gate.sufficiency_basis_bits).max(0.0);
    if deficit_bits > 0.0 && mode.requires_gate() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PRE_GATE_REFUSED",
            format!(
                "panel lower-bound sufficiency {:.6} bits is below anchor entropy {:.6} bits; deficit {:.6}",
                gate.sufficiency_basis_bits,
                gate.anchor_entropy_bits,
                gate.anchor_entropy_bits - gate.sufficiency_basis_bits
            ),
            "acquire or admit stronger grounded content lenses before full encode",
        ));
    }
    Ok((deficit_bits == 0.0, deficit_bits))
}

fn is_grounded_gate_eligible(audit: &AnchorAudit) -> bool {
    audit.grounded_gate_eligible
        && !audit.anchor_leaks_into_input
        && !audit.trivial_anchor
        && !audit.label_recoverable_from_input
}

fn streamed_manifest_names(args: &Args) -> CliResult<Vec<String>> {
    args.manifests
        .iter()
        .map(|path| read_manifest_name(path))
        .collect()
}

fn read_manifest_name(path: &Path) -> CliResult<String> {
    let manifest: LensForgeManifest = serde_json::from_slice(&fs::read(path).map_err(io_error)?)
        .map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                format!("parse {} failed: {error}", path.display()),
                "fix the frozen lens manifests before streaming FBIN",
            )
        })?;
    if manifest.name.trim().is_empty() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
            format!("{} has an empty lens name", path.display()),
            "fix the frozen lens manifest before streaming FBIN",
        ));
    }
    Ok(manifest.name)
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
