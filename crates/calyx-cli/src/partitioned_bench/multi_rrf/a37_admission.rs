use std::collections::BTreeMap;
use std::path::Path;

use calyx_assay::{A37_DIVERSITY_GATE_PASSED, DEFAULT_MIN_MARGINAL_BITS};
use calyx_core::CalyxError;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::Plan;
use crate::a37_admission_store;
use crate::assay_multi_anchor_card::model::MultiAnchorReport;
use crate::error::{CliError, CliResult};

const MIN_LENSES: usize = 10;
const SCHEMA_VERSION: u32 = 1;
const ROLE: &str = "a37_multi_anchor_admission_card";

#[derive(Debug, Deserialize)]
struct MultiAnchorAdmission {
    schema_version: u32,
    role: String,
    status: String,
    gate_passed: bool,
    lens_count: usize,
    passing_lens_count: usize,
    min_marginal_bits: f32,
    family_span_pass: bool,
    redundancy_bound_pass: bool,
    no_collapse_pass: bool,
    association_family_count: usize,
    min_best_marginal_bits: f32,
    max_best_marginal_bits: f32,
    weakest_lens: String,
    lenses: Vec<AdmissionLens>,
    source_reports: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AdmissionLens {
    slot: u16,
    name: String,
    passed: bool,
    best_marginal_bits: f32,
}

pub(super) fn load(path: Option<&Path>, plan: &Plan) -> CliResult<Option<Value>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let bytes = std::fs::read(path).map_err(|err| {
        error(
            "CALYX_FSV_A37_ADMISSION_CARD_IO",
            format!("read A37 admission card {} failed: {err}", path.display()),
            "pass a byte-readable multi_anchor_ensemble_card.json",
        )
    })?;
    let card: MultiAnchorAdmission = serde_json::from_slice(&bytes).map_err(|err| {
        error(
            "CALYX_FSV_A37_ADMISSION_CARD_INVALID",
            format!("parse A37 admission card {} failed: {err}", path.display()),
            "regenerate the multi-anchor card from the same plan roster",
        )
    })?;
    validate(&card, plan)?;
    Ok(Some(report(path, &bytes, &card)))
}

pub(super) fn load_from_cf(
    cf_root: Option<&Path>,
    association_key: &str,
    plan: &Plan,
) -> CliResult<Option<Value>> {
    let Some(cf_root) = cf_root else {
        return Ok(None);
    };
    let (record, readback) =
        a37_admission_store::read::<MultiAnchorReport>(cf_root, association_key)
            .map_err(CliError::from)?;
    let card = from_report(record);
    validate(&card, plan)?;
    Ok(Some(report_db(&readback, &card)))
}

fn from_report(report: MultiAnchorReport) -> MultiAnchorAdmission {
    MultiAnchorAdmission {
        schema_version: report.schema_version,
        role: report.role,
        status: report.status,
        gate_passed: report.gate_passed,
        lens_count: report.lens_count,
        passing_lens_count: report.passing_lens_count,
        min_marginal_bits: report.min_marginal_bits,
        family_span_pass: report.family_span_pass,
        redundancy_bound_pass: report.redundancy_bound_pass,
        no_collapse_pass: report.no_collapse_pass,
        association_family_count: report.association_family_count,
        min_best_marginal_bits: report.min_best_marginal_bits,
        max_best_marginal_bits: report.max_best_marginal_bits,
        weakest_lens: report.weakest_lens,
        lenses: report
            .lenses
            .into_iter()
            .map(|lens| AdmissionLens {
                slot: lens.slot,
                name: lens.name,
                passed: lens.passed,
                best_marginal_bits: lens.best_marginal_bits,
            })
            .collect(),
        source_reports: report.source_reports,
    }
}

