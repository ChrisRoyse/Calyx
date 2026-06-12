//! Oracle self-consistency measured from grounded recurrence streams.

use std::collections::BTreeMap;

use calyx_assay::{MIN_ASSAY_SAMPLES, entropy_bits, ksg_mi_continuous_discrete};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::recurrence::{Occurrence, read_series};
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{
    AnchorValue, CalyxError, Clock, Constellation, LedgerRef, Result as CalyxResult, VaultStore,
    content_address,
};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use serde::{Deserialize, Serialize};

use crate::{DomainId, OracleError, OracleSelfConsistency};

pub const ORACLE_DOMAIN_METADATA_KEY: &str = "oracle.domain";
pub const ORACLE_FALLBACK_DOMAIN_METADATA_KEY: &str = "domain";
pub const MIN_FLAKINESS_PAIRS: u64 = 10;
pub const MIN_VALIDITY_SAMPLES: usize = MIN_ASSAY_SAMPLES;

const KSG_K: usize = 3;
const LEDGER_ACTOR: &str = "calyx-oracle";
const LEDGER_TAG: &str = "oracle_self_consistency_v1";

pub fn oracle_self_consistency<C>(
    vault: &AsterVault<C>,
    domain: DomainId,
    clock: &dyn Clock,
) -> Result<OracleSelfConsistency, OracleError>
where
    C: Clock,
{
    let series = domain_series(vault, &domain)?;
    let stats = consistency_stats(&domain, &series)?;
    let mut result = OracleSelfConsistency::with_provenance(
        stats.flakiness,
        stats.validity,
        stats.provisional,
        None,
    );
    let provenance = write_ledger(vault, &domain, &stats, &result, clock)?;
    result.provenance = Some(provenance);
    Ok(result)
}

fn domain_series<C>(
    vault: &AsterVault<C>,
    domain: &DomainId,
) -> Result<Vec<Vec<Occurrence>>, OracleError>
where
    C: Clock,
{
    let mut out = Vec::new();
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
        let recurrence = read_series(vault, cx.cx_id).map_err(|_| OracleError::NoRecurrence {
            domain: domain.clone(),
        })?;
        out.push(recurrence.occurrences);
    }
    if out.is_empty() {
        return Err(OracleError::DomainNotFound);
    }
    Ok(out)
}

fn matches_domain(cx: &Constellation, domain: &DomainId) -> bool {
    cx.metadata_value(ORACLE_DOMAIN_METADATA_KEY) == Some(domain.as_str())
        || cx.metadata_value(ORACLE_FALLBACK_DOMAIN_METADATA_KEY) == Some(domain.as_str())
}

fn consistency_stats(
    domain: &DomainId,
    series: &[Vec<Occurrence>],
) -> Result<ConsistencyStats, OracleError> {
    let mut total_pairs = 0_u64;
    let mut agreement_pairs = 0_u64;
    let mut validity_samples = Vec::new();

    for occurrences in series {
        let observations = observations_from_series(domain, occurrences)?;
        let mut counts = BTreeMap::<String, u64>::new();
        for observation in &observations {
            *counts.entry(observation.verdict.clone()).or_default() += 1;
            if let Some(truth) = &observation.ground_truth {
                validity_samples.push(ValiditySample {
                    verdict: observation.verdict.clone(),
                    ground_truth: truth.clone(),
                });
            }
        }
        let n = observations.len() as u64;
        total_pairs += pair_count(n);
        agreement_pairs += counts.values().map(|count| pair_count(*count)).sum::<u64>();
    }

    if total_pairs < MIN_FLAKINESS_PAIRS {
        return Err(OracleError::NoRecurrence {
            domain: domain.clone(),
        });
    }

    let flakiness = 1.0 - (agreement_pairs as f32 / total_pairs as f32);
    let (validity, provisional) = validity(domain, &validity_samples)?;
    Ok(ConsistencyStats {
        pair_count: total_pairs,
        agreement_pairs,
        validity_samples: validity_samples.len(),
        flakiness: flakiness.clamp(0.0, 1.0),
        validity: validity.clamp(0.0, 1.0),
        provisional,
    })
}

fn observations_from_series(
    domain: &DomainId,
    occurrences: &[Occurrence],
) -> Result<Vec<OracleObservation>, OracleError> {
    let mut out = Vec::new();
    for occurrence in occurrences {
        if occurrence.context.bytes.is_empty() {
            continue;
        }
        let parsed: RecurrenceEvidence = serde_json::from_slice(&occurrence.context.bytes)
            .map_err(|_| OracleError::NoRecurrence {
                domain: domain.clone(),
            })?;
        let Some(verdict) = parsed
            .verdict_label()
            .map_err(|_| OracleError::NoRecurrence {
                domain: domain.clone(),
            })?
        else {
            continue;
        };
        out.push(OracleObservation {
            verdict,
            ground_truth: parsed
                .ground_truth_label()
                .map_err(|_| OracleError::NoRecurrence {
                    domain: domain.clone(),
                })?,
        });
    }
    Ok(out)
}

