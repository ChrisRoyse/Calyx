use std::path::Path;
use std::str::FromStr;

use calyx_aster::dedup::{DedupAction, DedupPolicy, TauStrategy, TctCosineConfig, check_dedup};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, GuardTauProfile, SlotId, SlotVector, VaultId, VaultStore, dense_cosine};
use serde_json::json;

use crate::error::CliError;

pub struct DedupReadbackArgs<'a> {
    pub vault: &'a Path,
    pub cx_id: &'a str,
    pub slot: &'a str,
    pub tau: &'a str,
    pub near_cos: &'a str,
    pub distinct_cos: &'a str,
    pub vault_id: &'a str,
    pub salt: &'a str,
}

pub fn readback_dedup_check(args: DedupReadbackArgs<'_>) -> crate::error::CliResult {
    let slot = parse_slot(args.slot)?;
    let tau = parse_cosine(args.tau, "--tau")?;
    let near_cos = parse_cosine(args.near_cos, "--near-cos")?;
    let distinct_cos = parse_cosine(args.distinct_cos, "--distinct-cos")?;
    let cx_id = CxId::from_str(args.cx_id)
        .map_err(|error| CliError::usage(format!("invalid --cx-id: {error}")))?;
    let vault_id = VaultId::from_str(args.vault_id)
        .map_err(|error| CliError::usage(format!("invalid --vault-id: {error}")))?;
    let vault = AsterVault::open(
        args.vault,
        vault_id,
        args.salt.as_bytes(),
        VaultOptions::default(),
    )?;
    let snapshot = vault.snapshot();
    let existing = vault.get(cx_id, snapshot)?;
    let source = existing
        .slots
        .get(&slot)
        .and_then(|vector| vector.as_dense())
        .ok_or_else(|| {
            CliError::runtime(format!("cx {cx_id} has no dense vector for slot {slot}"))
        })?;
    let near_vector = vector_at_cosine(source, near_cos)?;
    let distinct_vector = vector_at_cosine(source, distinct_cos)?;
    let mut near = existing.clone();
    near.cx_id = CxId::from_bytes([0xd0; 16]);
    near.slots.insert(slot, dense(near_vector.clone()));
    let mut distinct = existing.clone();
    distinct.cx_id = CxId::from_bytes([0xd1; 16]);
    distinct.slots.insert(slot, dense(distinct_vector.clone()));
    let policy = DedupPolicy::TctCosine(TctCosineConfig::new(
        vec![slot],
        TauStrategy::PerSlot(vec![(slot, tau)]),
        DedupAction::Collapse,
    )?);
    let no_profile: Option<&dyn GuardTauProfile> = None;
    let near_decision = check_dedup(&near, &vault, &policy, no_profile)?;
    let distinct_decision = check_dedup(&distinct, &vault, &policy, no_profile)?;
    let readback = json!({
        "existing": cx_id,
        "slot": slot,
        "tau": tau,
        "near": {
            "cx_id": near.cx_id,
            "target_cos": near_cos,
            "actual_cos": dense_cosine(source, &near_vector),
            "decision": near_decision,
        },
        "distinct": {
            "cx_id": distinct.cx_id,
            "target_cos": distinct_cos,
            "actual_cos": dense_cosine(source, &distinct_vector),
            "decision": distinct_decision,
        }
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback)
            .map_err(|error| CliError::runtime(format!("serialize readback: {error}")))?
    );
    Ok(())
}

fn parse_slot(value: &str) -> crate::error::CliResult<SlotId> {
    value
        .parse::<u16>()
        .map(SlotId::new)
        .map_err(|error| CliError::usage(format!("invalid --slot: {error}")))
}

fn parse_cosine(value: &str, flag: &str) -> crate::error::CliResult<f32> {
    let parsed = value
        .parse::<f32>()
        .map_err(|error| CliError::usage(format!("invalid {flag}: {error}")))?;
    if parsed.is_finite() && (-1.0..=1.0).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(CliError::usage(format!(
            "{flag} must be finite and in -1.0..=1.0"
        )))
    }
}

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

fn vector_at_cosine(source: &[f32], target: f32) -> crate::error::CliResult<Vec<f32>> {
    if source.len() < 2 {
        return Err(CliError::runtime(
            "dedup-check readback requires a dense slot with dim >= 2",
        ));
    }
    let norm = source.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= 0.0 || source.iter().any(|value| !value.is_finite()) {
        return Err(CliError::runtime(
            "source vector must be finite and non-zero",
        ));
    }
    let unit = source.iter().map(|value| value / norm).collect::<Vec<_>>();
    let basis_index = least_aligned_basis(&unit);
    let mut perpendicular = vec![0.0_f32; unit.len()];
    perpendicular[basis_index] = 1.0;
    let projection = unit[basis_index];
    for (value, unit_value) in perpendicular.iter_mut().zip(&unit) {
        *value -= projection * unit_value;
    }
    let perp_norm = perpendicular
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if !perp_norm.is_finite() || perp_norm <= 0.0 {
        return Err(CliError::runtime("could not derive perpendicular vector"));
    }
    for value in &mut perpendicular {
        *value /= perp_norm;
    }
    let side = (1.0 - target * target).max(0.0).sqrt();
    Ok(unit
        .iter()
        .zip(perpendicular)
        .map(|(unit, perp)| target * unit + side * perp)
        .collect())
}

fn least_aligned_basis(unit: &[f32]) -> usize {
    unit.iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            left.abs()
                .partial_cmp(&right.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}
