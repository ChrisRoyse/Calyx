use std::collections::{BTreeMap, BTreeSet};

use calyx_assay::{Direction, TEResult};
use calyx_aster::cf::{ColumnFamily, base_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::encode;
use calyx_core::{
    AnchorValue, Asymmetry, Constellation, CxFlags, CxId, InputRef, LedgerRef, LensId, Modality,
    Panel, QuantPolicy, Slot, SlotId, SlotShape, SlotState, SystemClock, content_address,
};
use calyx_oracle::{
    ConsequenceTree, DomainId, ORACLE_ACTION_METADATA_KEY, ORACLE_DOMAIN_METADATA_KEY,
};
use serde_json::{Value, json};

use crate::learner_origin::model::OracleForecastRequest;

use super::super::{OriginError, ensure_nonempty, sha256_array, storage_error};
use super::{
    ORACLE_FORECAST_EVIDENCE_KIND, ORACLE_FORECAST_GRAPH_KIND, ORACLE_FORECAST_PANEL_VERSION,
};

pub(super) type OracleGraphRows = Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>;

pub(super) fn build_oracle_panel(
    request: &OracleForecastRequest,
    now: u64,
) -> Result<Panel, OriginError> {
    let concepts = if request.panel_concepts.is_empty() {
        vec![request.action_id.clone()]
    } else {
        request.panel_concepts.clone()
    };
    if concepts.len() > 256 {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_TOO_MANY_CONCEPTS",
            "oracle forecast accepts at most 256 panel concepts",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut slots = Vec::with_capacity(concepts.len());
    for (index, concept) in concepts.iter().enumerate() {
        ensure_nonempty("panelConcepts", concept)?;
        if !seen.insert(concept.as_str()) {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_DUPLICATE_CONCEPT",
                format!("duplicate panel concept {concept}"),
            ));
        }
        let slot_id = SlotId::new((index + 1) as u16);
        slots.push(Slot {
            slot_id,
            slot_key: slot_id.with_key(format!("oracle-forecast-{concept}")),
            lens_id: LensId::from_bytes(content_address([
                b"calyxweb-oracle-forecast-lens".as_slice(),
                concept.as_bytes(),
                &(index as u64).to_be_bytes(),
            ])),
            shape: SlotShape::Dense(1),
            modality: Modality::Structured,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some(format!("oracle-forecast:{concept}")),
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: BTreeMap::new(),
            state: SlotState::Active,
            added_at_panel_version: ORACLE_FORECAST_PANEL_VERSION,
        });
    }
    Ok(Panel {
        version: ORACLE_FORECAST_PANEL_VERSION,
        slots,
        created_at: now,
        kernel_ref: None,
        guard_ref: None,
    })
}

pub(super) fn build_oracle_source_constellation(
    vault: &calyx_aster::vault::AsterVault<SystemClock>,
    request: &OracleForecastRequest,
    request_id: &str,
    domain: &DomainId,
    body_hash: &str,
    now: u64,
) -> Result<Constellation, OriginError> {
    let input_bytes = serde_json::to_vec(&json!({
        "kind": ORACLE_FORECAST_EVIDENCE_KIND,
        "requestId": request_id,
        "learnerId": request.learner_id,
        "domain": domain.to_string(),
        "actionId": request.action_id,
        "observationCount": request.observations.len(),
        "payloadSha256": body_hash
    }))
    .map_err(|error| OriginError::internal(error.to_string()))?;
    let cx_id = vault.cx_id_for_input(&input_bytes, ORACLE_FORECAST_PANEL_VERSION);
    Ok(Constellation {
        cx_id,
        vault_id: vault.vault_id(),
        panel_version: ORACLE_FORECAST_PANEL_VERSION,
        created_at: now,
        input_ref: InputRef {
            hash: sha256_array(&input_bytes),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Structured,
        slots: BTreeMap::new(),
        scalars: BTreeMap::from([(
            "oracle.observation_count".to_string(),
            request.observations.len() as f64,
        )]),
        metadata: BTreeMap::from([
            (
                "origin_kind".to_string(),
                ORACLE_FORECAST_EVIDENCE_KIND.to_string(),
            ),
            ("origin_version".to_string(), "1".to_string()),
            ("payload_sha256".to_string(), body_hash.to_string()),
            ("request_id".to_string(), request_id.to_string()),
            ("learner_id".to_string(), request.learner_id.clone()),
            ("domain".to_string(), domain.to_string()),
            ("action_id".to_string(), request.action_id.clone()),
        ]),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            redacted_input: true,
            ..CxFlags::default()
        },
    })
}

