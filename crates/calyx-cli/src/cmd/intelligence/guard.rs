use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use calyx_aster::cf::ColumnFamily;
use calyx_core::{CalyxError, Constellation, CxId, SlotId};
use calyx_ward::{
    CalibrationInput, GuardId, GuardPolicy, GuardProfile, NoveltyAction, SlotKind, WardError,
    calibrate, guard,
};
use serde::Deserialize;

use super::core::{
    dense, dense_dim, load_context, load_docs, parse_cx_id, read_json_row, text_vector,
    write_json_row,
};
use super::model::{
    GuardCheckOut, GuardProfileOut, SlotTauOut, default_guard_key, guard_profile_key,
};
use super::parse::{GuardArgs, GuardCommand};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[derive(Default)]
pub(super) struct SlotScores {
    pub(super) good: Vec<f32>,
    pub(super) bad: Vec<f32>,
}

#[derive(Deserialize)]
struct CalibrationLine {
    slot: Option<u16>,
    score: Option<f32>,
    good: Option<bool>,
    class: Option<String>,
    label: Option<String>,
    kind: Option<String>,
}

pub(super) fn command(args: GuardArgs) -> CliResult {
    match args.command {
        GuardCommand::Calibrate {
            domain,
            set,
            target_far,
        } => calibrate_command(&args.vault, &domain, &set, target_far),
        GuardCommand::Check { cx_id, identity_cx } => {
            check_command(&args.vault, &cx_id, identity_cx.as_deref())
        }
        GuardCommand::Generate {
            candidate_text,
            identity_cx,
        } => generate_command(&args.vault, &candidate_text, identity_cx.as_deref()),
    }
}

fn calibrate_command(vault_name: &str, domain: &str, set: &Path, target_far: f32) -> CliResult {
    let ctx = load_context(vault_name)?;
    let scores = read_calibration_set(set)?;
    let inputs = scores
        .into_iter()
        .map(|(slot, scores)| CalibrationInput {
            slot,
            good_scores: scores.good,
            bad_scores: scores.bad,
            slot_kind: slot_kind(target_far),
            target_far,
        })
        .collect::<Vec<_>>();
    let corpus_size = inputs
        .iter()
        .map(|input| input.good_scores.len() + input.bad_scores.len())
        .sum::<usize>();
    let template = GuardProfile {
        guard_id: guard_id()?,
        panel_version: u64::from(ctx.state.panel.version),
        domain: domain.to_string(),
        tau: BTreeMap::new(),
        required_slots: Vec::new(),
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::RejectClosed,
    };
    let clock = calyx_core::SystemClock;
    let profile = calibrate(template, inputs, 0.01, &clock).map_err(ward_to_cli)?;
    write_json_row(
        &ctx.vault,
        ColumnFamily::Guard,
        guard_profile_key(domain),
        &profile,
    )?;
    write_json_row(
        &ctx.vault,
        ColumnFamily::Guard,
        default_guard_key(),
        &profile,
    )?;
    print_json(&profile_out(&profile, corpus_size)?)
}

fn check_command(vault_name: &str, cx_id: &str, identity_cx: Option<&str>) -> CliResult {
    let ctx = load_context(vault_name)?;
    let profile = load_profile(&ctx.vault)?;
    let docs = load_docs(&ctx.vault)?;
    let cx = parse_cx_id(cx_id, "--cx")?;
    let identity = identity_cx
        .map(|raw| parse_cx_id(raw, "--identity-cx"))
        .transpose()?
        .unwrap_or(cx);
    let produced = required_vectors(&docs, cx, &profile)?;
    let matched = required_vectors(&docs, identity, &profile)?;
    let verdict = guard(&profile, &produced, &matched, true).map_err(ward_to_cli)?;
    print_json(&check_out(&verdict.per_slot, verdict.overall_pass))
}

fn generate_command(vault_name: &str, text: &str, identity_cx: Option<&str>) -> CliResult {
    let ctx = load_context(vault_name)?;
    let profile = load_profile(&ctx.vault)?;
    let docs = load_docs(&ctx.vault)?;
    let identity =
        match identity_cx {
            Some(raw) => parse_cx_id(raw, "--identity-cx")?,
            None => docs.keys().next().copied().ok_or_else(|| {
                CalyxError::vault_access_denied("guard generate needs identity cx")
            })?,
        };
    let matched = required_vectors(&docs, identity, &profile)?;
    let produced = generated_vectors(text, docs.get(&identity), &profile)?;
    let verdict = guard(&profile, &produced, &matched, true).map_err(ward_to_cli)?;
    print_json(&check_out(&verdict.per_slot, verdict.overall_pass))
}

pub(super) fn read_calibration_set(path: &Path) -> CliResult<BTreeMap<SlotId, SlotScores>> {
    let text = fs::read_to_string(path).map_err(|error| {
        CliError::io(format!(
            "read guard calibration set {}: {error}",
            path.display()
        ))
    })?;
    let mut scores = BTreeMap::<SlotId, SlotScores>::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: CalibrationLine = serde_json::from_str(line).map_err(|error| {
            CliError::usage(format!("parse calibration JSONL line {}: {error}", idx + 1))
        })?;
        let score = row.score.ok_or_else(|| {
            CliError::usage(format!("calibration line {} missing score", idx + 1))
        })?;
        let slot = SlotId::new(row.slot.unwrap_or(0));
        let entry = scores.entry(slot).or_default();
        if row_is_good(&row)? {
            entry.good.push(score);
        } else {
            entry.bad.push(score);
        }
    }
    if scores.is_empty() {
        return Err(CliError::usage("guard calibration set is empty"));
    }
    Ok(scores)
}

