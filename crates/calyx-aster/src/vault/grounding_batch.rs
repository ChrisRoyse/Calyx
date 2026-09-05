use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use calyx_core::{Anchor, CalyxError, Clock, CxId, LedgerRef, Result, VaultStore};
use calyx_ledger::{ActorId, EntryKind, RedactionPolicy, SubjectId, decode as decode_ledger};
use rayon::prelude::*;
use sha2::{Digest, Sha256};

use super::{AsterVault, encode, ledger_hook};
use crate::cf::{ColumnFamily, anchor_key, base_key};

const ACTOR: &str = "calyx-grounding-batch";
const MAX_EVENTS: usize = 16_384;
const MAX_TARGETS_PER_EVENT: usize = 4_096;
const MAX_ANCHORS_PER_EVENT: usize = 64;
const MAX_LEDGER_PAYLOAD_BYTES: usize = 64 * 1024;
const PREFLIGHT_SCAN_PAGE_ROWS: usize = 16_384;
const LEDGER_SCAN_PAGE_ROWS: usize = 1_024;
const CHECKPOINT_EVENTS: usize = 32;
const CX_ID_BYTES: usize = 16;

fn grounding_input_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_GROUNDING_INPUT_INVALID",
        message: message.into(),
        remediation: "fix the grounding request; validation completed before any write",
    }
}

fn grounding_conflict(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_GROUNDING_CONFLICT",
        message: message.into(),
        remediation: "correct the plan to match the persisted anchor or use a distinct AnchorKind; no write was performed",
    }
}

fn grounding_verify_missing(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_GROUNDING_VERIFY_MISSING",
        message: message.into(),
        remediation: "apply the exact hash-bound plan before verify-only, then reopen and verify again",
    }
}

