//! Reverse Oracle traversal for epistemic symmetry.

#[path = "reverse_query/corpus.rs"]
mod corpus;
#[path = "reverse_query_context.rs"]
mod reverse_query_context;

use std::collections::{BTreeMap, HashMap, HashSet};

use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorValue, Clock, Constellation, LedgerRef, content_address};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use serde::Serialize;

use crate::{Cause, DomainId, ORACLE_ACTION_METADATA_KEY, OracleError, evidence_error};
use corpus::{ActionGroup, ReverseCorpus, ReverseStats};

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
    let corpus = ReverseCorpus::load(vault, &domain)?;
    let mut state = WalkState::new(answer, &domain, corpus.stats())?;
    let candidates = walk_answer(&corpus, answer, &domain, 0, &mut state)?;

    if !state.found {
        let label = answer_label(answer, &domain)?;
        if state.stats.domain_rows_scanned == 0 {
            return Err(OracleError::DomainNotFound);
        }
        return Err(OracleError::NoCausesFound {
            domain,
            answer_label: label,
        });
    }

    for candidate in candidates {
        upsert_cause(&mut state.causes, candidate, &mut state.stats);
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
    visited_answers: HashSet<String>,
    visited_actions: HashSet<String>,
    expanded_cache: HashMap<String, Vec<CauseCandidate>>,
    causes: BTreeMap<CauseKey, CauseAccumulator>,
    stats: ReverseStats,
    found: bool,
}

impl WalkState {
    fn new(
        answer: &AnchorValue,
        domain: &DomainId,
        stats: ReverseStats,
    ) -> Result<Self, OracleError> {
        Ok(Self {
            visited_answers: HashSet::from([answer_label(answer, domain)?]),
            visited_actions: action_labels_for_answer(answer),
            expanded_cache: HashMap::new(),
            causes: BTreeMap::new(),
            stats,
            found: false,
        })
    }
}

fn walk_answer(
    corpus: &ReverseCorpus,
    answer: &AnchorValue,
    domain: &DomainId,
    depth: u8,
    state: &mut WalkState,
) -> Result<Vec<CauseCandidate>, OracleError> {
    state.stats.walk_calls += 1;
    if depth > MAX_REVERSE_DEPTH {
        state.stats.depth_prunes += 1;
        return Ok(Vec::new());
    }

    let label = answer_label(answer, domain)?;
    let mut candidates = collect_structural_causes(corpus, &label, domain, state);
    let grouped = grouped_recurrence_edges(corpus, &label);
    for (action, group) in grouped {
        if state.visited_actions.contains(&action) {
            if state.expanded_cache.contains_key(&action) {
                state.stats.memo_hits += 1;
            } else {
                state.stats.cycle_skips += 1;
            }
            continue;
        }
        state.found = true;
        state.stats.matched_edges += group.grounded_count + group.provisional_count;
        let candidate = CauseCandidate {
            action_or_event: action.clone(),
            domain: group.domain(domain),
            grounded_count: group.grounded_count,
            grounded_support: corpus.action_counts(&action).grounded,
            provisional_count: group.provisional_count,
            provisional_support: corpus.action_counts(&action).provisional,
            provisional_confidence: posterior_confidence(
                group.provisional_count,
                corpus.action_counts(&action).provisional,
            ),
        };
        let has_grounded = candidate.grounded_count > 0;
        candidates.push(candidate);
        if has_grounded && depth < MAX_REVERSE_DEPTH {
            candidates.extend(expand_action(corpus, &action, domain, depth + 1, state)?);
        }
    }
    Ok(candidates)
}

fn collect_structural_causes(
    corpus: &ReverseCorpus,
    answer_label: &str,
    domain: &DomainId,
    state: &mut WalkState,
) -> Vec<CauseCandidate> {
    let mut candidates = Vec::new();
    for edge in corpus.structural_edges(answer_label) {
        state.found = true;
        state.stats.structural_matches += 1;
        let action_counts = corpus.action_counts(&edge.action_or_event);
        let co_counts = corpus.action_answer_counts(&edge.action_or_event, answer_label);
        let support = action_counts.total();
        let co_occurrences = co_counts.total().min(support);
        let measured_bound = posterior_confidence(co_occurrences, support);
        candidates.push(CauseCandidate {
            action_or_event: edge.action_or_event.clone(),
            domain: domain.clone(),
            grounded_count: 0,
            grounded_support: 0,
            provisional_count: co_occurrences,
            provisional_support: support,
            provisional_confidence: unit(edge.confidence).min(measured_bound),
        });
    }
    candidates
}

fn grouped_recurrence_edges(
    corpus: &ReverseCorpus,
    answer_label: &str,
) -> BTreeMap<String, ActionGroup> {
    let mut grouped = BTreeMap::<String, ActionGroup>::new();
    for edge in corpus.recurrence_edges(answer_label) {
        grouped.entry(edge.action_id.clone()).or_default().add(edge);
    }
    grouped
}