fn row_is_good(row: &CalibrationLine) -> CliResult<bool> {
    if let Some(value) = row.good {
        return Ok(value);
    }
    let label = row
        .class
        .as_ref()
        .or(row.label.as_ref())
        .or(row.kind.as_ref())
        .ok_or_else(|| CliError::usage("calibration row requires good or class/label/kind"))?
        .to_ascii_lowercase();
    match label.as_str() {
        "good" | "pass" | "match" | "identity" | "clean" => Ok(true),
        "bad" | "fail" | "ood" | "injection" | "attack" | "reject" => Ok(false),
        other => Err(CliError::usage(format!(
            "unknown calibration class {other}; expected good or injection"
        ))),
    }
}

fn load_profile(vault: &calyx_aster::vault::AsterVault) -> CliResult<GuardProfile> {
    read_json_row(vault, ColumnFamily::Guard, &default_guard_key())?.ok_or_else(|| {
        CalyxError::guard_provisional("guard check requires a calibrated guard profile").into()
    })
}

fn required_vectors(
    docs: &BTreeMap<CxId, Constellation>,
    cx_id: CxId,
    profile: &GuardProfile,
) -> CliResult<BTreeMap<SlotId, Vec<f32>>> {
    let cx = docs.get(&cx_id).ok_or_else(|| {
        CalyxError::vault_access_denied(format!("constellation {cx_id} not found"))
    })?;
    let mut out = BTreeMap::new();
    for slot in &profile.required_slots {
        let values = dense(cx, *slot).ok_or_else(|| {
            CalyxError::stale_derived(format!("constellation {cx_id} lacks dense slot {slot}"))
        })?;
        out.insert(*slot, values.to_vec());
    }
    Ok(out)
}

fn generated_vectors(
    text: &str,
    identity: Option<&Constellation>,
    profile: &GuardProfile,
) -> CliResult<BTreeMap<SlotId, Vec<f32>>> {
    let identity = identity
        .ok_or_else(|| CalyxError::vault_access_denied("identity constellation not found"))?;
    let mut out = BTreeMap::new();
    for slot in &profile.required_slots {
        let dim = dense_dim(identity, &[*slot]).ok_or_else(|| {
            CalyxError::stale_derived(format!("identity lacks dense slot {slot}"))
        })?;
        out.insert(*slot, text_vector(text, dim));
    }
    Ok(out)
}

fn profile_out(profile: &GuardProfile, corpus_size: usize) -> CliResult<GuardProfileOut> {
    let calibration = profile.calibration.as_ref().ok_or_else(|| {
        CalyxError::guard_provisional("calibrated profile lacks calibration meta")
    })?;
    let tau = max_tau(profile);
    Ok(GuardProfileOut {
        domain: profile.domain.clone(),
        tau,
        far: calibration.far,
        frr: calibration.frr,
        calibration_corpus_size: corpus_size,
        blocked_injection_rate: 1.0 - calibration.far,
        per_slot_tau: profile
            .tau
            .iter()
            .map(|(slot, tau)| SlotTauOut {
                slot: slot.get(),
                tau: *tau,
            })
            .collect(),
    })
}

fn check_out(slots: &[calyx_ward::SlotVerdict], pass: bool) -> GuardCheckOut {
    let tau_cos = slots.iter().map(|slot| slot.tau).fold(0.0_f32, f32::max);
    let min_cos = slots.iter().map(|slot| slot.cos).fold(1.0_f32, f32::min);
    GuardCheckOut {
        verdict: if pass { "pass" } else { "ood" },
        tau: 1.0 - tau_cos,
        distance: 1.0 - min_cos,
    }
}

fn max_tau(profile: &GuardProfile) -> f32 {
    profile.tau.values().copied().fold(0.0, f32::max)
}

fn slot_kind(target_far: f32) -> SlotKind {
    if target_far <= SlotKind::Identity.default_target_far() {
        SlotKind::Identity
    } else if target_far <= SlotKind::Content.default_target_far() {
        SlotKind::Content
    } else {
        SlotKind::Stylistic
    }
}

fn guard_id() -> CliResult<GuardId> {
    GuardId::from_str(GUARD_UUID)
        .map_err(|error| CliError::usage(format!("parse fixed guard id: {error}")))
}

fn ward_to_cli(error: WardError) -> CliError {
    let code = error.code();
    let text = error.to_string();
    let message = text
        .strip_prefix(code)
        .and_then(|rest| rest.strip_prefix(": "))
        .unwrap_or(&text)
        .to_string();
    CliError::from(CalyxError {
        code,
        message,
        remediation: match code {
            "CALYX_GUARD_PROVISIONAL" => "calibrate before high-stakes use",
            "CALYX_GUARD_OOD" => "new-region or reject per policy",
            _ => "inspect guard calibration inputs and required slots",
        },
    })
}
