use calyx_core::{CalyxError, LedgerRef, Result};
use calyx_ledger::LedgerCfStore;
use serde::{Deserialize, Serialize};

use crate::{
    AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry, CALYX_ANNEAL_LEDGER_INVALID_ENTRY,
    CALYX_LEDGER_WRITE_FAIL, ChangeId, LogicalTime, MetricSnapshot,
};

use super::{
    CALYX_ASSAY_INVALID_METRIC, GateOutcome, ProposalOutcome, ProposalTerminalState, RejectReason,
    describe,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensAdmittedEntry {
    pub candidate_desc: String,
    pub bits_gain: f64,
    pub max_corr: f64,
    pub sufficiency_before: f64,
    pub sufficiency_after: f64,
    pub change_id: ChangeId,
    pub ts: LogicalTime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensRejectedEntry {
    pub candidate_desc: String,
    pub reason: RejectReason,
    pub deficit_gap: f64,
    pub ts: LogicalTime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "entry")]
pub enum AdmissionRecord {
    #[serde(rename = "LensAdmitted")]
    LensAdmitted(LensAdmittedEntry),
    #[serde(rename = "LensRejected")]
    LensRejected(LensRejectedEntry),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProposalHistoryReadback {
    pub ledger_ref: LedgerRef,
    pub record: AdmissionRecord,
}

pub fn record_admitted<S, C>(
    admitted: &LensAdmittedEntry,
    ledger: &mut AnnealLedger<S, C>,
) -> Result<LedgerRef>
where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    validate_admitted(admitted)?;
    let record = AdmissionRecord::LensAdmitted(admitted.clone());
    ledger
        .write(ledger_entry(record)?)
        .map_err(ledger_write_fail)
}

pub fn record_rejected<S, C>(
    rejected: &LensRejectedEntry,
    ledger: &mut AnnealLedger<S, C>,
) -> Result<LedgerRef>
where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    validate_rejected(rejected)?;
    let record = AdmissionRecord::LensRejected(rejected.clone());
    ledger
        .write(ledger_entry(record)?)
        .map_err(ledger_write_fail)
}

pub fn record_outcome<S, C>(
    outcome: &ProposalOutcome,
    ledger: &mut AnnealLedger<S, C>,
    ts: LogicalTime,
    deficit_gap: f64,
) -> Result<Option<LedgerRef>>
where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    match &outcome.terminal_state {
        ProposalTerminalState::Admitted => {
            let record = admitted_from_outcome(outcome, ts)?;
            record_admitted(&record, ledger).map(Some)
        }
        ProposalTerminalState::GateRejected => {
            let record = rejected_from_outcome(outcome, ts, deficit_gap)?;
            record_rejected(&record, ledger).map(Some)
        }
        ProposalTerminalState::HotAddFailed { .. }
        | ProposalTerminalState::SubstrateReverted { .. }
        | ProposalTerminalState::NoSufficiencyGain => {
            let record = rejected_from_outcome(outcome, ts, deficit_gap)?;
            record_rejected(&record, ledger).map(Some)
        }
        ProposalTerminalState::NoDeficit => Ok(None),
    }
}

pub fn proposal_history<S, C>(ledger: &AnnealLedger<S, C>, n: usize) -> Result<Vec<AdmissionRecord>>
where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    Ok(proposal_history_with_refs(ledger, n)?
        .into_iter()
        .map(|readback| readback.record)
        .collect())
}

pub fn proposal_history_with_refs<S, C>(
    ledger: &AnnealLedger<S, C>,
    n: usize,
) -> Result<Vec<ProposalHistoryReadback>>
where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for readback in ledger.read_recent_with_refs(usize::MAX)? {
        if let Some(record) = record_from_entry(readback.ledger_ref, readback.entry)? {
            records.push(record);
        }
    }
    if n < records.len() {
        records.drain(0..records.len() - n);
    }
    Ok(records)
}