/// One independently atomic grounding event applied to existing constellations.
/// Event identity is exactly SHA-256(payload), and every target receives the
/// same intervention-scoped anchor set.
#[derive(Clone, Debug)]
pub struct GroundingBatchEvent {
    pub event_id: [u8; 32],
    /// Prior schema event whose exact physical anchors this event may
    /// atomically rebind to its new ledger identity.
    pub supersedes_event_id: Option<[u8; 32]>,
    pub targets: Vec<CxId>,
    pub anchors: Vec<Anchor>,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroundingBatchDisposition {
    Committed,
    ExistingExact,
}

#[derive(Clone, Debug)]
pub struct GroundingBatchEventResult {
    pub event_id: [u8; 32],
    pub disposition: GroundingBatchDisposition,
    pub ledger_ref: LedgerRef,
    pub targets: usize,
    pub anchors_per_target: usize,
}

#[derive(Clone, Debug)]
pub struct GroundingBatchReport {
    pub events: Vec<GroundingBatchEventResult>,
    pub committed: usize,
    pub existing_exact: usize,
    pub physical_anchor_rows: usize,
    pub checkpoint_event_interval: usize,
    pub checkpoint_flushes: usize,
    pub mvcc_versions_reclaimed: usize,
    pub mvcc_value_bytes_reclaimed: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreflightDisposition {
    Missing,
    ExistingExact,
    MigrationRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BasePresence {
    Missing,
    Present,
}

#[derive(Clone, Debug)]
struct AnchorExpectation {
    event_index: usize,
    anchor_index: usize,
    expected_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
struct LedgerMatches {
    current: Option<LedgerRef>,
    superseded: Option<LedgerRef>,
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Validates the complete request before its first write, then commits each
    /// semantic event as one Base + Anchors + Ledger durable group commit.
    /// Exact replay is a true zero-write result.
    pub fn ground_existing_batch(
        &self,
        events: Vec<GroundingBatchEvent>,
        verify_only: bool,
    ) -> Result<GroundingBatchReport> {
        validate_events(&events)?;
        self.with_durable_commit_lock(move || {
            let snapshot = self.snapshot();
            let ledger_matches = matching_ledger_refs(self, snapshot, &events)?;
            let base_presence = classify_base_presence_streamed(self, snapshot, &events)?;
            let dispositions = preflight_events_streamed(
                self,
                snapshot,
                &events,
                &ledger_matches,
                &base_presence,
            )?;
            if verify_only
                && dispositions
                    .iter()
                    .any(|item| *item != PreflightDisposition::ExistingExact)
            {
                let missing = dispositions
                    .iter()
                    .filter(|item| **item == PreflightDisposition::Missing)
                    .count();
                let migrations = dispositions
                    .iter()
                    .filter(|item| **item == PreflightDisposition::MigrationRequired)
                    .count();
                return Err(grounding_verify_missing(format!(
                    "grounding batch verify-only found {missing} missing events and {migrations} pending schema migrations"
                )));
            }

            let mut results = Vec::with_capacity(events.len());
            let mut committed = 0usize;
            let mut existing_exact = 0usize;
            let mut physical_anchor_rows = 0usize;
            let mut checkpoint_flushes = 0usize;
            let mut committed_since_checkpoint = 0usize;
            let mut mvcc_versions_reclaimed = 0usize;
            let mut mvcc_value_bytes_reclaimed = 0usize;
            let mut checkpoint_targets = BTreeSet::new();
            if !verify_only
                && dispositions
                    .iter()
                    .any(|item| *item != PreflightDisposition::ExistingExact)
            {
                for (event, disposition) in events.iter().zip(dispositions.iter()) {
                    if *disposition == PreflightDisposition::ExistingExact {
                        checkpoint_targets.extend(event.targets.iter().copied());
                    }
                }
                let reclaimed =
                    checkpoint_grounding_epoch_locked(self, 0, &checkpoint_targets)?;
                checkpoint_targets.clear();
                checkpoint_flushes += 1;
                mvcc_versions_reclaimed = mvcc_versions_reclaimed
                    .checked_add(reclaimed.0)
                    .ok_or_else(|| {
                        CalyxError::aster_corrupt_shard(
                            "grounding reclaimed-version count overflow",
                        )
                    })?;
                mvcc_value_bytes_reclaimed = mvcc_value_bytes_reclaimed
                    .checked_add(reclaimed.1)
                    .ok_or_else(|| {
                        CalyxError::aster_corrupt_shard(
                            "grounding reclaimed-byte count overflow",
                        )
                    })?;
            }
            for ((event, disposition), ledger_match) in
                events.iter().zip(dispositions).zip(ledger_matches)
            {
                physical_anchor_rows = physical_anchor_rows
                    .checked_add(event.targets.len().saturating_mul(event.anchors.len()))
                    .ok_or_else(|| {
                        grounding_input_invalid("grounding batch physical anchor count overflow")
                    })?;
                let (result_disposition, ledger_ref) = match disposition {
                    PreflightDisposition::ExistingExact => {
                        existing_exact += 1;
                        (
                            GroundingBatchDisposition::ExistingExact,
                            ledger_match.current.ok_or_else(|| {
                                CalyxError::aster_corrupt_shard(
                                    "exact grounding replay lost its ledger reference",
                                )
                            })?,
                        )
                    }
                    PreflightDisposition::Missing => {
                        if verify_only {
                            unreachable!("verify-only missing events fail before apply")
                        }
                        committed += 1;
                        committed_since_checkpoint += 1;
                        (
                            GroundingBatchDisposition::Committed,
                            commit_event_locked(self, event)?,
                        )
                    }
                    PreflightDisposition::MigrationRequired => {
                        if verify_only {
                            unreachable!("verify-only migrations fail before apply")
                        }
                        committed += 1;
                        committed_since_checkpoint += 1;
                        (
                            GroundingBatchDisposition::Committed,
                            commit_migration_locked(self, event)?,
                        )
                    }
                };
                results.push(GroundingBatchEventResult {
                    event_id: event.event_id,
                    disposition: result_disposition,
                    ledger_ref,
                    targets: event.targets.len(),
                    anchors_per_target: event.anchors.len(),
                });
                if result_disposition == GroundingBatchDisposition::Committed {
                    checkpoint_targets.extend(event.targets.iter().copied());
                }
                if committed_since_checkpoint == CHECKPOINT_EVENTS {
                    let reclaimed = checkpoint_grounding_epoch_locked(
                        self,
                        committed,
                        &checkpoint_targets,
                    )?;
                    checkpoint_flushes += 1;
                    committed_since_checkpoint = 0;
                    checkpoint_targets.clear();
                    mvcc_versions_reclaimed = mvcc_versions_reclaimed
                        .checked_add(reclaimed.0)
                        .ok_or_else(|| {
                            CalyxError::aster_corrupt_shard(
                                "grounding reclaimed-version count overflow",
                            )
                        })?;
                    mvcc_value_bytes_reclaimed = mvcc_value_bytes_reclaimed
                        .checked_add(reclaimed.1)
                        .ok_or_else(|| {
                            CalyxError::aster_corrupt_shard(
                                "grounding reclaimed-byte count overflow",
                            )
                        })?;
                }
            }
            if committed_since_checkpoint != 0 {
                let reclaimed =
                    checkpoint_grounding_epoch_locked(self, committed, &checkpoint_targets)?;
                checkpoint_flushes += 1;
                mvcc_versions_reclaimed = mvcc_versions_reclaimed
                    .checked_add(reclaimed.0)
                    .ok_or_else(|| {
                        CalyxError::aster_corrupt_shard(
                            "grounding reclaimed-version count overflow",
                        )
                    })?;
                mvcc_value_bytes_reclaimed = mvcc_value_bytes_reclaimed
                    .checked_add(reclaimed.1)
                    .ok_or_else(|| {
                        CalyxError::aster_corrupt_shard(
                            "grounding reclaimed-byte count overflow",
                        )
                    })?;
            }
            Ok(GroundingBatchReport {
                events: results,
                committed,
                existing_exact,
                physical_anchor_rows,
                checkpoint_event_interval: CHECKPOINT_EVENTS,
                checkpoint_flushes,
                mvcc_versions_reclaimed,
                mvcc_value_bytes_reclaimed,
            })
        })
    }
}

fn checkpoint_grounding_epoch_locked<C: Clock>(
    vault: &AsterVault<C>,
    committed: usize,
    targets: &BTreeSet<CxId>,
) -> Result<(usize, usize)> {
    vault.flush_locked().map_err(|error| CalyxError {
        code: error.code,
        message: format!(
            "grounding checkpoint failed after {committed} committed events: {}",
            error.message
        ),
        remediation: error.remediation,
    })?;
    let current_seq = vault.rows.current_seq();
    let safe_point = vault.rows.snapshot_gc_safe_point(vault.clock.now());
    if safe_point != current_seq {
        return Err(CalyxError {
            code: "CALYX_GROUNDING_CHECKPOINT_PINNED",
            message: format!(
                "grounding checkpoint cannot reclaim obsolete MVCC versions after {committed} committed events: current_seq={current_seq} oldest_pinned_seq={safe_point}"
            ),
            remediation: "close the exact long-lived reader pin before resuming the hash-bound grounding plan; committed events remain durable and idempotently replayable",
        });
    }
    let keys = targets
        .iter()
        .map(|id| (ColumnFamily::Base, base_key(*id)))
        .collect::<Vec<_>>();
    let reclaimed = vault
        .rows
        .reclaim_snapshot_versions_for_keys(safe_point, &keys)
        .map_err(|error| CalyxError {
            code: error.code,
            message: format!(
                "grounding checkpoint MVCC reclamation failed after {committed} committed events at safe_point={safe_point}: {}",
                error.message
            ),
            remediation: error.remediation,
        })?;
    if reclaimed.rate_limited || reclaimed.compaction_debt != 0 {
        return Err(CalyxError {
            code: "CALYX_GROUNDING_CHECKPOINT_INCOMPLETE",
            message: format!(
                "grounding checkpoint left obsolete MVCC state after {committed} committed events: safe_point={} versions_reclaimed={} bytes_freed={} compaction_debt={} rate_limited={}",
                reclaimed.safe_point_seq,
                reclaimed.versions_reclaimed,
                reclaimed.bytes_freed,
                reclaimed.compaction_debt,
                reclaimed.rate_limited
            ),
            remediation: "inspect reader leases and MVCC GC state before resuming; committed events remain durable and idempotently replayable",
        });
    }
    Ok((reclaimed.versions_reclaimed, reclaimed.bytes_freed))
}

fn validate_events(events: &[GroundingBatchEvent]) -> Result<()> {
    if events.is_empty() || events.len() > MAX_EVENTS {
        return Err(grounding_input_invalid(format!(
            "grounding batch events={} outside 1..={MAX_EVENTS}",
            events.len()
        )));
    }
    let mut event_ids = BTreeSet::new();
    let mut superseded_event_ids = BTreeSet::new();
    let mut global_kinds = BTreeSet::new();
    for event in events {
        if !event_ids.insert(event.event_id) {
            return Err(grounding_input_invalid(
                "grounding batch contains a duplicate event id",
            ));
        }
        if let Some(superseded) = event.supersedes_event_id
            && (superseded == event.event_id || !superseded_event_ids.insert(superseded))
        {
            return Err(grounding_input_invalid(
                "grounding batch superseded event ids must be unique and differ from current event ids",
            ));
        }
        if event.payload.is_empty() || event.payload.len() > MAX_LEDGER_PAYLOAD_BYTES {
            return Err(grounding_input_invalid(format!(
                "grounding event payload bytes={} outside 1..={MAX_LEDGER_PAYLOAD_BYTES}",
                event.payload.len()
            )));
        }
        RedactionPolicy::check_payload(&event.payload)?;
        let observed: [u8; 32] = Sha256::digest(&event.payload).into();
        if observed != event.event_id {
            return Err(grounding_input_invalid(
                "grounding event id is not SHA-256(payload)",
            ));
        }
        if event.targets.is_empty() || event.targets.len() > MAX_TARGETS_PER_EVENT {
            return Err(grounding_input_invalid(format!(
                "grounding event targets={} outside 1..={MAX_TARGETS_PER_EVENT}",
                event.targets.len()
            )));
        }
        if !event.targets.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(grounding_input_invalid(
                "grounding event targets must be strictly increasing and unique",
            ));
        }
        if event.anchors.is_empty() || event.anchors.len() > MAX_ANCHORS_PER_EVENT {
            return Err(grounding_input_invalid(format!(
                "grounding event anchors={} outside 1..={MAX_ANCHORS_PER_EVENT}",
                event.anchors.len()
            )));
        }
        let mut local_kinds = BTreeSet::new();
        for anchor in &event.anchors {
            anchor.validate_schema()?;
            if !local_kinds.insert(anchor.kind.clone()) {
                return Err(grounding_input_invalid(
                    "grounding event contains a duplicate anchor kind",
                ));
            }
            if !global_kinds.insert(anchor.kind.clone()) {
                return Err(grounding_input_invalid(format!(
                    "grounding batch reuses physical AnchorKind {:?} across events",
                    anchor.kind
                )));
            }
        }
        if !event
            .anchors
            .windows(2)
            .all(|pair| pair[0].kind < pair[1].kind)
        {
            return Err(grounding_input_invalid(
                "grounding event anchors must be strictly ordered by kind",
            ));
        }
    }
    if event_ids
        .iter()
        .any(|event_id| superseded_event_ids.contains(event_id))
    {
        return Err(grounding_input_invalid(
            "grounding batch current and superseded event id sets overlap",
        ));
    }
    Ok(())
}

fn matching_ledger_refs<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: u64,
    events: &[GroundingBatchEvent],
) -> Result<Vec<LedgerMatches>> {
    let mut by_id = BTreeMap::new();
    for (index, event) in events.iter().enumerate() {
        by_id.insert(event.event_id, (index, false));
        if let Some(superseded) = event.supersedes_event_id {
            by_id.insert(superseded, (index, true));
        }
    }
    let mut matches = vec![LedgerMatches::default(); events.len()];
    vault.rows.scan_cf_pages_renewing_latest(
        snapshot,
        ColumnFamily::Ledger,
        LEDGER_SCAN_PAGE_ROWS,
        &vault.clock,
        |page| {
            for (_, bytes) in page {
                let entry = decode_ledger(&bytes)?;
                let SubjectId::Query(subject) = &entry.subject else {
                    continue;
                };
                let Ok(event_id) = <[u8; 32]>::try_from(subject.as_slice()) else {
                    continue;
                };
                let Some((index, superseded)) = by_id.get(&event_id).copied() else {
                    continue;
                };
                let event = &events[index];
                let ledger_ref = LedgerRef {
                    seq: entry.seq,
                    hash: entry.entry_hash,
                };
                if superseded {
                    if matches[index].superseded.is_some() {
                        return Err(CalyxError::aster_corrupt_shard(
                            "grounding event has duplicate superseded ledger entries",
                        ));
                    }
                    let observed: [u8; 32] = Sha256::digest(&entry.payload).into();
                    if entry.kind != EntryKind::Grounding
                        || observed != event_id
                        || entry.actor != ActorId::Service(ACTOR.to_string())
                    {
                        return Err(CalyxError::aster_corrupt_shard(
                            "superseded grounding ledger identity is invalid",
                        ));
                    }
                    matches[index].superseded = Some(ledger_ref);
                } else {
                    if matches[index].current.is_some() {
                        return Err(CalyxError::aster_corrupt_shard(
                            "grounding event has duplicate current ledger entries",
                        ));
                    }
                    if entry.kind != EntryKind::Grounding
                        || entry.payload != event.payload
                        || entry.actor != ActorId::Service(ACTOR.to_string())
                    {
                        return Err(CalyxError::aster_corrupt_shard(
                            "grounding event ledger identity conflicts with requested event",
                        ));
                    }
                    matches[index].current = Some(ledger_ref);
                }
            }
            Ok::<(), CalyxError>(())
        },
    )?;
    Ok(matches)
}

/// Classifies every planned Base anchor with one bounded ordered CF scan.
///
/// The compact application index contains one `(target, event)` pair per plan
/// target occurrence. It replaces the former `BTreeMap<CxId, Vec<Anchor>>`,
/// which retained the decoded anchor vectors for every distinct target and
/// drove multi-gigabyte peak heaps on exhaustive grounding plans. The merge
/// scan decodes only a matching Base row, checks all events that reference it,
/// and releases the row with its scan page.
fn classify_base_presence_streamed<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: u64,
    events: &[GroundingBatchEvent],
) -> Result<Vec<BasePresence>> {
    let application_count = events.iter().try_fold(0usize, |count, event| {
        count
            .checked_add(event.targets.len())
            .ok_or_else(|| grounding_input_invalid("grounding target application count overflow"))
    })?;
    let mut applications = Vec::with_capacity(application_count);
    for (event_index, event) in events.iter().enumerate() {
        applications.extend(event.targets.iter().map(|id| (*id, event_index)));
    }
    applications.par_sort_unstable();

    let mut present = vec![0usize; events.len()];
    let mut missing = vec![0usize; events.len()];
    let mut cursor = 0usize;
    vault.rows.scan_cf_pages_renewing_latest(
        snapshot,
        ColumnFamily::Base,
        PREFLIGHT_SCAN_PAGE_ROWS,
        &vault.clock,
        |page| {
            for (key, bytes) in page {
                if key.len() != CX_ID_BYTES {
                    return Err(CalyxError::aster_corrupt_shard(format!(
                        "Base CF key has {} bytes, expected {CX_ID_BYTES}",
                        key.len()
                    )));
                }
                let id = CxId::from_bytes(
                    key.as_slice()
                        .try_into()
                        .expect("Base key length checked"),
                );
                if cursor < applications.len() && applications[cursor].0 < id {
                    return Err(grounding_conflict(format!(
                        "grounding event target {} is absent from Base",
                        applications[cursor].0
                    )));
                }
                if cursor == applications.len() || applications[cursor].0 != id {
                    continue;
                }

                let constellation = encode::decode_constellation_base(&bytes)?;
                if constellation.cx_id != id {
                    return Err(CalyxError::aster_corrupt_shard(format!(
                        "Base key {id} conflicts with encoded constellation {}",
                        constellation.cx_id
                    )));
                }
                let mut anchors = constellation.anchors;
                anchors.sort_by(|left, right| left.kind.cmp(&right.kind));
                if let Some(duplicate) =
                    anchors.windows(2).find(|pair| pair[0].kind == pair[1].kind)
                {
                    return Err(CalyxError::aster_corrupt_shard(format!(
                        "Base has duplicate anchors for target {id} kind {:?}",
                        duplicate[0].kind
                    )));
                }

                while cursor < applications.len() && applications[cursor].0 == id {
                    let event_index = applications[cursor].1;
                    for anchor in &events[event_index].anchors {
                        match anchors
                            .binary_search_by(|existing| existing.kind.cmp(&anchor.kind))
                        {
                            Ok(index) if anchors[index] == *anchor => {
                                present[event_index] = present[event_index]
                                    .checked_add(1)
                                    .ok_or_else(|| {
                                        grounding_input_invalid(
                                            "grounding Base present count overflow",
                                        )
                                    })?;
                            }
                            Ok(_) => {
                                return Err(grounding_conflict(format!(
                                    "grounding plan conflicts with persisted Base anchor for target {id} kind {:?}",
                                    anchor.kind
                                )));
                            }
                            Err(_) => {
                                missing[event_index] = missing[event_index]
                                    .checked_add(1)
                                    .ok_or_else(|| {
                                        grounding_input_invalid(
                                            "grounding Base missing count overflow",
                                        )
                                    })?;
                            }
                        }
                    }
                    cursor += 1;
                }
            }
            Ok::<(), CalyxError>(())
        },
    )?;
    if cursor != applications.len() {
        return Err(grounding_conflict(format!(
            "grounding event target {} is absent from Base",
            applications[cursor].0
        )));
    }

    let mut presence = Vec::with_capacity(events.len());
    for ((event, present), missing) in events.iter().zip(present).zip(missing) {
        let expected = event
            .targets
            .len()
            .checked_mul(event.anchors.len())
            .ok_or_else(|| grounding_input_invalid("grounding Base count overflow"))?;
        match (present, missing) {
            (0, value) if value == expected => presence.push(BasePresence::Missing),
            (value, 0) if value == expected => presence.push(BasePresence::Present),
            _ => {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "grounding event {} has partial Base state: present={present} missing={missing} expected={expected}",
                    hex_32(&event.event_id)
                )));
            }
        }
    }
    Ok(presence)
}