fn validate(card: &MultiAnchorAdmission, plan: &Plan) -> CliResult {
    if card.schema_version != SCHEMA_VERSION || card.role != ROLE || card.source_reports.is_empty()
    {
        return Err(error(
            "CALYX_FSV_A37_ADMISSION_CARD_INVALID",
            format!(
                "A37 admission card schema_version={} role={} source_reports={}",
                card.schema_version,
                card.role,
                card.source_reports.len()
            ),
            "pass a schema v1 a37_multi_anchor_admission_card with source report readback",
        ));
    }
    if card.lens_count < MIN_LENSES || card.lenses.len() < MIN_LENSES {
        return Err(error(
            "CALYX_FSV_A37_ADMISSION_CARD_TOO_SMALL",
            format!(
                "A37 admission card lens_count={} lenses={}; A35 requires {MIN_LENSES}",
                card.lens_count,
                card.lenses.len()
            ),
            "generate the multi-anchor admission card from a real >=10-lens panel",
        ));
    }
    if card.lens_count != plan.slots.len() || card.lenses.len() != plan.slots.len() {
        return Err(stale(format!(
            "A37 admission card lens count {} / {} != plan slots {}",
            card.lens_count,
            card.lenses.len(),
            plan.slots.len()
        )));
    }
    let plan_names = plan
        .slots
        .iter()
        .map(|slot| {
            slot.name
                .as_ref()
                .map(|name| (slot.slot, name.clone()))
                .ok_or_else(|| stale_message(format!("slot {} missing lens name", slot.slot)))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(CliError::from)?;
    let card_names = card
        .lenses
        .iter()
        .map(|lens| (lens.slot, lens.name.clone()))
        .collect::<BTreeMap<_, _>>();
    if card_names != plan_names {
        return Err(stale(format!(
            "A37 admission card roster {:?} != plan roster {:?}",
            card_names, plan_names
        )));
    }
    if !card.gate_passed
        || card.status != A37_DIVERSITY_GATE_PASSED
        || !card.family_span_pass
        || !card.redundancy_bound_pass
        || !card.no_collapse_pass
        || card.passing_lens_count != card.lens_count
        || card.lenses.iter().any(|lens| !lens.passed)
    {
        return Err(error(
            "CALYX_FSV_A37_ADMISSION_CARD_REFUSED",
            format!(
                "A37 admission refused: status={} gate_passed={} family_span={} redundancy={} no_collapse={} passing_lenses={}/{} weakest_lens={} min_best_marginal_bits={:.6}",
                card.status,
                card.gate_passed,
                card.family_span_pass,
                card.redundancy_bound_pass,
                card.no_collapse_pass,
                card.passing_lens_count,
                card.lens_count,
                card.weakest_lens,
                card.min_best_marginal_bits
            ),
            "pass a gate-passed multi-anchor A37 admission card for the exact plan roster",
        ));
    }
    finite("min_best_marginal_bits", card.min_best_marginal_bits)?;
    finite("max_best_marginal_bits", card.max_best_marginal_bits)?;
    finite("min_marginal_bits", card.min_marginal_bits)?;
    if card.min_marginal_bits < DEFAULT_MIN_MARGINAL_BITS {
        return Err(error(
            "CALYX_FSV_A37_ADMISSION_CARD_REFUSED",
            format!(
                "A37 admission card min_marginal_bits={:.6} below required {:.6}",
                card.min_marginal_bits, DEFAULT_MIN_MARGINAL_BITS
            ),
            "regenerate the multi-anchor card with the default A37 marginal-bit floor",
        ));
    }
    for lens in &card.lenses {
        finite("lens.best_marginal_bits", lens.best_marginal_bits)?;
        if lens.best_marginal_bits < card.min_marginal_bits {
            return Err(error(
                "CALYX_FSV_A37_ADMISSION_CARD_REFUSED",
                format!(
                    "A37 admission lens {} slot {} best_marginal_bits={:.6} below card min_marginal_bits={:.6}",
                    lens.name, lens.slot, lens.best_marginal_bits, card.min_marginal_bits
                ),
                "regenerate the multi-anchor card from source reports that meet the A37 no-collapse floor",
            ));
        }
    }
    Ok(())
}

fn report(path: &Path, bytes: &[u8], card: &MultiAnchorAdmission) -> Value {
    json!({
        "mode": "assay_multi_anchor_a37_admission",
        "path": path.display().to_string(),
        "bytes": bytes.len(),
        "sha256": hex_sha256(bytes),
        "schema_version": card.schema_version,
        "role": card.role.as_str(),
        "status": card.status.as_str(),
        "gate_passed": card.gate_passed,
        "lens_count": card.lens_count,
        "passing_lens_count": card.passing_lens_count,
        "min_marginal_bits": card.min_marginal_bits,
        "association_family_count": card.association_family_count,
        "family_span_pass": card.family_span_pass,
        "redundancy_bound_pass": card.redundancy_bound_pass,
        "no_collapse_pass": card.no_collapse_pass,
        "min_best_marginal_bits": card.min_best_marginal_bits,
        "max_best_marginal_bits": card.max_best_marginal_bits,
        "weakest_lens": card.weakest_lens.as_str(),
        "source_reports": &card.source_reports,
    })
}

fn report_db(
    readback: &a37_admission_store::A37AdmissionDbReadback,
    card: &MultiAnchorAdmission,
) -> Value {
    json!({
        "mode": "assay_multi_anchor_a37_admission_db",
        "db_readback": readback,
        "schema_version": card.schema_version,
        "role": card.role.as_str(),
        "status": card.status.as_str(),
        "gate_passed": card.gate_passed,
        "lens_count": card.lens_count,
        "passing_lens_count": card.passing_lens_count,
        "min_marginal_bits": card.min_marginal_bits,
        "association_family_count": card.association_family_count,
        "family_span_pass": card.family_span_pass,
        "redundancy_bound_pass": card.redundancy_bound_pass,
        "no_collapse_pass": card.no_collapse_pass,
        "min_best_marginal_bits": card.min_best_marginal_bits,
        "max_best_marginal_bits": card.max_best_marginal_bits,
        "weakest_lens": card.weakest_lens.as_str(),
        "source_reports": &card.source_reports,
    })
}

fn finite(field: &'static str, value: f32) -> CliResult {
    if value.is_finite() {
        Ok(())
    } else {
        Err(error(
            "CALYX_FSV_A37_ADMISSION_CARD_INVALID",
            format!("A37 admission card has non-finite {field}"),
            "regenerate the multi-anchor admission card with finite metrics",
        ))
    }
}

fn stale(message: String) -> CliError {
    error(
        "CALYX_FSV_A37_ADMISSION_CARD_STALE",
        message,
        "regenerate the multi-anchor admission card from the exact partitioned-RRF plan roster",
    )
}

fn stale_message(message: String) -> CalyxError {
    CalyxError {
        code: "CALYX_FSV_A37_ADMISSION_CARD_STALE",
        message,
        remediation: "regenerate the multi-anchor admission card from the exact partitioned-RRF plan roster",
    }
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CliError {
    CliError::from(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
#[path = "a37_admission_tests.rs"]
mod tests;
