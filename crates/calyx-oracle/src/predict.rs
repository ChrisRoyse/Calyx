//! Vault-backed Oracle consequence prediction.

mod context;

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::recurrence::read_series;
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{
    AnchorValue, Clock, Constellation, LedgerRef, Panel, VaultStore, content_address,
};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use calyx_ward::{DEFAULT_TAU, GuardId, GuardVerdict, SlotVerdict};
use serde::{Deserialize, Serialize};

use context::{ConsequenceSeed, PredictionContext};

use crate::{
    Consequence, DomainId, OracleError, Prediction, SufficiencyBound, check_sufficiency,
    oracle_self_consistency,
};
use crate::{ORACLE_DOMAIN_METADATA_KEY, ORACLE_FALLBACK_DOMAIN_METADATA_KEY};

pub const ORACLE_ACTION_METADATA_KEY: &str = "oracle.action";
const ORACLE_FALLBACK_ACTION_METADATA_KEY: &str = "action";
const LEDGER_ACTOR: &str = "calyx-oracle";
const LEDGER_TAG: &str = "oracle_predict_v1";
const HOP_ATTENUATION: f32 = 0.7;
const PROVISIONAL_GUARD_ID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c104";

/// Action plus the exact `panel_t` snapshot whose sufficiency gates prediction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub action_id: String,
    pub panel: Panel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<GuardVerdict>,
}

pub fn oracle_predict<C>(
    vault: &AsterVault<C>,
    action: &Action,
    domain: DomainId,
    clock: &dyn Clock,
) -> Result<Prediction, OracleError>
where
    C: Clock,
{
    let bound = check_sufficiency(vault, &action.panel, domain.clone(), clock)?;
    let evidence = prediction_evidence(vault, &action.action_id, &domain)?;
    let posterior = posterior(&evidence.observations, &domain)?;
    let consistency = oracle_self_consistency(vault, domain.clone(), clock)?;
    let confidence = apply_confidence_ceiling(
        posterior.raw_confidence,
        consistency.ceiling,
        bound.dpi_ceiling,
    );
    let guard = action
        .guard
        .clone()
        .unwrap_or_else(|| provisional_guard(&action.panel));
    let ledger_ref = write_prediction_ledger(
        vault,
        LedgerWriteInput {
            domain: &domain,
            action,
            evidence: &evidence,
            posterior: &posterior,
            bound: &bound,
            self_consistency_ceiling: consistency.ceiling,
            confidence,
            clock,
        },
    )?;
    let consequences = first_order_consequences(
        &evidence.observations,
        &posterior.outcome_label,
        confidence,
        &ledger_ref,
    );
    Ok(Prediction {
        outcome: posterior.outcome,
        confidence,
        consequences,
        bound,
        provenance: ledger_ref,
        guard,
    })
}

fn prediction_evidence<C>(
    vault: &AsterVault<C>,
    action_id: &str,
    domain: &DomainId,
) -> Result<PredictionEvidence, OracleError>
where
    C: Clock,
{
    let mut observations = Vec::new();
    let mut source_cx_ids = BTreeSet::new();
    for (_, bytes) in vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .map_err(|_| OracleError::NoRecurrence {
            domain: domain.clone(),
        })?
    {
        let cx =
            encode::decode_constellation_base(&bytes).map_err(|_| OracleError::NoRecurrence {
                domain: domain.clone(),
            })?;
        if !matches_domain(&cx, domain) {
            continue;
        }
        let base_action_match = matches_action(&cx, action_id);
        collect_series(
            vault,
            &cx,
            action_id,
            domain,
            base_action_match,
            &mut observations,
        )?;
        if observations
            .iter()
            .any(|row| row.cx_id == cx.cx_id.to_string())
        {
            source_cx_ids.insert(cx.cx_id.to_string());
        }
    }
    if observations.is_empty() {
        return Err(OracleError::NoRecurrence {
            domain: domain.clone(),
        });
    }
    Ok(PredictionEvidence {
        source_cx_ids: source_cx_ids.into_iter().collect(),
        observations,
    })
}