pub fn record_from_entry(
    ledger_ref: LedgerRef,
    entry: AnnealLedgerEntry,
) -> Result<Option<ProposalHistoryReadback>> {
    match entry.action {
        AnnealLedgerAction::LensAdmitted | AnnealLedgerAction::LensRejected => {
            let Some(record) = entry.proposal else {
                return Err(invalid_entry(
                    "Lens proposal ledger action is missing structured proposal payload",
                ));
            };
            validate_action_matches_record(entry.action, &record)?;
            Ok(Some(ProposalHistoryReadback { ledger_ref, record }))
        }
        _ => Ok(None),
    }
}

fn admitted_from_outcome(outcome: &ProposalOutcome, ts: LogicalTime) -> Result<LensAdmittedEntry> {
    let candidate = outcome
        .candidate
        .as_ref()
        .ok_or_else(|| invalid_entry("admitted proposal is missing candidate"))?;
    let (bits_gain, max_corr) = match &outcome.gate_outcome {
        Some(GateOutcome::Admitted { bits, max_corr, .. }) => (*bits, *max_corr),
        _ => {
            return Err(invalid_entry(
                "admitted proposal is missing admitted gate outcome",
            ));
        }
    };
    let sufficiency_after = outcome
        .sufficiency_after
        .ok_or_else(|| invalid_entry("admitted proposal is missing sufficiency_after"))?;
    let change_id = outcome
        .change_id
        .ok_or_else(|| invalid_entry("admitted proposal is missing change_id"))?;
    Ok(LensAdmittedEntry {
        candidate_desc: describe(candidate),
        bits_gain,
        max_corr,
        sufficiency_before: outcome.sufficiency_before,
        sufficiency_after,
        change_id,
        ts,
    })
}

fn rejected_from_outcome(
    outcome: &ProposalOutcome,
    ts: LogicalTime,
    deficit_gap: f64,
) -> Result<LensRejectedEntry> {
    let candidate = outcome
        .candidate
        .as_ref()
        .ok_or_else(|| invalid_entry("rejected proposal is missing candidate"))?;
    let reason = reject_reason_from_outcome(outcome)?;
    Ok(LensRejectedEntry {
        candidate_desc: describe(candidate),
        reason,
        deficit_gap,
        ts,
    })
}

fn reject_reason_from_outcome(outcome: &ProposalOutcome) -> Result<RejectReason> {
    match &outcome.terminal_state {
        ProposalTerminalState::GateRejected => match &outcome.gate_outcome {
            Some(GateOutcome::Rejected { reason }) => Ok(reason.clone()),
            _ => Err(invalid_entry("gate rejection is missing reject reason")),
        },
        ProposalTerminalState::HotAddFailed { code } => {
            Ok(RejectReason::HotAddFailed { code: code.clone() })
        }
        ProposalTerminalState::SubstrateReverted { reason } => {
            Ok(RejectReason::SubstrateReverted {
                shadow_reason: reason.clone(),
            })
        }
        ProposalTerminalState::NoSufficiencyGain => {
            let after = outcome
                .sufficiency_after
                .ok_or_else(|| invalid_entry("no-gain proposal is missing sufficiency_after"))?;
            Ok(RejectReason::NoSufficiencyGain {
                before: outcome.sufficiency_before,
                after,
            })
        }
        ProposalTerminalState::NoDeficit | ProposalTerminalState::Admitted => {
            Err(invalid_entry("terminal state is not a rejection"))
        }
    }
}

