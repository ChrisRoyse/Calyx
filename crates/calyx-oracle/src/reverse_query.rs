//! Reverse Oracle traversal for epistemic symmetry.

#[path = "reverse_query_context.rs"]
mod reverse_query_context;

use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::recurrence::read_series;
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{AnchorValue, Clock, Constellation, LedgerRef, VaultStore, content_address};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use serde::Serialize;

use crate::{
    Cause, DomainId, ORACLE_ACTION_METADATA_KEY, ORACLE_DOMAIN_METADATA_KEY,
    ORACLE_FALLBACK_DOMAIN_METADATA_KEY, OracleError,
};
use reverse_query_context::ReverseContext;

pub const MAX_REVERSE_DEPTH: u8 = 3;
pub const ORACLE_EFFECT_METADATA_KEY: &str = "oracle.effect";
pub const ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY: &str = "oracle.structural_confidence";

const ORACLE_FALLBACK_ACTION_METADATA_KEY: &str = "action";
const LEDGER_ACTOR: &str = "calyx-oracle";
const LEDGER_TAG: &str = "reverse_query_v1";
const STRUCTURAL_CONFIDENCE: f32 = 0.35;

pub fn reverse_query<C>(
    vault: &AsterVault<C>,
    answer: &AnchorValue,
    domain: DomainId,
    clock: &dyn Clock,
) -> Result<Vec<Cause>, OracleError>
where
    C: Clock,
{
    let mut state = WalkState::new(answer)?;
    walk_answer(vault, answer, &domain, 0, &mut state)?;

    if !state.found {
        return Err(OracleError::DomainNotFound);
    }

    let stats = state.stats.clone();
    let mut out = state
        .causes
        .into_values()
        .map(CauseAccumulator::into_cause)
        .collect::<Vec<_>>();
    sort_causes(&mut out);
    let ledger_ref = write_reverse_ledger(vault, answer, &domain, &out, &stats, clock)?;
    for cause in &mut out {
        cause.provenance = ledger_ref.clone();
    }
    Ok(out)
}

struct WalkState {
    visited_answers: BTreeSet<String>,
    visited_actions: BTreeSet<String>,
    causes: BTreeMap<CauseKey, CauseAccumulator>,
    stats: ReverseStats,
    found: bool,
}

impl WalkState {
    fn new(answer: &AnchorValue) -> Result<Self, OracleError> {
        Ok(Self {
            visited_answers: BTreeSet::from([answer_label(answer)?]),
            visited_actions: action_labels_for_answer(answer),
            causes: BTreeMap::new(),
            stats: ReverseStats::default(),
            found: false,
        })
    }
}

fn walk_answer<C>(
    vault: &AsterVault<C>,
    answer: &AnchorValue,
    domain: &DomainId,
    depth: u8,
    state: &mut WalkState,
) -> Result<(), OracleError>
where
    C: Clock,
{
    state.stats.walk_calls += 1;
    if depth > MAX_REVERSE_DEPTH {
        state.stats.depth_prunes += 1;
        return Ok(());
    }

    let rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .map_err(|_| OracleError::NoRecurrence {
            domain: domain.clone(),
        })?;
    state.stats.base_rows_scanned += rows.len() as u64;

    for (_, bytes) in rows {
        let cx =
            encode::decode_constellation_base(&bytes).map_err(|_| OracleError::NoRecurrence {
                domain: domain.clone(),
            })?;
        if !matches_domain(&cx, domain) {
            continue;
        }
        state.stats.domain_rows_scanned += 1;
        collect_structural_cause(&cx, answer, domain, state);
        collect_recurrence_causes(vault, &cx, answer, domain, depth, state)?;
    }
    Ok(())
}

fn collect_recurrence_causes<C>(
    vault: &AsterVault<C>,
    cx: &Constellation,
    answer: &AnchorValue,
    domain: &DomainId,
    depth: u8,
    state: &mut WalkState,
) -> Result<(), OracleError>
where
    C: Clock,
{
    let series = read_series(vault, cx.cx_id).map_err(|_| OracleError::NoRecurrence {
        domain: domain.clone(),
    })?;
    state.stats.recurrence_rows_scanned += series.occurrences.len() as u64;
    let base_action = action_from_constellation(cx);
    for occurrence in &series.occurrences {
        if occurrence.context.bytes.is_empty() {
            continue;
        }
        let parsed: ReverseContext =
            serde_json::from_slice(&occurrence.context.bytes).map_err(|_| {
                OracleError::NoRecurrence {
                    domain: domain.clone(),
                }
            })?;
        for edge in parsed.edges() {
            if !edge.matches_answer(answer, domain) {
                continue;
            }
            let Some(action) = parsed
                .action()
                .or(base_action.as_deref())
                .filter(|value| !value.trim().is_empty())
            else {
                continue;
            };
            if state.visited_actions.contains(action) {
                state.stats.cycle_skips += 1;
                continue;
            }
            state.found = true;
            state.stats.matched_edges += 1;
            let candidate = CauseCandidate {
                action_or_event: action.to_string(),
                domain: edge.domain_id(),
                provisional: !edge.is_grounded(),
                confidence: grounded_confidence(1),
            };
            let inserted_grounded = upsert_cause(&mut state.causes, candidate, &mut state.stats);
            maybe_walk_antecedent(vault, action, domain, depth, inserted_grounded, state)?;
        }
    }
    Ok(())
}