fn preflight_events_streamed<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: u64,
    events: &[GroundingBatchEvent],
    ledger_matches: &[LedgerMatches],
    base_presence: &[BasePresence],
) -> Result<Vec<PreflightDisposition>> {
    if events.len() != ledger_matches.len() || events.len() != base_presence.len() {
        return Err(CalyxError::aster_corrupt_shard(
            "grounding preflight state vectors have inconsistent lengths",
        ));
    }

    let mut offsets = Vec::with_capacity(events.len());
    let mut total_expected = 0usize;
    let mut expectations = BTreeMap::<Vec<u8>, AnchorExpectation>::new();
    for (event_index, event) in events.iter().enumerate() {
        offsets.push(total_expected);
        let event_rows = event
            .targets
            .len()
            .checked_mul(event.anchors.len())
            .ok_or_else(|| grounding_input_invalid("grounding anchor count overflow"))?;
        total_expected = total_expected
            .checked_add(event_rows)
            .ok_or_else(|| grounding_input_invalid("grounding total anchor count overflow"))?;
        for (anchor_index, anchor) in event.anchors.iter().enumerate() {
            let key = anchor_key(event.targets[0], &anchor.kind);
            let suffix = key.get(CX_ID_BYTES..).ok_or_else(|| {
                CalyxError::aster_corrupt_shard("encoded anchor key is shorter than its CxId")
            })?;
            let expectation = AnchorExpectation {
                event_index,
                anchor_index,
                expected_bytes: encode::encode_anchor(anchor)?,
            };
            if expectations.insert(suffix.to_vec(), expectation).is_some() {
                return Err(grounding_input_invalid(
                    "grounding anchor-kind encodings must be globally unique",
                ));
            }
        }
    }

    let mut seen = vec![0_u64; total_expected.div_ceil(u64::BITS as usize)];
    let mut present = vec![0usize; events.len()];
    vault.rows.scan_cf_pages_renewing_latest(
        snapshot,
        ColumnFamily::Anchors,
        PREFLIGHT_SCAN_PAGE_ROWS,
        &vault.clock,
        |page| {
            for (key, bytes) in page {
                if key.len() < CX_ID_BYTES {
                    return Err(CalyxError::aster_corrupt_shard(format!(
                        "Anchors CF key has {} bytes, shorter than a CxId",
                        key.len()
                    )));
                }
                let suffix = &key[CX_ID_BYTES..];
                let Some(expectation) = expectations.get(suffix) else {
                    continue;
                };
                let id = CxId::from_bytes(
                    key[..CX_ID_BYTES]
                        .try_into()
                        .expect("CxId slice length checked"),
                );
                let event = &events[expectation.event_index];
                let target_index = event.targets.binary_search(&id).map_err(|_| {
                    grounding_conflict(format!(
                        "plan-owned anchor kind {:?} is persisted for non-plan target {id}",
                        event.anchors[expectation.anchor_index].kind
                    ))
                })?;
                let anchor = &event.anchors[expectation.anchor_index];
                if base_presence[expectation.event_index] != BasePresence::Present {
                    return Err(CalyxError::aster_corrupt_shard(format!(
                        "Anchors CF contains target {id} kind {:?} while Base is missing that event",
                        anchor.kind
                    )));
                }
                if bytes.as_slice() != expectation.expected_bytes.as_slice() {
                    let stored = encode::decode_anchor(&bytes)?;
                    if stored == *anchor {
                        return Err(CalyxError::aster_corrupt_shard(format!(
                            "Anchors CF has non-canonical bytes for target {id} kind {:?}",
                            anchor.kind
                        )));
                    }
                    return Err(CalyxError::aster_corrupt_shard(format!(
                        "Base and Anchors CF disagree for target {id} kind {:?}",
                        anchor.kind
                    )));
                }
                let event_offset = offsets[expectation.event_index];
                let target_offset = target_index
                    .checked_mul(event.anchors.len())
                    .and_then(|value| value.checked_add(expectation.anchor_index))
                    .and_then(|value| value.checked_add(event_offset))
                    .ok_or_else(|| {
                        CalyxError::aster_corrupt_shard(
                            "grounding presence-bitmap index overflow",
                        )
                    })?;
                let word = target_offset / u64::BITS as usize;
                let mask = 1_u64 << (target_offset % u64::BITS as usize);
                if seen[word] & mask != 0 {
                    return Err(CalyxError::aster_corrupt_shard(format!(
                        "Anchors CF scan returned duplicate visible row for target {id} kind {:?}",
                        anchor.kind
                    )));
                }
                seen[word] |= mask;
                present[expectation.event_index] = present[expectation.event_index]
                    .checked_add(1)
                    .ok_or_else(|| {
                        CalyxError::aster_corrupt_shard(
                            "grounding present-anchor count overflow",
                        )
                    })?;
            }
            Ok::<(), CalyxError>(())
        },
    )?;

    let mut dispositions = Vec::with_capacity(events.len());
    for (event_index, ((event, ledger_match), base)) in events
        .iter()
        .zip(ledger_matches)
        .zip(base_presence)
        .enumerate()
    {
        let expected = event.targets.len().saturating_mul(event.anchors.len());
        let stored_present = present[event_index];
        let disposition = match (
            *base,
            stored_present,
            ledger_match.current.is_some(),
            ledger_match.superseded.is_some(),
        ) {
            (BasePresence::Present, value, true, _) if value == expected => {
                PreflightDisposition::ExistingExact
            }
            (BasePresence::Missing, 0, false, false) => PreflightDisposition::Missing,
            (BasePresence::Present, value, false, true) if value == expected => {
                PreflightDisposition::MigrationRequired
            }
            _ => {
                let first_missing = (0..expected).find(|relative| {
                    let absolute = offsets[event_index] + relative;
                    seen[absolute / u64::BITS as usize] & (1_u64 << (absolute % u64::BITS as usize))
                        == 0
                });
                let missing_detail = first_missing.map_or_else(
                    || "none".to_string(),
                    |relative| {
                        let target_index = relative / event.anchors.len();
                        let anchor_index = relative % event.anchors.len();
                        format!(
                            "target={} kind={:?}",
                            event.targets[target_index], event.anchors[anchor_index].kind
                        )
                    },
                );
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "grounding event {} is incomplete: base={base:?} anchors_present={stored_present} expected={expected} first_missing={missing_detail} current_ledger_present={} superseded_ledger_present={}",
                    hex_32(&event.event_id),
                    ledger_match.current.is_some(),
                    ledger_match.superseded.is_some()
                )));
            }
        };
        dispositions.push(disposition);
    }
    Ok(dispositions)
}