pub(super) fn build_oracle_graph_rows(
    vault: &calyx_aster::vault::AsterVault<SystemClock>,
    request: &OracleForecastRequest,
    request_id: &str,
    domain: &DomainId,
    body_hash: &str,
    now: u64,
) -> Result<(OracleGraphRows, usize, usize), OriginError> {
    if request.observations.is_empty() {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_EMPTY_ORACLE_GRAPH",
            "oracle forecast observations must contain recurrence evidence",
        ));
    }
    let mut by_action =
        BTreeMap::<String, Vec<&crate::learner_origin::model::OracleObservationRequest>>::new();
    for observation in &request.observations {
        ensure_nonempty("observations.actionId", &observation.action_id)?;
        validate_anchor_value("observations.outcome", &observation.outcome)?;
        if let Some(ground_truth) = &observation.ground_truth {
            validate_anchor_value("observations.groundTruth", ground_truth)?;
        }
        for consequence in &observation.consequences {
            ensure_nonempty(
                "observations.consequences.actionOrEvent",
                &consequence.action_or_event,
            )?;
            validate_anchor_value("observations.consequences.outcome", &consequence.outcome)?;
        }
        by_action
            .entry(observation.action_id.clone())
            .or_default()
            .push(observation);
    }
    if !by_action.contains_key(&request.action_id) {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_MISSING_FORECAST_ACTION",
            "observations must include at least one row for actionId",
        ));
    }

    let mut rows = Vec::new();
    let mut recurrence_count = 0_usize;
    for (action_id, observations) in by_action {
        let cx_id = oracle_graph_cx_id(request_id, domain, &action_id);
        let base = oracle_graph_constellation(OracleGraphConstellationInput {
            vault,
            cx_id,
            request,
            request_id,
            domain,
            action_id: &action_id,
            observation_count: observations.len(),
            body_hash,
            now,
        })?;
        rows.push((
            ColumnFamily::Base,
            base_key(cx_id),
            encode::encode_constellation_base(&base).map_err(storage_error)?,
        ));
        for (index, observation) in observations.iter().enumerate() {
            let occurrence = Occurrence {
                id: OccurrenceId(index as u64),
                t_k: EpochSecs((now / 1_000).saturating_add(index as u64) as i64),
                context: occurrence_context(oracle_observation_context(observation, domain)?)?,
            };
            rows.push((
                ColumnFamily::Recurrence,
                recurrence_key(cx_id, index as u64),
                encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                    .map_err(storage_error)?,
            ));
            recurrence_count += 1;
        }
    }
    let base_count = rows
        .iter()
        .filter(|(cf, _, _)| *cf == ColumnFamily::Base)
        .count();
    Ok((rows, base_count, recurrence_count))
}

struct OracleGraphConstellationInput<'a> {
    vault: &'a calyx_aster::vault::AsterVault<SystemClock>,
    cx_id: CxId,
    request: &'a OracleForecastRequest,
    request_id: &'a str,
    domain: &'a DomainId,
    action_id: &'a str,
    observation_count: usize,
    body_hash: &'a str,
    now: u64,
}

fn oracle_graph_constellation(
    input: OracleGraphConstellationInput<'_>,
) -> Result<Constellation, OriginError> {
    let input_bytes = serde_json::to_vec(&json!({
        "kind": ORACLE_FORECAST_GRAPH_KIND,
        "requestId": input.request_id,
        "domain": input.domain.to_string(),
        "actionId": input.action_id,
        "observationCount": input.observation_count,
        "payloadSha256": input.body_hash
    }))
    .map_err(|error| OriginError::internal(error.to_string()))?;
    Ok(Constellation {
        cx_id: input.cx_id,
        vault_id: input.vault.vault_id(),
        panel_version: ORACLE_FORECAST_PANEL_VERSION,
        created_at: input.now,
        input_ref: InputRef {
            hash: sha256_array(&input_bytes),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Structured,
        slots: BTreeMap::new(),
        scalars: BTreeMap::from([(
            "recurrence.frequency".to_string(),
            input.observation_count as f64,
        )]),
        metadata: BTreeMap::from([
            (
                "origin_kind".to_string(),
                ORACLE_FORECAST_GRAPH_KIND.to_string(),
            ),
            ("origin_version".to_string(), "1".to_string()),
            ("payload_sha256".to_string(), input.body_hash.to_string()),
            ("request_id".to_string(), input.request_id.to_string()),
            ("learner_id".to_string(), input.request.learner_id.clone()),
            (
                ORACLE_DOMAIN_METADATA_KEY.to_string(),
                input.domain.to_string(),
            ),
            (
                ORACLE_ACTION_METADATA_KEY.to_string(),
                input.action_id.to_string(),
            ),
        ]),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            redacted_input: true,
            ..CxFlags::default()
        },
    })
}