fn maybe_walk_antecedent<C>(
    vault: &AsterVault<C>,
    action: &str,
    domain: &DomainId,
    depth: u8,
    grounded: bool,
    state: &mut WalkState,
) -> Result<(), OracleError>
where
    C: Clock,
{
    if !grounded || depth >= MAX_REVERSE_DEPTH {
        return Ok(());
    }
    if state.visited_actions.contains(action) {
        state.stats.cycle_skips += 1;
        return Ok(());
    }
    let next = AnchorValue::Text(action.to_string());
    let label = answer_label(&next)?;
    if !state.visited_answers.insert(label) {
        state.stats.cycle_skips += 1;
        return Ok(());
    }
    state.visited_actions.insert(action.to_string());
    walk_answer(vault, &next, domain, depth.saturating_add(1), state)?;
    state.visited_answers.remove(&answer_label(&next)?);
    state.visited_actions.remove(action);
    Ok(())
}

fn collect_structural_cause(
    cx: &Constellation,
    answer: &AnchorValue,
    domain: &DomainId,
    state: &mut WalkState,
) {
    if !structural_answer_matches(cx, answer) {
        return;
    }
    let Some(action) = action_from_constellation(cx) else {
        return;
    };
    state.found = true;
    state.stats.structural_matches += 1;
    let confidence = structural_confidence(cx);
    let candidate = CauseCandidate {
        action_or_event: action,
        domain: domain.clone(),
        provisional: true,
        confidence,
    };
    upsert_cause(&mut state.causes, candidate, &mut state.stats);
}

fn upsert_cause(
    causes: &mut BTreeMap<CauseKey, CauseAccumulator>,
    candidate: CauseCandidate,
    stats: &mut ReverseStats,
) -> bool {
    let key = CauseKey::new(&candidate.domain, &candidate.action_or_event);
    let accumulator = causes
        .entry(key)
        .or_insert_with(|| CauseAccumulator::new(candidate.action_or_event, candidate.domain));
    if candidate.provisional {
        stats.provisional_causes_observed += 1;
        accumulator.add_provisional(candidate.confidence);
        false
    } else {
        stats.grounded_causes_observed += 1;
        accumulator.add_grounded();
        true
    }
}

fn write_reverse_ledger<C>(
    vault: &AsterVault<C>,
    answer: &AnchorValue,
    domain: &DomainId,
    causes: &[Cause],
    stats: &ReverseStats,
    clock: &dyn Clock,
) -> Result<LedgerRef, OracleError>
where
    C: Clock,
{
    let payload = ReverseLedgerPayload {
        tag: LEDGER_TAG,
        domain: domain.as_str().to_string(),
        answer_digest: hex_bytes(&content_address([answer_label(answer)?.as_bytes()])),
        cause_count: causes.len(),
        grounded_count: causes.iter().filter(|cause| !cause.provisional).count(),
        provisional_count: causes.iter().filter(|cause| cause.provisional).count(),
        cause_digests: causes
            .iter()
            .map(|cause| hex_bytes(&content_address([cause.action_or_event.as_bytes()])))
            .collect(),
        max_reverse_depth: MAX_REVERSE_DEPTH,
        stats: stats.clone(),
        ts: clock.now(),
    };
    let bytes = serde_json::to_vec(&payload).map_err(|_| OracleError::LedgerWriteFailure)?;
    vault
        .append_ledger_entry(
            EntryKind::Answer,
            SubjectId::Query(reverse_subject(domain, answer)?.to_vec()),
            bytes,
            ActorId::Service(LEDGER_ACTOR.to_string()),
        )
        .map_err(|_| OracleError::LedgerWriteFailure)
}

fn reverse_subject(domain: &DomainId, answer: &AnchorValue) -> Result<[u8; 16], OracleError> {
    Ok(content_address([
        domain.as_str().as_bytes(),
        answer_label(answer)?.as_bytes(),
        LEDGER_TAG.as_bytes(),
    ]))
}

