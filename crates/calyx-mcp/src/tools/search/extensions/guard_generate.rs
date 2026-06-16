use std::collections::BTreeMap;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Constellation, CxId, SlotVector, VaultStore};
use calyx_ward::{GuardProfile, MatchedSlots, ProducedSlots, WardError, guard};
use serde_json::{Value, json};

use crate::server::{ToolError, ToolResult};

use super::runtime::{NavRuntime, parse_cx_id};

const DEFAULT_GUARD_KEY: &[u8] = b"profile\0default";

pub(super) fn run(
    runtime: &NavRuntime,
    candidate_text: &str,
    identity_cx: Option<&str>,
) -> ToolResult<Value> {
    let profile = load_guard_profile(&runtime.vault)?;
    let identity =
        match identity_cx {
            Some(raw) => parse_cx_id(raw)?,
            None => runtime.docs.keys().next().copied().ok_or_else(|| {
                CalyxError::vault_access_denied("guard generate needs identity cx")
            })?,
        };
    let matched = required_vectors(&runtime.docs, identity, &profile)?;
    let produced = generated_vectors(&runtime.docs, identity, candidate_text, &profile)?;
    let verdict = guard(&profile, &produced, &matched, true).map_err(ward_to_tool)?;
    let min_cos = verdict
        .per_slot
        .iter()
        .map(|slot| slot.cos)
        .fold(1.0_f32, f32::min);
    let max_tau = verdict
        .per_slot
        .iter()
        .map(|slot| slot.tau)
        .fold(0.0_f32, f32::max);
    Ok(json!({
        "verdict": if verdict.overall_pass { "pass" } else { "ood" },
        "tau": 1.0 - max_tau,
        "distance": 1.0 - min_cos,
        "identity_cx": identity.to_string(),
    }))
}

fn load_guard_profile(vault: &AsterVault) -> ToolResult<GuardProfile> {
    let Some(bytes) = vault.read_cf_at(vault.snapshot(), ColumnFamily::Guard, DEFAULT_GUARD_KEY)?
    else {
        return Err(CalyxError::guard_provisional(
            "guard generate requires a calibrated guard profile",
        )
        .into());
    };
    let profile: GuardProfile = serde_json::from_slice(&bytes)
        .map_err(|err| CalyxError::aster_corrupt_shard(format!("decode guard profile: {err}")))?;
    if !profile.is_calibrated() {
        return Err(CalyxError::guard_provisional(
            "guard generate requires a calibrated guard profile",
        )
        .into());
    }
    Ok(profile)
}

fn required_vectors(
    docs: &BTreeMap<CxId, Constellation>,
    cx_id: CxId,
    profile: &GuardProfile,
) -> ToolResult<MatchedSlots> {
    let cx = docs.get(&cx_id).ok_or_else(|| {
        CalyxError::vault_access_denied(format!("constellation {cx_id} not found"))
    })?;
    let mut out = BTreeMap::new();
    for slot in &profile.required_slots {
        let values = cx
            .slots
            .get(slot)
            .and_then(SlotVector::as_dense)
            .ok_or_else(|| {
                CalyxError::stale_derived(format!("constellation {cx_id} lacks dense slot {slot}"))
            })?;
        out.insert(*slot, values.to_vec());
    }
    Ok(out)
}

fn generated_vectors(
    docs: &BTreeMap<CxId, Constellation>,
    identity: CxId,
    text: &str,
    profile: &GuardProfile,
) -> ToolResult<ProducedSlots> {
    let cx = docs
        .get(&identity)
        .ok_or_else(|| CalyxError::vault_access_denied("identity constellation not found"))?;
    let mut out = BTreeMap::new();
    for slot in &profile.required_slots {
        let dim = cx
            .slots
            .get(slot)
            .and_then(SlotVector::as_dense)
            .map(|values| values.len())
            .ok_or_else(|| {
                CalyxError::stale_derived(format!("identity lacks dense slot {slot}"))
            })?;
        out.insert(*slot, text_vector(text, dim));
    }
    Ok(out)
}

fn text_vector(text: &str, dim: usize) -> Vec<f32> {
    let dim = dim.max(1);
    let mut out = Vec::with_capacity(dim);
    for idx in 0..dim {
        let idx = idx.to_le_bytes();
        let digest = calyx_core::content_address([text.as_bytes(), idx.as_slice()]);
        let raw = u32::from_le_bytes(digest[0..4].try_into().expect("hash slice"));
        let unit = raw as f32 / u32::MAX as f32;
        out.push(unit * 2.0 - 1.0);
    }
    normalize(&mut out);
    out
}

fn normalize(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in values {
            *value /= norm;
        }
    }
}

fn ward_to_tool(error: WardError) -> ToolError {
    let code = error.code();
    let text = error.to_string();
    let message = text
        .strip_prefix(code)
        .and_then(|rest| rest.strip_prefix(": "))
        .unwrap_or(&text)
        .to_string();
    CalyxError {
        code,
        message,
        remediation: match code {
            "CALYX_GUARD_PROVISIONAL" => "calibrate before high-stakes use",
            "CALYX_GUARD_OOD" => "new-region or reject per policy",
            _ => "inspect guard calibration inputs and required slots",
        },
    }
    .into()
}
