use std::collections::BTreeSet;
use std::path::Path;

use calyx_assay::EnsembleCard;
use calyx_core::CalyxError;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::Plan;
use crate::error::{CliError, CliResult};

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
    validate(&card, plan)?;
    Ok(Some(report(path, &bytes, &card)))
}

fn validate(card: &EnsembleCard, plan: &Plan) -> CliResult {
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
    finite(card)?;
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
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use calyx_assay::sufficiency::PanelSufficiency;
    use calyx_assay::{EnsembleDecision, EnsembleLensValue, EnsemblePairValue, PidBits, TrustTag};
    use calyx_core::SlotId;

    use super::super::PlanSlot;
    use super::*;

    #[test]
    fn recall_gate_requires_ensemble_card() {
        let err = load(None, &plan(10), true).unwrap_err();

        assert_eq!(err.code(), "CALYX_FSV_A35_ENSEMBLE_CARD_REQUIRED");
    }

    #[test]
    fn valid_ten_lens_card_reports_decomposition() {
        let root = temp_root("rrf-ensemble-card");
        let path = root.join("ensemble_card.json");
        std::fs::write(&path, serde_json::to_vec(&card(10, 0)).unwrap()).unwrap();

        let report = load(Some(&path), &plan(10), true).unwrap().unwrap();

        assert_eq!(report["panel_lens_count"], 10);
        assert_eq!(report["n_eff"], 8.5);
        assert_eq!(report["card_sha256"].as_str().unwrap().len(), 64);
        assert_eq!(report["lens_values"].as_array().unwrap().len(), 10);
        assert_eq!(report["pair_values"].as_array().unwrap().len(), 45);
        let synergy = report["pair_values"][0]["synergy_gain_bits"]
            .as_f64()
            .unwrap();
        assert!((synergy - 0.02).abs() < 1.0e-6);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn stale_card_slot_set_fails_closed() {
        let root = temp_root("rrf-ensemble-stale");
        let path = root.join("ensemble_card.json");
        std::fs::write(&path, serde_json::to_vec(&card(10, 1)).unwrap()).unwrap();

        let err = load(Some(&path), &plan(10), true).unwrap_err();

        assert_eq!(err.code(), "CALYX_FSV_A35_ENSEMBLE_CARD_STALE");
        let _ = std::fs::remove_dir_all(root);
    }

    fn plan(count: u16) -> Plan {
        Plan {
            timeline: None,
            slots: (0..count).map(plan_slot).collect(),
        }
    }

    fn plan_slot(slot: u16) -> PlanSlot {
        PlanSlot {
            slot,
            lens_id: Some(format!("{:032x}", slot + 1)),
            weights_sha256: Some(format!("{:064x}", slot + 1)),
            bits_about: Some(0.1),
            vault: PathBuf::from(format!("vault-{slot}")),
            queries: PathBuf::from(format!("queries-{slot}.i8bin")),
            corpus: PathBuf::from(format!("corpus-{slot}.i8bin")),
        }
    }

    fn card(count: u16, slot_offset: u16) -> EnsembleCard {
        let lenses = (0..count)
            .map(|idx| lens_value(idx + slot_offset))
            .collect::<Vec<_>>();
        let mut pairs = Vec::new();
        for a in 0..count {
            for b in (a + 1)..count {
                pairs.push(pair_value(a + slot_offset, b + slot_offset));
            }
        }
        EnsembleCard {
            schema_version: 1,
            source: "unit-test".to_string(),
            pid_method: "bounded_decision_surrogate_v1".to_string(),
            panel_lens_count: count as usize,
            n_samples: 64,
            anchor_entropy_bits: 1.0,
            panel_bits: 0.5,
            panel_ci: [0.4, 0.6],
            n_eff: 8.5,
            sufficient: false,
            deficit_bits: 0.5,
            deficit_proposal: None,
            sufficiency: PanelSufficiency {
                panel_bits: 0.5,
                anchor_entropy_bits: 1.0,
                observation_scope: None,
                sufficient: false,
                deficit_bits: 0.5,
                deficits: Vec::new(),
                trust: TrustTag::Provisional,
            },
            lenses,
            pairs,
            keep_count: count as usize,
            park_count: 0,
            retire_count: 0,
        }
    }

    fn lens_value(slot: u16) -> EnsembleLensValue {
        EnsembleLensValue {
            name: format!("lens-{slot}"),
            slot: SlotId::new(slot),
            solo_bits: 0.2,
            solo_ci: [0.1, 0.3],
            panel_without_bits: 0.45,
            marginal_bits: 0.05,
            marginal_ci: [0.02, 0.08],
            pid: PidBits {
                unique_bits: 0.05,
                redundant_bits: 0.15,
                synergistic_bits: 0.02,
            },
            max_pairwise_corr: 0.4,
            max_pairwise_nmi: 0.3,
            decision: EnsembleDecision::Keep,
            decision_reason: "unit test".to_string(),
        }
    }

    fn pair_value(a: u16, b: u16) -> EnsemblePairValue {
        EnsemblePairValue {
            a: format!("lens-{a}"),
            b: format!("lens-{b}"),
            slot_a: SlotId::new(a),
            slot_b: SlotId::new(b),
            corr: 0.2,
            nmi: 0.1,
            pair_bits: 0.3,
            pair_ci: [0.2, 0.4],
            synergy_gain_bits: 0.02,
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