fn sort_causes(causes: &mut [Cause]) {
    causes.sort_by(|left, right| {
        left.provisional
            .cmp(&right.provisional)
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| left.action_or_event.cmp(&right.action_or_event))
            .then_with(|| left.domain.cmp(&right.domain))
    });
}

fn structural_answer_matches(cx: &Constellation, answer: &AnchorValue) -> bool {
    cx.anchors.iter().any(|anchor| &anchor.value == answer)
        || cx
            .metadata_value(ORACLE_EFFECT_METADATA_KEY)
            .is_some_and(|value| metadata_anchor_matches(value, answer))
}

fn metadata_anchor_matches(raw: &str, answer: &AnchorValue) -> bool {
    serde_json::from_str::<AnchorValue>(raw).is_ok_and(|value| value == *answer)
        || matches!(answer, AnchorValue::Text(text) | AnchorValue::Enum(text) if raw == text)
}

fn action_from_constellation(cx: &Constellation) -> Option<String> {
    cx.metadata_value(ORACLE_ACTION_METADATA_KEY)
        .or_else(|| cx.metadata_value(ORACLE_FALLBACK_ACTION_METADATA_KEY))
        .map(ToOwned::to_owned)
}

fn matches_domain(cx: &Constellation, domain: &DomainId) -> bool {
    cx.metadata_value(ORACLE_DOMAIN_METADATA_KEY) == Some(domain.as_str())
        || cx.metadata_value(ORACLE_FALLBACK_DOMAIN_METADATA_KEY) == Some(domain.as_str())
}

fn structural_confidence(cx: &Constellation) -> f32 {
    cx.metadata_value(ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY)
        .and_then(|raw| raw.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map(unit)
        .unwrap_or(STRUCTURAL_CONFIDENCE)
}

fn grounded_confidence(count: u64) -> f32 {
    count as f32 / (count.saturating_add(1) as f32)
}

fn answer_label(answer: &AnchorValue) -> Result<String, OracleError> {
    serde_json::to_string(answer).map_err(|_| OracleError::NoRecurrence {
        domain: DomainId::from("unknown"),
    })
}

fn action_labels_for_answer(answer: &AnchorValue) -> BTreeSet<String> {
    match answer {
        AnchorValue::Text(value) | AnchorValue::Enum(value) => BTreeSet::from([value.clone()]),
        _ => BTreeSet::new(),
    }
}

fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CauseKey {
    domain: DomainId,
    action_or_event: String,
}

impl CauseKey {
    fn new(domain: &DomainId, action_or_event: &str) -> Self {
        Self {
            domain: domain.clone(),
            action_or_event: action_or_event.to_string(),
        }
    }
}

#[derive(Clone, Debug)]
struct CauseCandidate {
    action_or_event: String,
    domain: DomainId,
    provisional: bool,
    confidence: f32,
}

#[derive(Clone, Debug)]
struct CauseAccumulator {
    action_or_event: String,
    domain: DomainId,
    grounded_count: u64,
    provisional_confidence: f32,
}

impl CauseAccumulator {
    fn new(action_or_event: String, domain: DomainId) -> Self {
        Self {
            action_or_event,
            domain,
            grounded_count: 0,
            provisional_confidence: 0.0,
        }
    }

    fn add_grounded(&mut self) {
        self.grounded_count = self.grounded_count.saturating_add(1);
    }

    fn add_provisional(&mut self, confidence: f32) {
        self.provisional_confidence = self.provisional_confidence.max(unit(confidence));
    }

    fn into_cause(self) -> Cause {
        let grounded = self.grounded_count > 0;
        Cause {
            action_or_event: self.action_or_event,
            domain: self.domain,
            confidence: if grounded {
                grounded_confidence(self.grounded_count)
            } else {
                self.provisional_confidence
            },
            provisional: !grounded,
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
struct ReverseStats {
    walk_calls: u64,
    base_rows_scanned: u64,
    domain_rows_scanned: u64,
    recurrence_rows_scanned: u64,
    matched_edges: u64,
    structural_matches: u64,
    grounded_causes_observed: u64,
    provisional_causes_observed: u64,
    cycle_skips: u64,
    depth_prunes: u64,
}

#[derive(Clone, Debug, Serialize)]
struct ReverseLedgerPayload {
    tag: &'static str,
    domain: String,
    answer_digest: String,
    cause_count: usize,
    grounded_count: usize,
    provisional_count: usize,
    cause_digests: Vec<String>,
    max_reverse_depth: u8,
    stats: ReverseStats,
    ts: u64,
}

#[cfg(test)]
#[path = "reverse_query_tests.rs"]
mod tests;