fn hex_32(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

fn commit_event_locked<C: Clock>(
    vault: &AsterVault<C>,
    event: &GroundingBatchEvent,
) -> Result<LedgerRef> {
    let kind = EntryKind::Grounding;
    let subject = SubjectId::Query(event.event_id.to_vec());
    let actor = ActorId::Service(ACTOR.to_string());

    let Some(hook) = &vault.ledger_hook else {
        let mut ledger_rows = Vec::new();
        let ledger_ref = vault.stage_raw_ledger_entry_locked(
            &mut ledger_rows,
            kind,
            subject,
            event.payload.clone(),
            actor,
        )?;
        let mut rows = event_rows(vault, event, &ledger_ref)?;
        rows.extend(ledger_rows);
        vault.commit_rows_locked(&rows)?;
        return Ok(ledger_ref);
    };

    let mut guard = ledger_hook::lock_hook(hook)?;
    let staged = guard.stage_with_checkpoints(kind, subject, event.payload.clone(), actor)?;
    let ledger_ref = staged
        .first()
        .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
        .ledger_ref();
    let mut rows = event_rows(vault, event, &ledger_ref)?;
    rows.extend(staged.iter().map(|row| encode::WriteRow {
        cf: ColumnFamily::Ledger,
        key: row.key().to_vec(),
        value: row.value().to_vec(),
    }));
    vault.commit_rows_locked(&rows)?;
    for row in &staged {
        guard.commit_staged(row)?;
    }
    Ok(ledger_ref)
}

fn commit_migration_locked<C: Clock>(
    vault: &AsterVault<C>,
    event: &GroundingBatchEvent,
) -> Result<LedgerRef> {
    let kind = EntryKind::Grounding;
    let subject = SubjectId::Query(event.event_id.to_vec());
    let actor = ActorId::Service(ACTOR.to_string());

    let Some(hook) = &vault.ledger_hook else {
        let mut ledger_rows = Vec::new();
        let ledger_ref = vault.stage_raw_ledger_entry_locked(
            &mut ledger_rows,
            kind,
            subject,
            event.payload.clone(),
            actor,
        )?;
        let mut rows = migration_rows(vault, event, &ledger_ref)?;
        rows.extend(ledger_rows);
        vault.commit_rows_locked(&rows)?;
        return Ok(ledger_ref);
    };

    let mut guard = ledger_hook::lock_hook(hook)?;
    let staged = guard.stage_with_checkpoints(kind, subject, event.payload.clone(), actor)?;
    let ledger_ref = staged
        .first()
        .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
        .ledger_ref();
    let mut rows = migration_rows(vault, event, &ledger_ref)?;
    rows.extend(staged.iter().map(|row| encode::WriteRow {
        cf: ColumnFamily::Ledger,
        key: row.key().to_vec(),
        value: row.value().to_vec(),
    }));
    vault.commit_rows_locked(&rows)?;
    for row in &staged {
        guard.commit_staged(row)?;
    }
    Ok(ledger_ref)
}

fn migration_rows<C: Clock>(
    vault: &AsterVault<C>,
    event: &GroundingBatchEvent,
    ledger_ref: &LedgerRef,
) -> Result<Vec<encode::WriteRow>> {
    let snapshot = vault.snapshot();
    let reads = event
        .targets
        .iter()
        .map(|id| crate::mvcc::CfRead::new(ColumnFamily::Base, base_key(*id)))
        .collect::<Vec<_>>();
    let pinned = vault.snapshot_handle(snapshot);
    let base_values = vault
        .rows
        .read_batch(pinned.snapshot(), &reads, &vault.clock)?;
    let target_rows = event
        .targets
        .par_iter()
        .zip(base_values.into_par_iter())
        .map(|(id, base_bytes)| {
            let base_bytes = base_bytes.ok_or_else(|| {
                CalyxError::aster_corrupt_shard(format!(
                    "grounding migration target {id} disappeared from Base after preflight"
                ))
            })?;
            let mut constellation = encode::decode_constellation_base(&base_bytes)?;
            if constellation.cx_id != *id {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "Base key {id} conflicts with encoded constellation {} during migration",
                    constellation.cx_id
                )));
            }
            for anchor in &event.anchors {
                let matches = constellation
                    .anchors
                    .iter()
                    .filter(|existing| existing.kind == anchor.kind)
                    .collect::<Vec<_>>();
                if matches.as_slice() != [anchor] {
                    return Err(CalyxError::aster_corrupt_shard(format!(
                        "grounding migration lost its exact preflight anchor for target {id} kind {:?}",
                        anchor.kind
                    )));
                }
            }
            constellation.provenance = ledger_ref.clone();
            constellation.flags.ungrounded = false;
            constellation.validate_schema()?;
            Ok(encode::WriteRow {
                cf: ColumnFamily::Base,
                key: base_key(*id),
                value: encode::encode_constellation_base(&constellation)?,
            })
        })
        .collect::<Vec<Result<encode::WriteRow>>>();

    // Indexed parallel collection preserves target order. Resolve errors in that
    // same order so corrupt-state diagnostics remain deterministic.
    target_rows.into_iter().collect()
}