fn ledger_entry(record: AdmissionRecord) -> Result<AnnealLedgerEntry> {
    let hash = record_hash(&record)?;
    let (action, change_id, ts, description) = match &record {
        AdmissionRecord::LensAdmitted(entry) => (
            AnnealLedgerAction::LensAdmitted,
            entry.change_id,
            entry.ts,
            format!(
                "LensAdmitted candidate='{}' bits_gain={:.6} max_corr={:.6} sufficiency={:.6}->{:.6}",
                entry.candidate_desc,
                entry.bits_gain,
                entry.max_corr,
                entry.sufficiency_before,
                entry.sufficiency_after
            ),
        ),
        AdmissionRecord::LensRejected(entry) => (
            AnnealLedgerAction::LensRejected,
            rejected_change_id(entry)?,
            entry.ts,
            format!(
                "LensRejected candidate='{}' reason={} deficit_gap={:.6}",
                entry.candidate_desc,
                reject_label(&entry.reason),
                entry.deficit_gap
            ),
        ),
    };
    Ok(AnnealLedgerEntry {
        action,
        change_id,
        artifact_id: format!("lens-proposal-{}", hex_prefix(&hash)),
        prior_ptr_hash: record_context_hash(&record)?,
        candidate_ptr_hash: hash,
        metrics: MetricSnapshot::empty(ts),
        ts,
        description,
        fault: None,
        proposal: Some(record),
        details: None,
        prev_hash: None,
    })
}

fn validate_admitted(entry: &LensAdmittedEntry) -> Result<()> {
    validate_desc(&entry.candidate_desc)?;
    validate_metric("bits_gain", entry.bits_gain)?;
    validate_unit_metric("max_corr", entry.max_corr)?;
    validate_metric("sufficiency_before", entry.sufficiency_before)?;
    validate_metric("sufficiency_after", entry.sufficiency_after)?;
    if entry.sufficiency_after <= entry.sufficiency_before {
        return Err(invalid_metric(
            "sufficiency_after must exceed sufficiency_before for LensAdmitted",
        ));
    }
    Ok(())
}

fn validate_rejected(entry: &LensRejectedEntry) -> Result<()> {
    validate_desc(&entry.candidate_desc)?;
    validate_reject_reason(&entry.reason)?;
    validate_metric("deficit_gap", entry.deficit_gap)?;
    Ok(())
}

fn validate_reject_reason(reason: &RejectReason) -> Result<()> {
    match reason {
        RejectReason::InsufficientBits { bits, threshold } => {
            validate_metric("reject.bits", *bits)?;
            validate_metric("reject.threshold", *threshold)?;
            Ok(())
        }
        RejectReason::NonLearnedSignal {
            signal_kind,
            required,
        } => {
            if signal_kind == required {
                return Err(invalid_metric(
                    "non_learned_signal reject reason cannot already satisfy required kind",
                ));
            }
            Ok(())
        }
        RejectReason::TooCorrelated {
            corr, threshold, ..
        } => {
            validate_unit_metric("reject.corr", *corr)?;
            validate_unit_metric("reject.threshold", *threshold)
        }
        RejectReason::ProfileTimeout => Ok(()),
        RejectReason::ResourceBudgetExceeded {
            vram_mb,
            ram_mb,
            ms_per_input,
            max_vram_mb,
            max_ram_mb,
            max_ms_per_input,
        } => {
            validate_metric("reject.vram_mb", *vram_mb)?;
            validate_metric("reject.ram_mb", *ram_mb)?;
            validate_metric("reject.ms_per_input", *ms_per_input)?;
            validate_metric("reject.max_vram_mb", *max_vram_mb)?;
            validate_metric("reject.max_ram_mb", *max_ram_mb)?;
            validate_metric("reject.max_ms_per_input", *max_ms_per_input)?;
            Ok(())
        }
        RejectReason::HotAddFailed { code } => {
            if code.trim().is_empty() {
                return Err(invalid_entry(
                    "reject.hot_add_failed code must not be empty",
                ));
            }
            Ok(())
        }
        RejectReason::SubstrateReverted { .. } => Ok(()),
        RejectReason::NoSufficiencyGain { before, after } => {
            validate_metric("reject.before", *before)?;
            validate_metric("reject.after", *after)?;
            if after > before {
                return Err(invalid_metric(
                    "no_sufficiency_gain reject reason cannot improve sufficiency",
                ));
            }
            Ok(())
        }
    }
}