fn validity(domain: &DomainId, samples: &[ValiditySample]) -> Result<(f32, bool), OracleError> {
    if samples.is_empty() {
        return Ok((0.0, true));
    }
    if samples.len() < MIN_VALIDITY_SAMPLES {
        return Err(OracleError::NoRecurrence {
            domain: domain.clone(),
        });
    }
    if samples
        .iter()
        .all(|sample| sample.verdict == sample.ground_truth)
    {
        return Ok((1.0, false));
    }

    let truth_codes = label_codes(samples.iter().map(|sample| &sample.ground_truth));
    let entropy = entropy_bits(&truth_codes);
    if entropy <= f32::EPSILON {
        let matches = samples
            .iter()
            .filter(|sample| sample.verdict == sample.ground_truth)
            .count();
        return Ok((matches as f32 / samples.len() as f32, false));
    }

    let verdict_index = label_index(samples.iter().map(|sample| &sample.verdict));
    let x = samples
        .iter()
        .map(|sample| one_hot(verdict_index[&sample.verdict], verdict_index.len()))
        .collect::<Vec<_>>();
    let estimate = ksg_mi_continuous_discrete(&x, &truth_codes, KSG_K).map_err(|_| {
        OracleError::NoRecurrence {
            domain: domain.clone(),
        }
    })?;
    Ok(((estimate.bits / entropy).clamp(0.0, 1.0), false))
}

fn write_ledger<C>(
    vault: &AsterVault<C>,
    domain: &DomainId,
    stats: &ConsistencyStats,
    result: &OracleSelfConsistency,
    clock: &dyn Clock,
) -> Result<LedgerRef, OracleError>
where
    C: Clock,
{
    let digest = domain_digest(domain);
    let payload = MeasurementPayload::new(domain, stats, result, clock.now());
    let bytes = serde_json::to_vec(&payload).map_err(|_| OracleError::LedgerWriteFailure)?;
    vault
        .append_ledger_entry(
            EntryKind::Assay,
            SubjectId::Query(digest.to_vec()),
            bytes,
            ActorId::Service(LEDGER_ACTOR.to_string()),
        )
        .map_err(|_| OracleError::LedgerWriteFailure)
}

#[derive(Clone, Debug, PartialEq)]
struct OracleObservation {
    verdict: String,
    ground_truth: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct ValiditySample {
    verdict: String,
    ground_truth: String,
}

#[derive(Clone, Debug, PartialEq)]
struct ConsistencyStats {
    pair_count: u64,
    agreement_pairs: u64,
    validity_samples: usize,
    flakiness: f32,
    validity: f32,
    provisional: bool,
}

#[derive(Clone, Debug, Serialize)]
struct MeasurementPayload {
    tag: &'static str,
    domain_id: String,
    pair_count: u64,
    agreement_pairs: u64,
    validity_samples: u64,
    flakiness: f32,
    validity: f32,
    ceiling: f32,
    provisional: bool,
    ts: u64,
}

impl MeasurementPayload {
    fn new(
        domain: &DomainId,
        stats: &ConsistencyStats,
        result: &OracleSelfConsistency,
        ts: u64,
    ) -> Self {
        Self {
            tag: LEDGER_TAG,
            domain_id: hex_bytes(&domain_digest(domain)),
            pair_count: stats.pair_count,
            agreement_pairs: stats.agreement_pairs,
            validity_samples: stats.validity_samples as u64,
            flakiness: stats.flakiness,
            validity: stats.validity,
            ceiling: result.ceiling,
            provisional: stats.provisional,
            ts,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct RecurrenceEvidence {
    #[serde(default, rename = "oracle_verdict")]
    oracle_verdict: Option<AnchorEvidence>,
    #[serde(default, rename = "outcome_anchor")]
    outcome_anchor: Option<AnchorEvidence>,
    #[serde(default, rename = "ground_truth_anchor")]
    ground_truth_anchor: Option<AnchorEvidence>,
}

impl RecurrenceEvidence {
    fn verdict_label(&self) -> CalyxResult<Option<String>> {
        self.oracle_verdict
            .as_ref()
            .or(self.outcome_anchor.as_ref())
            .map(AnchorEvidence::label)
            .transpose()
    }

    fn ground_truth_label(&self) -> CalyxResult<Option<String>> {
        self.ground_truth_anchor
            .as_ref()
            .map(AnchorEvidence::label)
            .transpose()
    }
}

#[derive(Clone, Debug, Deserialize)]
struct AnchorEvidence {
    value: AnchorValue,
}

impl AnchorEvidence {
    fn label(&self) -> CalyxResult<String> {
        serde_json::to_string(&self.value).map_err(|error| {
            CalyxError::aster_corrupt_shard(format!("anchor label encode: {error}"))
        })
    }
}

fn pair_count(n: u64) -> u64 {
    n.saturating_mul(n.saturating_sub(1)) / 2
}

fn label_index<'a>(labels: impl Iterator<Item = &'a String>) -> BTreeMap<String, usize> {
    let mut index = BTreeMap::new();
    for label in labels {
        let next = index.len();
        index.entry(label.clone()).or_insert(next);
    }
    index
}

fn label_codes<'a>(labels: impl Iterator<Item = &'a String>) -> Vec<usize> {
    let mut index = BTreeMap::new();
    let mut out = Vec::new();
    for label in labels {
        let next = index.len();
        out.push(*index.entry(label.clone()).or_insert(next));
    }
    out
}

fn one_hot(index: usize, len: usize) -> Vec<f32> {
    let mut out = vec![0.0; len];
    out[index] = 1.0;
    out
}

fn domain_digest(domain: &DomainId) -> [u8; 16] {
    content_address([domain.as_str().as_bytes()])
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "self_consistency_tests.rs"]
mod tests;
