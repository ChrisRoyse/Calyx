use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use calyx_assay::{EnsembleCard, validate_ensemble_card_redundancy};
use calyx_core::CalyxError;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::Plan;
use crate::error::{CliError, CliResult};

#[path = "ensemble_a37_gate.rs"]
mod a37_gate;
const MIN_LENSES: usize = 10;

pub(super) fn load(path: Option<&Path>, plan: &Plan, required: bool) -> CliResult<Option<Value>> {
    let Some(path) = path else {
        return if required {
            Err(error(
                "CALYX_FSV_A35_ENSEMBLE_CARD_REQUIRED",
                "partitioned-rrf recall/SLO gates require --ensemble-card",
                "run assay ensemble-card for the exact >=10-lens panel and pass the persisted card",
            ))
        } else {
            Ok(None)
        };
    };
    let bytes = std::fs::read(path).map_err(|err| {
        error(
            "CALYX_FSV_A35_ENSEMBLE_CARD_IO",
            format!("read ensemble card {} failed: {err}", path.display()),
            "pass a byte-readable ensemble_card.json produced by assay ensemble-card",
        )
    })?;
    let card: EnsembleCard = serde_json::from_slice(&bytes).map_err(|err| {
        error(
            "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID",
            format!("parse ensemble card {} failed: {err}", path.display()),
            "regenerate the ensemble card from the same panel corpus",
        )
    })?;
    validate(&card, plan, required)?;
    Ok(Some(report(path, &bytes, &card)))
}
fn validate(card: &EnsembleCard, plan: &Plan, required: bool) -> CliResult {
    let expected = plan.slots.len();
    if card.panel_lens_count < MIN_LENSES || card.lenses.len() < MIN_LENSES {
        return Err(error(
            "CALYX_FSV_A35_ENSEMBLE_CARD_TOO_SMALL",
            format!(
                "ensemble card has panel_lens_count={} lenses={}; A35 requires at least {MIN_LENSES}",
                card.panel_lens_count,
                card.lenses.len()
            ),
            "run assay ensemble-card with a real >=10-lens content panel",
        ));
    }
    if card.panel_lens_count != expected || card.lenses.len() != expected {
        return Err(stale(format!(
            "ensemble card lens count {} / {} != plan slots {expected}",
            card.panel_lens_count,
            card.lenses.len()
        )));
    }
    let plan_slots = plan
        .slots
        .iter()
        .map(|slot| slot.slot)
        .collect::<BTreeSet<_>>();
    let card_slots = card
        .lenses
        .iter()
        .map(|lens| lens.slot.get())
        .collect::<BTreeSet<_>>();
    if card_slots != plan_slots {
        return Err(stale(format!(
            "ensemble card slots {:?} != plan slots {:?}",
            card_slots, plan_slots
        )));
    }
    let plan_names = plan_slot_names(plan)?;
    let card_names = card
        .lenses
        .iter()
        .map(|lens| (lens.slot.get(), lens.name.clone()))
        .collect::<BTreeMap<_, _>>();
    if card_names != plan_names {
        return Err(stale(format!(
            "ensemble card slot/name roster {:?} != plan roster {:?}",
            card_names, plan_names
        )));
    }
    let expected_pairs = expected.saturating_sub(1) * expected / 2;
    if card.pairs.len() != expected_pairs {
        return Err(error(
            "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID",
            format!(
                "ensemble card pairs {} != expected {expected_pairs}",
                card.pairs.len()
            ),
            "regenerate ensemble_card.json so every pairwise cross-term is present",
        ));
    }
    validate_pair_names(card, &card_names)?;
    validate_ensemble_card_redundancy(card).map_err(|err| {
        error(
            "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID",
            format!("ensemble card redundancy evidence is invalid: {err}"),
            "regenerate ensemble_card.json with current Assay redundancy evidence",
        )
    })?;
    finite(card)?;
    a37_gate::validate(&card.a37_diversity, required)?;
    Ok(())
}
fn plan_slot_names(plan: &Plan) -> CliResult<BTreeMap<u16, String>> {
    let mut names = BTreeMap::new();
    for slot in &plan.slots {
        let Some(name) = slot.name.as_deref().filter(|name| !name.trim().is_empty()) else {
            return Err(error(
                "CALYX_FSV_A35_PLAN_NAME_REQUIRED",
                format!(
                    "slot {} missing lens name for ensemble-card binding",
                    slot.slot
                ),
                "re-export partitioned_rrf_plan.json with the current assay export-fbin or stream-fbin writer",
            ));
        };
        names.insert(slot.slot, name.to_owned());
    }
    Ok(names)
}
fn validate_pair_names(card: &EnsembleCard, card_names: &BTreeMap<u16, String>) -> CliResult {
    let mut pairs = BTreeSet::new();
    for pair in &card.pairs {
        let slot_a = pair.slot_a.get();
        let slot_b = pair.slot_b.get();
        let Some(name_a) = card_names.get(&slot_a) else {
            return Err(error(
                "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID",
                format!("pair references missing slot_a {slot_a}"),
                "regenerate ensemble_card.json so every pair references a panel lens",
            ));
        };
        let Some(name_b) = card_names.get(&slot_b) else {
            return Err(error(
                "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID",
                format!("pair references missing slot_b {slot_b}"),
                "regenerate ensemble_card.json so every pair references a panel lens",
            ));
        };
        if pair.a != *name_a || pair.b != *name_b {
            return Err(error(
                "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID",
                format!(
                    "pair ({slot_a}, {slot_b}) names ({}, {}) do not match lens roster ({name_a}, {name_b})",
                    pair.a, pair.b
                ),
                "regenerate ensemble_card.json so pair cross-terms use the exact panel roster",
            ));
        }
        if !pairs.insert((slot_a, slot_b)) {
            return Err(error(
                "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID",
                format!("duplicate pair ({slot_a}, {slot_b}) in ensemble card"),
                "regenerate ensemble_card.json so every pairwise cross-term is unique",
            ));
        }
    }
    let slots = card_names.keys().copied().collect::<Vec<_>>();
    let mut expected_pairs = BTreeSet::new();
    for (idx, slot_a) in slots.iter().enumerate() {
        for slot_b in slots.iter().skip(idx + 1) {
            expected_pairs.insert((*slot_a, *slot_b));
        }
    }
    if pairs != expected_pairs {
        return Err(error(
            "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID",
            format!(
                "ensemble card pair slots {:?} != expected {:?}",
                pairs, expected_pairs
            ),
            "regenerate ensemble_card.json so every canonical pairwise cross-term is present",
        ));
    }
    Ok(())
}