fn validate_action_matches_record(
    action: AnnealLedgerAction,
    record: &AdmissionRecord,
) -> Result<()> {
    match (action, record) {
        (AnnealLedgerAction::LensAdmitted, AdmissionRecord::LensAdmitted(entry)) => {
            validate_admitted(entry)
        }
        (AnnealLedgerAction::LensRejected, AdmissionRecord::LensRejected(entry)) => {
            validate_rejected(entry)
        }
        _ => Err(invalid_entry(
            "Lens proposal ledger action does not match structured proposal payload",
        )),
    }
}

fn rejected_change_id(entry: &LensRejectedEntry) -> Result<ChangeId> {
    let hash = record_hash(&AdmissionRecord::LensRejected(entry.clone()))?;
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&hash[..8]);
    Ok(ChangeId(u64::from_be_bytes(bytes).max(1)))
}

fn record_hash(record: &AdmissionRecord) -> Result<[u8; 32]> {
    serde_json::to_vec(record)
        .map(|bytes| blake3::hash(&bytes).into())
        .map_err(|error| invalid_entry(format!("serialize proposal record: {error}")))
}

fn record_context_hash(record: &AdmissionRecord) -> Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    match record {
        AdmissionRecord::LensAdmitted(entry) => {
            hasher.update(&entry.sufficiency_before.to_le_bytes());
            hasher.update(&entry.sufficiency_after.to_le_bytes());
        }
        AdmissionRecord::LensRejected(entry) => {
            hasher.update(&entry.deficit_gap.to_le_bytes());
            hasher.update(reject_label(&entry.reason).as_bytes());
        }
    }
    Ok(hasher.finalize().into())
}

fn validate_desc(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(invalid_entry("candidate_desc must not be empty"));
    }
    Ok(())
}

fn validate_unit_metric(name: &'static str, value: f64) -> Result<()> {
    let value = validate_metric(name, value)?;
    if value > 1.0 {
        return Err(invalid_metric(format!(
            "{name} must be <= 1.0, got {value}"
        )));
    }
    Ok(())
}

fn validate_metric(name: &'static str, value: f64) -> Result<f64> {
    if !value.is_finite() || value < 0.0 {
        return Err(invalid_metric(format!(
            "{name} must be finite and non-negative, got {value}"
        )));
    }
    Ok(value)
}

fn reject_label(reason: &RejectReason) -> &'static str {
    match reason {
        RejectReason::InsufficientBits { .. } => "insufficient_bits",
        RejectReason::NonLearnedSignal { .. } => "non_learned_signal",
        RejectReason::TooCorrelated { .. } => "too_correlated",
        RejectReason::ProfileTimeout => "profile_timeout",
        RejectReason::ResourceBudgetExceeded { .. } => "resource_budget_exceeded",
        RejectReason::HotAddFailed { .. } => "hot_add_failed",
        RejectReason::SubstrateReverted { .. } => "substrate_reverted",
        RejectReason::NoSufficiencyGain { .. } => "no_sufficiency_gain",
    }
}

fn hex_prefix(bytes: &[u8; 32]) -> String {
    bytes[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn ledger_write_fail(error: CalyxError) -> CalyxError {
    CalyxError {
        code: CALYX_LEDGER_WRITE_FAIL,
        message: format!(
            "Lens proposal ledger write failed: {}: {}",
            error.code, error.message
        ),
        remediation: "repair the ledger CF before accepting the lens proposal record",
    }
}

fn invalid_metric(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_INVALID_METRIC,
        message: message.into(),
        remediation: "re-measure lens proposal metrics before recording admission history",
    }
}

fn invalid_entry(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_LEDGER_INVALID_ENTRY,
        message: message.into(),
        remediation: "repair the structured lens proposal ledger payload",
    }
}