fn collect_series<C>(
    vault: &AsterVault<C>,
    cx: &Constellation,
    action_id: &str,
    domain: &DomainId,
    base_action_match: bool,
    observations: &mut Vec<OutcomeObservation>,
) -> Result<(), OracleError>
where
    C: Clock,
{
    let series = read_series(vault, cx.cx_id).map_err(|_| OracleError::NoRecurrence {
        domain: domain.clone(),
    })?;
    for occurrence in &series.occurrences {
        if occurrence.context.bytes.is_empty() {
            continue;
        }
        let parsed: PredictionContext =
            serde_json::from_slice(&occurrence.context.bytes).map_err(|_| {
                OracleError::NoRecurrence {
                    domain: domain.clone(),
                }
            })?;
        if !parsed.matches_action(action_id, base_action_match) {
            continue;
        }
        let Some(outcome) = parsed.outcome() else {
            continue;
        };
        let label = outcome_label(&outcome).map_err(|_| OracleError::NoRecurrence {
            domain: domain.clone(),
        })?;
        observations.push(OutcomeObservation {
            cx_id: cx.cx_id.to_string(),
            outcome,
            outcome_label: label,
            consequences: parsed.consequences(),
        });
    }
    Ok(())
}

fn posterior(
    observations: &[OutcomeObservation],
    domain: &DomainId,
) -> Result<OutcomePosterior, OracleError> {
    let mut buckets = BTreeMap::<String, OutcomeBucket>::new();
    for observation in observations {
        let bucket = buckets
            .entry(observation.outcome_label.clone())
            .or_insert_with(|| OutcomeBucket {
                label: observation.outcome_label.clone(),
                outcome: observation.outcome.clone(),
                count: 0,
            });
        bucket.count += 1;
    }
    let mut ranked = buckets.into_values().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.label.cmp(&right.label))
    });
    let Some(top) = ranked.first().cloned() else {
        return Err(OracleError::NoRecurrence {
            domain: domain.clone(),
        });
    };
    let second_count = ranked.get(1).map_or(0, |bucket| bucket.count);
    let total = observations.len() as u64;
    let raw_confidence = raw_confidence(top.count, second_count, total);
    Ok(OutcomePosterior {
        outcome: top.outcome,
        outcome_label: top.label,
        total,
        top_count: top.count,
        second_count,
        distinct_outcomes: ranked.len() as u64,
        raw_confidence,
    })
}

fn raw_confidence(top_count: u64, second_count: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }
    let total = total as f32;
    let support = top_count as f32 / total;
    let separation = top_count.saturating_sub(second_count) as f32 / total;
    let sample_support = total / (total + 2.0);
    (support * separation * sample_support).clamp(0.0, 1.0)
}

fn apply_confidence_ceiling(raw: f32, self_consistency: f32, dpi_ceiling: f32) -> f32 {
    unit(raw).min(unit(self_consistency)).min(unit(dpi_ceiling))
}

fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn first_order_consequences(
    observations: &[OutcomeObservation],
    predicted_label: &str,
    confidence: f32,
    provenance: &LedgerRef,
) -> Vec<Consequence> {
    let mut buckets = BTreeMap::<ConsequenceKey, ConsequenceBucket>::new();
    let predicted_count = observations
        .iter()
        .filter(|observation| observation.outcome_label == predicted_label)
        .count()
        .max(1) as f32;
    for observation in observations
        .iter()
        .filter(|observation| observation.outcome_label == predicted_label)
    {
        for consequence in &observation.consequences {
            let Ok(label) = outcome_label(&consequence.outcome) else {
                continue;
            };
            let key = ConsequenceKey {
                action_or_event: consequence.action_or_event.clone(),
                domain: consequence.domain.clone(),
                outcome_label: label,
            };
            let bucket = buckets.entry(key).or_insert_with(|| ConsequenceBucket {
                outcome: consequence.outcome.clone(),
                count: 0,
            });
            bucket.count += 1;
        }
    }
    buckets
        .into_iter()
        .map(|(key, bucket)| Consequence {
            action_or_event: key.action_or_event,
            domain: DomainId::from(key.domain),
            outcome: bucket.outcome,
            confidence: (confidence * HOP_ATTENUATION * bucket.count as f32 / predicted_count)
                .clamp(0.0, confidence),
            hop: 1,
            provenance: provenance.clone(),
        })
        .collect()
}