fn report(path: &Path, bytes: &[u8], card: &EnsembleCard) -> Value {
    json!({
        "mode": "assay_ensemble_card",
        "card_path": path,
        "card_sha256": sha256_hex(bytes),
        "schema_version": card.schema_version,
        "source": card.source,
        "pid_method": card.pid_method,
        "panel_lens_count": card.panel_lens_count,
        "n_samples": card.n_samples,
        "anchor_entropy_bits": card.anchor_entropy_bits,
        "panel_bits": card.panel_bits,
        "panel_ci": card.panel_ci,
        "n_eff": card.n_eff,
        "a37_diversity": card.a37_diversity,
        "redundancy_method": card.redundancy_method,
        "panel_sufficiency": card.sufficiency,
        "sufficient": card.sufficient,
        "deficit_bits": card.deficit_bits,
        "deficit_proposal": card.deficit_proposal,
        "keep_count": card.keep_count,
        "park_count": card.park_count,
        "retire_count": card.retire_count,
        "lens_values": card.lenses.iter().map(|lens| json!({
            "slot": lens.slot,
            "name": lens.name,
            "solo_bits": lens.solo_bits,
            "marginal_bits": lens.marginal_bits,
            "pid_unique_bits": lens.pid.unique_bits,
            "pid_redundant_bits": lens.pid.redundant_bits,
            "pid_synergistic_bits": lens.pid.synergistic_bits,
            "max_pairwise_corr": lens.max_pairwise_corr,
            "max_pairwise_nmi": lens.max_pairwise_nmi,
            "decision": lens.decision,
        })).collect::<Vec<_>>(),
        "pair_values": card.pairs.iter().map(|pair| json!({
            "slot_a": pair.slot_a,
            "slot_b": pair.slot_b,
            "a": pair.a,
            "b": pair.b,
            "corr": pair.corr,
            "nmi": pair.nmi,
            "redundancy": pair.redundancy,
            "pair_bits": pair.pair_bits,
            "synergy_gain_bits": pair.synergy_gain_bits,
        })).collect::<Vec<_>>(),
    })
}

fn finite(card: &EnsembleCard) -> CliResult {
    for (name, value) in [
        ("anchor_entropy_bits", card.anchor_entropy_bits),
        ("panel_bits", card.panel_bits),
        ("panel_ci_low", card.panel_ci[0]),
        ("panel_ci_high", card.panel_ci[1]),
        ("n_eff", card.n_eff),
        ("deficit_bits", card.deficit_bits),
    ] {
        ensure_finite(name, value)?;
    }
    for lens in &card.lenses {
        ensure_finite("lens.solo_bits", lens.solo_bits)?;
        ensure_finite("lens.marginal_bits", lens.marginal_bits)?;
        ensure_finite("lens.pid.unique_bits", lens.pid.unique_bits)?;
        ensure_finite("lens.pid.redundant_bits", lens.pid.redundant_bits)?;
        ensure_finite("lens.pid.synergistic_bits", lens.pid.synergistic_bits)?;
    }
    for pair in &card.pairs {
        ensure_finite("pair.corr", pair.corr)?;
        ensure_finite("pair.nmi", pair.nmi)?;
        ensure_finite("pair.pair_bits", pair.pair_bits)?;
        ensure_finite("pair.synergy_gain_bits", pair.synergy_gain_bits)?;
    }
    Ok(())
}

fn ensure_finite(name: &'static str, value: f32) -> CliResult {
    if value.is_finite() {
        Ok(())
    } else {
        Err(error(
            "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID",
            format!("ensemble card contains non-finite {name}={value}"),
            "regenerate the ensemble card and inspect the Assay metrics",
        ))
    }
}

fn stale(message: String) -> CliError {
    error(
        "CALYX_FSV_A35_ENSEMBLE_CARD_STALE",
        message,
        "pass an ensemble card generated from the same RRF panel slots",
    )
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "ensemble/tests.rs"]
mod tests;