fn oracle_observation_context(
    observation: &crate::learner_origin::model::OracleObservationRequest,
    default_domain: &DomainId,
) -> Result<Vec<u8>, OriginError> {
    let consequences = observation
        .consequences
        .iter()
        .map(|consequence| {
            let mut edge = json!({
                "action_or_event": consequence.action_or_event,
                "domain": consequence
                    .domain
                    .as_deref()
                    .unwrap_or(default_domain.as_str()),
                "outcome": {"value": consequence.outcome},
                "grounded": consequence.grounded
            });
            if consequence.provisional {
                edge["provisional"] = json!(true);
            }
            edge
        })
        .collect::<Vec<_>>();
    let mut value = json!({
        "outcome_anchor": {"value": observation.outcome}
    });
    if let Some(ground_truth) = &observation.ground_truth {
        value["ground_truth_anchor"] = json!({"value": ground_truth});
    }
    if consequences.len() == 1 {
        value["consequence"] = consequences.into_iter().next().expect("one consequence");
    } else if !consequences.is_empty() {
        value["consequences"] = json!(consequences);
    }
    serde_json::to_vec(&value).map_err(|error| OriginError::internal(error.to_string()))
}

fn occurrence_context(bytes: Vec<u8>) -> Result<OccurrenceContext, OriginError> {
    OccurrenceContext::new(bytes).map_err(|error| {
        if error.code == "CALYX_RECURRENCE_CONTEXT_TOO_LARGE" {
            OriginError::bad_request("CALYX_ORIGIN_RECURRENCE_CONTEXT_TOO_LARGE", error.message)
        } else {
            storage_error(error)
        }
    })
}

fn oracle_graph_cx_id(request_id: &str, domain: &DomainId, action_id: &str) -> CxId {
    CxId::from_bytes(content_address([
        ORACLE_FORECAST_GRAPH_KIND.as_bytes(),
        request_id.as_bytes(),
        domain.as_str().as_bytes(),
        action_id.as_bytes(),
    ]))
}

pub(super) fn transfer_entropy_stream(
    field: &str,
    samples: &[crate::learner_origin::model::TransferEntropySampleRequest],
) -> Result<Vec<(u64, f32)>, OriginError> {
    if samples.is_empty() {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_EMPTY_TRANSFER_ENTROPY",
            format!("{field} must contain samples"),
        ));
    }
    let mut out = Vec::with_capacity(samples.len());
    let mut seen = BTreeSet::new();
    for sample in samples {
        if !sample.value.is_finite() {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_INVALID_NUMBER",
                format!("{field} contains a non-finite value"),
            ));
        }
        if !seen.insert(sample.t) {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_INVALID_TRANSFER_ENTROPY",
                format!("{field} contains duplicate timestamp {}", sample.t),
            ));
        }
        out.push((sample.t, sample.value));
    }
    out.sort_by_key(|(timestamp, _)| *timestamp);
    Ok(out)
}

pub(super) fn prerequisite_edges(source: &str, target: &str, results: &[TEResult]) -> Vec<Value> {
    results
        .iter()
        .filter(|result| !result.provisional && result.t_a_to_b > result.t_b_to_a)
        .map(|result| {
            json!({
                "from": source,
                "to": target,
                "lag": result.lag,
                "tAToB": result.t_a_to_b,
                "tBToA": result.t_b_to_a,
                "dominantDirection": result.dominant_direction,
                "supported": result.dominant_direction == Direction::AToB
                    || result.t_a_to_b > result.t_b_to_a
            })
        })
        .collect()
}

pub(super) fn tree_ledger_seq(tree: &ConsequenceTree) -> Option<u64> {
    tree.children
        .iter()
        .find_map(|child| {
            (child.root.provenance.seq != u64::MAX).then_some(child.root.provenance.seq)
        })
        .or_else(|| tree.children.iter().find_map(tree_ledger_seq))
}

pub(super) fn validate_anchor_value(field: &str, value: &AnchorValue) -> Result<(), OriginError> {
    value.validate_schema().map_err(|error| {
        OriginError::bad_request(
            "CALYX_ORIGIN_INVALID_ANCHOR",
            format!("{field}: {}", error.message),
        )
    })
}
