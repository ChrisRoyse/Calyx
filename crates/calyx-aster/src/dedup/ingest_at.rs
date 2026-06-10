use calyx_core::{
    CalyxError, Clock, Constellation, CxId, GuardTauProfile, Result, SlotId, VaultStore,
};

use super::audit::DedupRestoreSnapshot;
use super::engine::check_dedup_without_conflict_write;
use super::ingest_event::{DedupOnlineKind, next_online_prefix, online_event_row, online_kind};
use super::ingest_ledger::{
    LedgerPayload, RecurrenceSignatureLedger, action_name, action_name_for_action, ledger_payload,
};
use super::signature::{SignatureResult, detect_recurrence_signature};
use super::{
    AnchorConflictResult, CALYX_DEDUP_INVALID_EVENT_TIME, ContestedWith, DedupAction,
    DedupDecision, DedupPolicy, DedupResult, EpochSecs, IngestInput, OccurrenceId, TctCosineConfig,
    check_anchor_conflict, contested_with_key, dedup_error, encode_contested_with,
    is_recurrence_series_policy,
};
use crate::cf::ColumnFamily;
use crate::recurrence::{OccurrenceContext, RetentionPolicy, build_append};
use crate::vault::AsterVault;

pub fn ingest_at<C>(
    vault: &AsterVault<C>,
    input: &IngestInput,
    at: EpochSecs,
    guard_profile: Option<&dyn GuardTauProfile>,
) -> Result<DedupResult>
where
    C: Clock,
{
    let policy = vault.dedup_policy().clone();
    if is_recurrence_series_policy(&policy) {
        return vault.with_recurrence_write_lock(|| {
            ingest_at_with_policy(vault, input, at, guard_profile, &policy)
        });
    }
    ingest_at_with_policy(vault, input, at, guard_profile, &policy)
}

fn ingest_at_with_policy<C>(
    vault: &AsterVault<C>,
    input: &IngestInput,
    at: EpochSecs,
    guard_profile: Option<&dyn GuardTauProfile>,
    policy: &DedupPolicy,
) -> Result<DedupResult>
where
    C: Clock,
{
    let new_cx = input.to_constellation(vault, at)?;
    let decision = check_dedup_without_conflict_write(&new_cx, vault, policy, guard_profile)?;
    match decision {
        DedupDecision::NoMatch => store_new(vault, new_cx, at, policy, "NoMatch", Vec::new()),
        DedupDecision::AnchorConflict { existing } => {
            let existing_cx = vault.get(existing, vault.snapshot())?;
            let online_rows = contested_rows(&new_cx, &existing_cx)?;
            store_new(vault, new_cx, at, policy, "AnchorConflict", online_rows)
        }
        DedupDecision::Match {
            existing,
            per_slot_cos,
        } => match policy {
            DedupPolicy::Exact => exact_duplicate(vault, &new_cx, at, existing, per_slot_cos),
            DedupPolicy::TctCosine(config) => {
                if same_event_exact(vault, new_cx.cx_id, existing, at)? {
                    exact_duplicate(vault, &new_cx, at, existing, per_slot_cos)
                } else if config.action == DedupAction::RecurrenceSeries {
                    recurrence_match(
                        vault,
                        RecurrenceMatch {
                            input,
                            new_cx,
                            at,
                            existing,
                            per_slot_cos,
                            config,
                            guard_profile,
                        },
                    )
                } else {
                    merge_match(
                        vault,
                        new_cx,
                        at,
                        existing,
                        per_slot_cos,
                        config.action.clone(),
                        None,
                    )
                }
            }
            DedupPolicy::Off => store_new(vault, new_cx, at, policy, "NoMatch", Vec::new()),
        },
    }
}

pub fn ingest<C>(
    vault: &AsterVault<C>,
    input: &IngestInput,
    clock: &dyn Clock,
    guard_profile: Option<&dyn GuardTauProfile>,
) -> Result<DedupResult>
where
    C: Clock,
{
    let now_secs = i64::try_from(clock.now() / 1_000).map_err(|_| {
        dedup_error(
            CALYX_DEDUP_INVALID_EVENT_TIME,
            "clock timestamp does not fit EpochSecs",
        )
    })?;
    ingest_at(vault, input, EpochSecs(now_secs), guard_profile)
}