fn event_rows<C: Clock>(
    vault: &AsterVault<C>,
    event: &GroundingBatchEvent,
    ledger_ref: &LedgerRef,
) -> Result<Vec<encode::WriteRow>> {
    let capacity = event.targets.len().saturating_mul(1 + event.anchors.len());
    let encoded_anchors = event
        .anchors
        .iter()
        .map(|anchor| Ok((anchor.kind.clone(), encode::encode_anchor(anchor)?)))
        .collect::<Result<Vec<_>>>()?;
    let snapshot = vault.snapshot();
    let reads = event
        .targets
        .iter()
        .map(|id| crate::mvcc::CfRead::new(ColumnFamily::Base, base_key(*id)))
        .collect::<Vec<_>>();
    let pinned = vault.snapshot_handle(snapshot);
    let base_values = vault
        .rows
        .read_batch(pinned.snapshot(), &reads, &vault.clock)?;
    let target_rows = event
        .targets
        .par_iter()
        .zip(base_values.into_par_iter())
        .map(|(id, base_bytes)| {
            let base_bytes = base_bytes.ok_or_else(|| {
                CalyxError::aster_corrupt_shard(format!(
                    "grounding event target {id} disappeared from Base after preflight"
                ))
            })?;
            let mut constellation = encode::decode_constellation_base(&base_bytes)?;
            if constellation.cx_id != *id {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "Base key {id} conflicts with encoded constellation {} after preflight",
                    constellation.cx_id
                )));
            }
            constellation.anchors.extend(event.anchors.iter().cloned());
            constellation.provenance = ledger_ref.clone();
            constellation.flags.ungrounded = false;
            constellation.validate_schema()?;

            let mut rows = Vec::with_capacity(1 + encoded_anchors.len());
            rows.push(encode::WriteRow {
                cf: ColumnFamily::Base,
                key: base_key(*id),
                value: encode::encode_constellation_base(&constellation)?,
            });
            rows.extend(
                encoded_anchors
                    .iter()
                    .map(|(kind, value)| encode::WriteRow {
                        cf: ColumnFamily::Anchors,
                        key: anchor_key(*id, kind),
                        value: value.clone(),
                    }),
            );
            Ok(rows)
        })
        .collect::<Vec<Result<Vec<encode::WriteRow>>>>();

    // Preserve the original target-major, anchor-minor canonical row order and
    // return the first corrupt target by plan order, independent of scheduling.
    let mut rows = Vec::with_capacity(capacity);
    for target_result in target_rows {
        rows.extend(target_result?);
    }
    Ok(rows)
}