fn expand_action(
    corpus: &ReverseCorpus,
    action: &str,
    domain: &DomainId,
    depth: u8,
    state: &mut WalkState,
) -> Result<Vec<CauseCandidate>, OracleError> {
    if let Some(cached) = state.expanded_cache.get(action) {
        state.stats.memo_hits += 1;
        return Ok(cached.clone());
    }
    if state.visited_actions.contains(action) {
        state.stats.cycle_skips += 1;
        return Ok(Vec::new());
    }
    let next = AnchorValue::Text(action.to_string());
    let label = answer_label(&next, domain)?;
    if !state.visited_answers.insert(label.clone()) {
        state.stats.cycle_skips += 1;
        return Ok(Vec::new());
    }
    state.visited_actions.insert(action.to_string());
    state.stats.expanded_actions += 1;
    let expanded = walk_answer(corpus, &next, domain, depth, state)?;
    state.visited_answers.remove(&label);
    state
        .expanded_cache
        .insert(action.to_string(), expanded.clone());
    Ok(expanded)
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
    if candidate.grounded_count > 0 {
        stats.grounded_causes_observed += candidate.grounded_count;
        accumulator.add_grounded(candidate.grounded_count, candidate.grounded_support);
    }
    if candidate.provisional_count > 0
        || candidate.provisional_support > 0
        || candidate.provisional_confidence > 0.0
    {
        stats.provisional_causes_observed += candidate.provisional_count;
        accumulator.add_provisional(
            candidate.provisional_count,
            candidate.provisional_support,
            candidate.provisional_confidence,
        );
    }
    candidate.grounded_count > 0
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
        answer_digest: hex_bytes(&content_address([answer_label(answer, domain)?.as_bytes()])),
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
        answer_label(answer, domain)?.as_bytes(),
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

fn action_from_constellation(cx: &Constellation) -> Option<String> {
    cx.metadata_value(ORACLE_ACTION_METADATA_KEY)
        .or_else(|| cx.metadata_value(ORACLE_FALLBACK_ACTION_METADATA_KEY))
        .map(ToOwned::to_owned)
}

fn structural_confidence(cx: &Constellation) -> f32 {
    cx.metadata_value(ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY)
        .and_then(|raw| raw.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map(unit)
        .unwrap_or(STRUCTURAL_CONFIDENCE)
}

fn posterior_confidence(co_occurrences: u64, action_occurrences: u64) -> f32 {
    let support = action_occurrences.max(co_occurrences);
    (co_occurrences.saturating_add(1) as f32 / support.saturating_add(2) as f32).clamp(0.0, 1.0)
}

fn answer_label(answer: &AnchorValue, domain: &DomainId) -> Result<String, OracleError> {
    serde_json::to_string(answer).map_err(|_| evidence_error::corrupt(domain, "answer label"))
}

fn action_labels_for_answer(answer: &AnchorValue) -> HashSet<String> {
    match answer {
        AnchorValue::Text(value) | AnchorValue::Enum(value) => HashSet::from([value.clone()]),
        _ => HashSet::new(),
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
    grounded_count: u64,
    grounded_support: u64,
    provisional_count: u64,
    provisional_support: u64,
    provisional_confidence: f32,
}

#[derive(Clone, Debug)]
struct CauseAccumulator {
    action_or_event: String,
    domain: DomainId,
    grounded_count: u64,
    grounded_support: u64,
    provisional_count: u64,
    provisional_support: u64,
    provisional_confidence: f32,
}

impl CauseAccumulator {
    fn new(action_or_event: String, domain: DomainId) -> Self {
        Self {
            action_or_event,
            domain,
            grounded_count: 0,
            grounded_support: 0,
            provisional_count: 0,
            provisional_support: 0,
            provisional_confidence: 0.0,
        }
    }

    fn add_grounded(&mut self, count: u64, support: u64) {
        self.grounded_count = self.grounded_count.saturating_add(count);
        self.grounded_support = self.grounded_support.max(support).max(self.grounded_count);
    }

    fn add_provisional(&mut self, count: u64, support: u64, confidence: f32) {
        self.provisional_count = self.provisional_count.saturating_add(count);
        self.provisional_support = self
            .provisional_support
            .max(support)
            .max(self.provisional_count);
        self.provisional_confidence = self.provisional_confidence.max(unit(confidence));
    }

    fn into_cause(self) -> Cause {
        let grounded = self.grounded_count > 0;
        Cause {
            action_or_event: self.action_or_event,
            domain: self.domain,
            confidence: if grounded {
                posterior_confidence(self.grounded_count, self.grounded_support)
            } else {
                self.provisional_confidence
            },
            support: if grounded {
                self.grounded_support
            } else {
                self.provisional_support
            },
            provisional: !grounded,
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
        }
    }
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