fn store_new<C>(
    vault: &AsterVault<C>,
    mut new_cx: Constellation,
    at: EpochSecs,
    policy: &DedupPolicy,
    decision: &'static str,
    mut online_rows: Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<DedupResult>
where
    C: Clock,
{
    let is_recurrence_series = matches!(
        policy,
        DedupPolicy::TctCosine(config) if config.action == DedupAction::RecurrenceSeries
    );
    let mut recurrence_rows = Vec::new();
    if is_recurrence_series {
        let append = build_append(
            vault,
            new_cx,
            at,
            OccurrenceContext::new(Vec::new())?,
            at,
            RetentionPolicy::default(),
        )?;
        let occurrence = append.occurrence_id;
        new_cx = append.updated_base;
        recurrence_rows = append.recurrence_rows;
        online_rows.push(online_event_row(
            DedupOnlineKind::Occurrence,
            new_cx.cx_id,
            new_cx.cx_id,
            occurrence,
            at,
            DedupAction::RecurrenceSeries,
            Vec::new(),
        )?);
    }
    let payload = ledger_payload(LedgerPayload {
        cx: &new_cx,
        at,
        result: "New",
        decision,
        action: action_name(policy),
        into: None,
        occurrence: None,
        per_slot_cos: &[],
        recurrence_signature: None,
        restore: None,
    })?;
    let id = new_cx.cx_id;
    vault.commit_dedup_ingest(
        Some(new_cx),
        None,
        online_rows,
        recurrence_rows,
        id,
        payload,
    )?;
    Ok(DedupResult::New(id))
}

fn exact_duplicate<C>(
    vault: &AsterVault<C>,
    new_cx: &Constellation,
    at: EpochSecs,
    existing: CxId,
    per_slot_cos: Vec<(SlotId, f32)>,
) -> Result<DedupResult>
where
    C: Clock,
{
    let payload = ledger_payload(LedgerPayload {
        cx: new_cx,
        at,
        result: "ExactDuplicate",
        decision: "Match",
        action: Some("Exact"),
        into: Some(existing),
        occurrence: None,
        per_slot_cos: &per_slot_cos,
        recurrence_signature: None,
        restore: None,
    })?;
    vault.commit_dedup_ingest(None, None, Vec::new(), Vec::new(), existing, payload)?;
    Ok(DedupResult::ExactDuplicate(existing))
}

fn merge_match<C>(
    vault: &AsterVault<C>,
    new_cx: Constellation,
    at: EpochSecs,
    existing: CxId,
    per_slot_cos: Vec<(SlotId, f32)>,
    action: DedupAction,
    signature: Option<RecurrenceSignatureLedger>,
) -> Result<DedupResult>
where
    C: Clock,
{
    let kind = online_kind(&action);
    let mut updated_base = None;
    let mut recurrence_rows = Vec::new();
    let mut before_base = None;
    let mut recurrence_tombstones = Vec::new();
    let occurrence = if action == DedupAction::RecurrenceSeries {
        let base = vault.get(existing, vault.snapshot())?;
        before_base = Some(base.clone());
        let append = build_append(
            vault,
            base,
            at,
            OccurrenceContext::new(Vec::new())?,
            at,
            RetentionPolicy::default(),
        )?;
        updated_base = Some(append.updated_base);
        recurrence_rows = append.recurrence_rows;
        recurrence_tombstones.push(append.occurrence_id);
        append.occurrence_id
    } else {
        next_occurrence_id(vault, kind, existing)?
    };
    let online_rows = vec![online_event_row(
        kind,
        existing,
        new_cx.cx_id,
        occurrence,
        at,
        action.clone(),
        per_slot_cos.clone(),
    )?];
    let restore = DedupRestoreSnapshot::new(
        vault.vault_id(),
        existing,
        new_cx.clone(),
        before_base,
        recurrence_tombstones,
    );
    let payload = ledger_payload(LedgerPayload {
        cx: &new_cx,
        at,
        result: "DedupMerge",
        decision: "Match",
        action: Some(action_name_for_action(&action)),
        into: Some(existing),
        occurrence: Some(occurrence),
        per_slot_cos: &per_slot_cos,
        recurrence_signature: signature,
        restore: Some(&restore),
    })?;
    let candidate = (action == DedupAction::Link).then_some(new_cx);
    let subject = candidate.as_ref().map_or(existing, |cx| cx.cx_id);
    vault.commit_dedup_ingest(
        candidate,
        updated_base,
        online_rows,
        recurrence_rows,
        subject,
        payload,
    )?;
    Ok(DedupResult::DedupMerge {
        into: existing,
        occurrence,
    })
}

fn recurrence_match<C>(vault: &AsterVault<C>, matched: RecurrenceMatch<'_>) -> Result<DedupResult>
where
    C: Clock,
{
    let existing_cx = vault.get(matched.existing, vault.snapshot())?;
    match detect_recurrence_signature(
        &matched.new_cx,
        &existing_cx,
        matched.config,
        matched.input.temporal_slot_ids(),
        matched.guard_profile,
        matched.at,
    )? {
        SignatureResult::RecurrenceSignature {
            same_action,
            new_time,
        } => merge_match(
            vault,
            matched.new_cx,
            matched.at,
            matched.existing,
            matched.per_slot_cos,
            DedupAction::RecurrenceSeries,
            Some(RecurrenceSignatureLedger {
                same_action,
                new_time,
            }),
        ),
        SignatureResult::SameTime => exact_duplicate(
            vault,
            &matched.new_cx,
            matched.at,
            matched.existing,
            matched.per_slot_cos,
        ),
        SignatureResult::NewContent | SignatureResult::ContentMismatch => store_new(
            vault,
            matched.new_cx,
            matched.at,
            &DedupPolicy::TctCosine(matched.config.clone()),
            "ContentMismatch",
            Vec::new(),
        ),
    }
}

struct RecurrenceMatch<'a> {
    input: &'a IngestInput,
    new_cx: Constellation,
    at: EpochSecs,
    existing: CxId,
    per_slot_cos: Vec<(SlotId, f32)>,
    config: &'a TctCosineConfig,
    guard_profile: Option<&'a dyn GuardTauProfile>,
}

fn same_event_exact<C>(
    vault: &AsterVault<C>,
    new_id: CxId,
    existing: CxId,
    at: EpochSecs,
) -> Result<bool>
where
    C: Clock,
{
    if new_id != existing {
        return Ok(false);
    }
    let existing_cx = vault.get(existing, vault.snapshot())?;
    Ok(existing_cx.created_at == at.to_u64()?)
}

fn contested_rows(
    new_cx: &Constellation,
    existing_cx: &Constellation,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let AnchorConflictResult::Conflicting {
        anchor_type,
        reason,
    } = check_anchor_conflict(new_cx, existing_cx)
    else {
        return Err(CalyxError::aster_corrupt_shard(
            "dedup decision reported anchor conflict but anchors are compatible",
        ));
    };
    let new_value = ContestedWith {
        contested_with: existing_cx.cx_id,
        anchor_type: anchor_type.clone(),
        reason: reason.clone(),
    };
    let existing_value = ContestedWith {
        contested_with: new_cx.cx_id,
        anchor_type,
        reason,
    };
    Ok(vec![
        (
            contested_with_key(new_cx.cx_id),
            encode_contested_with(&new_value)?,
        ),
        (
            contested_with_key(existing_cx.cx_id),
            encode_contested_with(&existing_value)?,
        ),
    ])
}

fn next_occurrence_id<C>(
    vault: &AsterVault<C>,
    kind: DedupOnlineKind,
    into: CxId,
) -> Result<OccurrenceId>
where
    C: Clock,
{
    let prefix = next_online_prefix(kind, into);
    let count = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Online)?
        .into_iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .count();
    Ok(OccurrenceId(count as u64))
}

#[cfg(test)]
#[path = "ingest_at_tests.rs"]
mod tests;
