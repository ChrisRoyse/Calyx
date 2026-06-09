//! Ledger provenance writers for Ward calibration and guard verdicts.

use std::error::Error;
use std::fmt;

use calyx_core::{CalyxError, Clock, CxId, LedgerRef, Result as CalyxResult, SlotId};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, LedgerCfStore, SubjectId};
use serde_json::{Value, json};

use crate::calibrate::{CalibrationInput, calibrate};
use crate::error::WardError;
use crate::guard::{MatchedSlots, ProducedSlots, guard};
use crate::profile::{
    CalibrationMeta, GuardPolicy, GuardProfile, NoveltyAction, SlotCalibrationMeta,
};
use crate::verdict::{GuardVerdict, SlotVerdict};

const ACTOR: &str = "calyx-ward";
const CALIBRATION_TAG: &str = "ward_calibration_v1";
const VERDICT_TAG: &str = "ward_guard_verdict_v1";

#[derive(Debug)]
pub enum WardLedgerError {
    Ward(WardError),
    Ledger(CalyxError),
}

impl fmt::Display for WardLedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ward(error) => fmt::Display::fmt(error, f),
            Self::Ledger(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl Error for WardLedgerError {}

impl From<WardError> for WardLedgerError {
    fn from(error: WardError) -> Self {
        Self::Ward(error)
    }
}

impl From<CalyxError> for WardLedgerError {
    fn from(error: CalyxError) -> Self {
        Self::Ledger(error)
    }
}

pub type WardLedgerResult<T> = std::result::Result<T, WardLedgerError>;

pub fn calibrate_with_ledger<S, C>(
    appender: &mut LedgerAppender<S, C>,
    profile_template: GuardProfile,
    inputs: Vec<CalibrationInput>,
    alpha: f32,
    clock: &dyn Clock,
) -> WardLedgerResult<(GuardProfile, LedgerRef)>
where
    S: LedgerCfStore,
    C: Clock,
{
    let profile = calibrate(profile_template, inputs, alpha, clock)?;
    let ledger_ref = append_calibration_provenance(appender, &profile)?;
    Ok((profile, ledger_ref))
}

pub fn guard_with_ledger<S, C>(
    appender: &mut LedgerAppender<S, C>,
    cx_id: CxId,
    profile: &GuardProfile,
    produced: &ProducedSlots,
    matched: &MatchedSlots,
    high_stakes: bool,
) -> WardLedgerResult<(GuardVerdict, LedgerRef)>
where
    S: LedgerCfStore,
    C: Clock,
{
    let verdict = guard(profile, produced, matched, high_stakes)?;
    let ledger_ref = append_guard_verdict(appender, cx_id, &verdict)?;
    Ok((verdict, ledger_ref))
}

pub fn append_calibration_provenance<S, C>(
    appender: &mut LedgerAppender<S, C>,
    profile: &GuardProfile,
) -> CalyxResult<LedgerRef>
where
    S: LedgerCfStore,
    C: Clock,
{
    let calibration = profile.calibration.as_ref().ok_or_else(|| {
        CalyxError::ledger_corrupt("Ward calibration provenance requires calibrated profile")
    })?;
    let payload =
        serde_json::to_vec(&calibration_payload(profile, calibration)).map_err(|error| {
            CalyxError::ledger_corrupt(format!("encode Ward calibration payload: {error}"))
        })?;
    appender.append(
        EntryKind::Guard,
        SubjectId::Guard(guard_subject(profile)),
        payload,
        ActorId::Service(ACTOR.to_string()),
    )
}

pub fn append_guard_verdict<S, C>(
    appender: &mut LedgerAppender<S, C>,
    cx_id: CxId,
    verdict: &GuardVerdict,
) -> CalyxResult<LedgerRef>
where
    S: LedgerCfStore,
    C: Clock,
{
    let payload = serde_json::to_vec(&verdict_payload(cx_id, verdict)).map_err(|error| {
        CalyxError::ledger_corrupt(format!("encode Ward guard payload: {error}"))
    })?;
    appender.append(
        EntryKind::Guard,
        SubjectId::Cx(cx_id),
        payload,
        ActorId::Service(ACTOR.to_string()),
    )
}

fn calibration_payload(profile: &GuardProfile, calibration: &CalibrationMeta) -> Value {
    json!({
        "ward_provenance": CALIBRATION_TAG,
        "guard_id": profile.guard_id.to_string(),
        "panel_version": profile.panel_version,
        "policy": policy_payload(&profile.policy),
        "required_slots": slots_payload(&profile.required_slots),
        "tau": tau_payload(profile),
        "calibration": calibration_meta_payload(calibration),
    })
}

fn verdict_payload(cx_id: CxId, verdict: &GuardVerdict) -> Value {
    json!({
        "ward_provenance": VERDICT_TAG,
        "cx_id": cx_id.to_string(),
        "guard_id": verdict.guard_id.to_string(),
        "overall_pass": verdict.overall_pass,
        "provisional": verdict.provisional,
        "action": action_payload(verdict.action.as_ref()),
        "per_slot": verdict.per_slot.iter().map(slot_verdict_payload).collect::<Vec<_>>(),
    })
}

fn calibration_meta_payload(meta: &CalibrationMeta) -> Value {
    json!({
        "corpus_hash": hex(&meta.corpus_hash),
        "estimator": meta.estimator,
        "far": meta.far,
        "frr": meta.frr,
        "confidence": meta.confidence,
        "ts": meta.ts,
        "per_slot": meta.per_slot.iter().map(|(slot, meta)| {
            slot_calibration_payload(*slot, meta)
        }).collect::<Vec<_>>(),
    })
}

fn slot_calibration_payload(slot: SlotId, meta: &SlotCalibrationMeta) -> Value {
    json!({
        "slot": slot.get(),
        "corpus_hash": hex(&meta.corpus_hash),
        "estimator": meta.estimator,
        "far": meta.far,
        "frr": meta.frr,
        "confidence": meta.confidence,
        "ts": meta.ts,
    })
}

fn slot_verdict_payload(verdict: &SlotVerdict) -> Value {
    json!({
        "slot": verdict.slot.get(),
        "cos": verdict.cos,
        "tau": verdict.tau,
        "pass": verdict.pass,
    })
}

fn tau_payload(profile: &GuardProfile) -> Vec<Value> {
    profile
        .tau
        .iter()
        .map(|(slot, tau)| json!({"slot": slot.get(), "tau": tau}))
        .collect()
}

fn slots_payload(slots: &[SlotId]) -> Vec<u16> {
    slots.iter().map(|slot| slot.get()).collect()
}

fn policy_payload(policy: &GuardPolicy) -> Value {
    match policy {
        GuardPolicy::AllRequired => json!({"type": "all_required"}),
        GuardPolicy::KofN { k } => json!({"type": "k_of_n", "k": k}),
    }
}

fn action_payload(action: Option<&NoveltyAction>) -> Option<&'static str> {
    action.map(|action| match action {
        NoveltyAction::NewRegion => "new_region",
        NoveltyAction::Quarantine => "quarantine",
        NoveltyAction::RejectClosed => "reject_closed",
    })
}

fn guard_subject(profile: &GuardProfile) -> Vec<u8> {
    profile.guard_id.to_string().into_bytes()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