fn write_prediction_ledger<C>(
    vault: &AsterVault<C>,
    input: LedgerWriteInput<'_>,
) -> Result<LedgerRef, OracleError>
where
    C: Clock,
{
    let subject = prediction_digest(
        input.domain,
        &input.action.action_id,
        &input.posterior.outcome_label,
    );
    let payload = PredictionLedgerPayload {
        tag: LEDGER_TAG,
        domain_id: hex_bytes(&domain_digest(input.domain)),
        action_id: input.action.action_id.clone(),
        action_digest: hex_bytes(&content_address([input.action.action_id.as_bytes()])),
        outcome_digest: hex_bytes(&content_address([input.posterior.outcome_label.as_bytes()])),
        source_cx_ids: input.evidence.source_cx_ids.clone(),
        recurrence_observations: input.posterior.total,
        top_count: input.posterior.top_count,
        second_count: input.posterior.second_count,
        distinct_outcomes: input.posterior.distinct_outcomes,
        raw_confidence: input.posterior.raw_confidence,
        self_consistency_ceiling: input.self_consistency_ceiling,
        dpi_ceiling: input.bound.dpi_ceiling,
        confidence: input.confidence,
        ts: input.clock.now(),
    };
    let bytes = serde_json::to_vec(&payload).map_err(|_| OracleError::LedgerWriteFailure)?;
    vault
        .append_ledger_entry(
            EntryKind::Answer,
            SubjectId::Query(subject.to_vec()),
            bytes,
            ActorId::Service(LEDGER_ACTOR.to_string()),
        )
        .map_err(|_| OracleError::LedgerWriteFailure)
}

fn provisional_guard(panel: &Panel) -> GuardVerdict {
    GuardVerdict {
        guard_id: GuardId::from_str(PROVISIONAL_GUARD_ID).expect("static guard id"),
        overall_pass: true,
        provisional: true,
        per_slot: panel
            .slots
            .iter()
            .map(|slot| SlotVerdict {
                slot: slot.slot_id,
                cos: 1.0,
                tau: DEFAULT_TAU,
                pass: true,
            })
            .collect(),
        action: None,
    }
}

fn matches_domain(cx: &Constellation, domain: &DomainId) -> bool {
    cx.metadata_value(ORACLE_DOMAIN_METADATA_KEY) == Some(domain.as_str())
        || cx.metadata_value(ORACLE_FALLBACK_DOMAIN_METADATA_KEY) == Some(domain.as_str())
}

fn matches_action(cx: &Constellation, action_id: &str) -> bool {
    cx.metadata_value(ORACLE_ACTION_METADATA_KEY) == Some(action_id)
        || cx.metadata_value(ORACLE_FALLBACK_ACTION_METADATA_KEY) == Some(action_id)
}

fn outcome_label(value: &AnchorValue) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

fn prediction_digest(domain: &DomainId, action_id: &str, outcome_label: &str) -> [u8; 16] {
    content_address([
        domain.as_str().as_bytes(),
        action_id.as_bytes(),
        outcome_label.as_bytes(),
    ])
}

fn domain_digest(domain: &DomainId) -> [u8; 16] {
    content_address([domain.as_str().as_bytes()])
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, PartialEq)]
struct PredictionEvidence {
    source_cx_ids: Vec<String>,
    observations: Vec<OutcomeObservation>,
}
#[derive(Clone, Debug, PartialEq)]
struct OutcomeObservation {
    cx_id: String,
    outcome: AnchorValue,
    outcome_label: String,
    consequences: Vec<ConsequenceSeed>,
}
#[derive(Clone, Debug, PartialEq)]
struct OutcomeBucket {
    label: String,
    outcome: AnchorValue,
    count: u64,
}
#[derive(Clone, Debug, PartialEq)]
struct OutcomePosterior {
    outcome: AnchorValue,
    outcome_label: String,
    total: u64,
    top_count: u64,
    second_count: u64,
    distinct_outcomes: u64,
    raw_confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ConsequenceKey {
    action_or_event: String,
    domain: String,
    outcome_label: String,
}

#[derive(Clone, Debug, PartialEq)]
struct ConsequenceBucket {
    outcome: AnchorValue,
    count: usize,
}

#[derive(Clone, Debug, Serialize)]
struct PredictionLedgerPayload {
    tag: &'static str,
    domain_id: String,
    action_id: String,
    action_digest: String,
    outcome_digest: String,
    source_cx_ids: Vec<String>,
    recurrence_observations: u64,
    top_count: u64,
    second_count: u64,
    distinct_outcomes: u64,
    raw_confidence: f32,
    self_consistency_ceiling: f32,
    dpi_ceiling: f32,
    confidence: f32,
    ts: u64,
}

struct LedgerWriteInput<'a> {
    domain: &'a DomainId,
    action: &'a Action,
    evidence: &'a PredictionEvidence,
    posterior: &'a OutcomePosterior,
    bound: &'a SufficiencyBound,
    self_consistency_ceiling: f32,
    confidence: f32,
    clock: &'a dyn Clock,
}

#[cfg(test)]
#[path = "predict_tests.rs"]
mod tests;
