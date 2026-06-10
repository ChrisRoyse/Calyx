//! PH42 recurrence anchors and oracle self-consistency for Assay.

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::recurrence::{FREQUENCY_SCALAR, OccurrenceContext, RecurrenceSeries, read_series};
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{AnchorKind, AnchorValue, CalyxError, Clock, Constellation, CxId, Result};
use serde::{Deserialize, Serialize};

pub const CALYX_ASSAY_MISSING_OUTCOME_SLOT: &str = "CALYX_ASSAY_MISSING_OUTCOME_SLOT";
pub const DEFAULT_OUTCOME_ANCHOR_LABEL: &str = "OutcomeAnchor";
pub const OUTCOME_CONTEXT_FIELD: &str = "outcome_anchor";
pub const CONSISTENT_AGREEMENT_THRESHOLD: f32 = 0.75;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecurrenceAnchor {
    pub cx_id: CxId,
    pub frequency: u64,
    pub cadence_secs: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeAgreement {
    Consistent { agreement_rate: f32 },
    Flaky { agreement_rate: f32 },
    Insufficient { n: usize },
}

impl OutcomeAgreement {
    pub fn agreement_rate(&self) -> Option<f32> {
        match self {
            Self::Consistent { agreement_rate } | Self::Flaky { agreement_rate } => {
                Some(*agreement_rate)
            }
            Self::Insufficient { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Domain {
    pub id: String,
    pub cx_ids: Vec<CxId>,
    pub outcome_anchor: AnchorKind,
}

impl Domain {
    pub fn new(id: impl Into<String>, cx_ids: Vec<CxId>) -> Self {
        Self::with_outcome_anchor(id, cx_ids, default_outcome_anchor())
    }

    pub fn with_outcome_anchor(
        id: impl Into<String>,
        cx_ids: Vec<CxId>,
        outcome_anchor: AnchorKind,
    ) -> Self {
        Self {
            id: id.into(),
            cx_ids,
            outcome_anchor,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutcomeAnchorObservation {
    pub kind: AnchorKind,
    pub value: AnchorValue,
}

#[derive(Serialize, Deserialize)]
struct OutcomeContextEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome_anchor: Option<OutcomeAnchorObservation>,
}

pub fn default_outcome_anchor() -> AnchorKind {
    AnchorKind::Label(DEFAULT_OUTCOME_ANCHOR_LABEL.to_string())
}

pub fn outcome_occurrence_context(
    kind: AnchorKind,
    value: AnchorValue,
) -> Result<OccurrenceContext> {
    validate_anchor_value(&value)?;
    let envelope = OutcomeContextEnvelope {
        outcome_anchor: Some(OutcomeAnchorObservation { kind, value }),
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|error| {
        CalyxError::aster_corrupt_shard(format!("encode outcome context: {error}"))
    })?;
    OccurrenceContext::new(bytes)
}

pub fn frequency_anchor_for<C>(cx_id: CxId, vault: &AsterVault<C>) -> Result<RecurrenceAnchor>
where
    C: Clock,
{
    let base = read_base_constellation(vault, cx_id)?
        .ok_or_else(|| CalyxError::stale_derived("frequency anchor requires base row"))?;
    Ok(RecurrenceAnchor {
        cx_id,
        frequency: frequency_from_base(&base)?,
        cadence_secs: None,
    })
}

pub fn measure_outcome_agreement<C>(cx_id: CxId, vault: &AsterVault<C>) -> Result<OutcomeAgreement>
where
    C: Clock,
{
    measure_outcome_agreement_for(cx_id, vault, &default_outcome_anchor())
}

pub fn measure_outcome_agreement_for<C>(
    cx_id: CxId,
    vault: &AsterVault<C>,
    outcome_anchor: &AnchorKind,
) -> Result<OutcomeAgreement>
where
    C: Clock,
{
    let series = read_series(vault, cx_id)?;
    measure_series_outcome_agreement(&series, outcome_anchor)
}

pub fn oracle_self_consistency<C>(domain: &Domain, vault: &AsterVault<C>) -> Result<f32>
where
    C: Clock,
{
    let mut agreements = Vec::new();
    for cx_id in &domain.cx_ids {
        let anchor = frequency_anchor_for(*cx_id, vault)?;
        if anchor.frequency < 3 {
            continue;
        }
        agreements.push(measure_outcome_agreement_for(
            *cx_id,
            vault,
            &domain.outcome_anchor,
        )?);
    }
    Ok(oracle_self_consistency_from_agreements(&agreements))
}

pub fn oracle_self_consistency_from_agreements(agreements: &[OutcomeAgreement]) -> f32 {
    let mut sum = 0.0_f32;
    let mut count = 0_usize;
    for agreement in agreements {
        if let Some(rate) = agreement.agreement_rate() {
            sum += rate;
            count += 1;
        }
    }
    if count == 0 { 1.0 } else { sum / count as f32 }
}

pub fn outcome_agreement_from_observations(
    observations: &[Option<AnchorValue>],
) -> OutcomeAgreement {
    if observations.len() < 3 {
        return OutcomeAgreement::Insufficient {
            n: observations.len(),
        };
    }

    let mut agreeing = 0_usize;
    let mut total = 0_usize;
    for left in 0..observations.len() {
        for right in (left + 1)..observations.len() {
            total += 1;
            if observations[left] == observations[right] {
                agreeing += 1;
            }
        }
    }

    classify_agreement(agreeing as f32 / total as f32)
}

fn measure_series_outcome_agreement(
    series: &RecurrenceSeries,
    outcome_anchor: &AnchorKind,
) -> Result<OutcomeAgreement> {
    if series.occurrences.len() < 3 {
        return Ok(OutcomeAgreement::Insufficient {
            n: series.occurrences.len(),
        });
    }

    let mut observations = Vec::with_capacity(series.occurrences.len());
    for occurrence in &series.occurrences {
        observations.push(outcome_from_context(&occurrence.context, outcome_anchor)?);
    }
    Ok(outcome_agreement_from_observations(&observations))
}

fn outcome_from_context(
    context: &OccurrenceContext,
    expected_kind: &AnchorKind,
) -> Result<Option<AnchorValue>> {
    if context.bytes.is_empty() {
        return Ok(None);
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&context.bytes) else {
        return Ok(None);
    };
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let Some(raw) = object.get(OUTCOME_CONTEXT_FIELD) else {
        return Ok(None);
    };
    let observation: OutcomeAnchorObservation =
        serde_json::from_value(raw.clone()).map_err(|error| {
            missing_outcome_slot(format!("invalid {OUTCOME_CONTEXT_FIELD} evidence: {error}"))
        })?;
    if observation.kind != *expected_kind {
        return Err(missing_outcome_slot(format!(
            "expected outcome anchor {expected_kind:?}, found {:?}",
            observation.kind
        )));
    }
    validate_anchor_value(&observation.value)?;
    Ok(Some(observation.value))
}

fn classify_agreement(agreement_rate: f32) -> OutcomeAgreement {
    if agreement_rate >= CONSISTENT_AGREEMENT_THRESHOLD {
        OutcomeAgreement::Consistent { agreement_rate }
    } else {
        OutcomeAgreement::Flaky { agreement_rate }
    }
}

fn read_base_constellation<C>(vault: &AsterVault<C>, cx_id: CxId) -> Result<Option<Constellation>>
where
    C: Clock,
{
    vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(cx_id))?
        .map(|bytes| encode::decode_constellation_base(&bytes))
        .transpose()
}

fn frequency_from_base(base: &Constellation) -> Result<u64> {
    let Some(value) = base.scalars.get(FREQUENCY_SCALAR) else {
        return Ok(0);
    };
    if !value.is_finite() || *value < 0.0 || value.fract() != 0.0 || *value > u64::MAX as f64 {
        return Err(CalyxError::aster_corrupt_shard(
            "recurrence frequency scalar must be a non-negative integer",
        ));
    }
    Ok(*value as u64)
}

fn validate_anchor_value(value: &AnchorValue) -> Result<()> {
    let valid = match value {
        AnchorValue::Bool(_)
        | AnchorValue::Enum(_)
        | AnchorValue::OneHot(_)
        | AnchorValue::Text(_) => true,
        AnchorValue::Number(value) => value.is_finite(),
        AnchorValue::Vector(values) => values.iter().all(|value| value.is_finite()),
    };
    if valid {
        Ok(())
    } else {
        Err(missing_outcome_slot(
            "outcome anchor value must contain only finite numbers",
        ))
    }
}

fn missing_outcome_slot(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_MISSING_OUTCOME_SLOT,
        message: message.into(),
        remediation: "attach a grounded OutcomeAnchor observation for each recurring outcome",
    }
}
