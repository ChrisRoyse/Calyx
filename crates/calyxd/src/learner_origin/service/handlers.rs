use std::collections::BTreeMap;

use calyx_core::{Anchor, AnchorKind, AnchorValue, Constellation};
use calyx_ledger::EntryKind;
use serde_json::{Value, json};

use crate::learner_origin::model::{
    DecisionRequest, KIND_DECISION, KIND_OUTCOME, KIND_SIGNAL_BATCH, OutcomeRequest,
    SignalBatchRequest,
};
use crate::learner_origin::privacy::reject_private_material;

use super::storage::OriginCommit;
use super::{
    LearnerOriginService, OriginError, OriginResponse, STATUS_CONFLICT, STATUS_CREATED,
    STATUS_FORBIDDEN, STATUS_NOT_FOUND, STATUS_OK, base_metadata, ensure_nonempty, hex,
    insert_optional, now_millis, parse_body, sha256_hex, stable_id,
};

impl LearnerOriginService {
    pub(super) fn handle_signal_batch(&self, body: &[u8]) -> Result<OriginResponse, OriginError> {
        let value = parse_body(body)?;
        reject_private_material(&value)
            .map_err(|detail| OriginError::bad_request("CALYX_ORIGIN_PRIVATE_FIELD", detail))?;
        let request: SignalBatchRequest = serde_json::from_value(value)
            .map_err(|error| OriginError::bad_request("CALYX_ORIGIN_JSON_INVALID", error))?;
        ensure_nonempty("batchId", &request.batch_id)?;
        ensure_nonempty("learnerId", &request.learner_id)?;
        if request.events.is_empty() {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_EMPTY_BATCH",
                "learner signal batch must contain at least one event",
            ));
        }
        let body_hash = sha256_hex(body);
        if let Some(existing) = self.find_by_idempotency(
            KIND_SIGNAL_BATCH,
            "batch_id",
            &request.batch_id,
            request.idempotency_key.as_deref(),
        )? {
            return self.duplicate_response(
                KIND_SIGNAL_BATCH,
                "batchId",
                &request.batch_id,
                &body_hash,
                existing,
            );
        }

        let mut metadata = base_metadata(KIND_SIGNAL_BATCH, &body_hash);
        metadata.insert("batch_id".to_string(), request.batch_id.clone());
        metadata.insert("learner_id".to_string(), request.learner_id.clone());
        insert_optional(
            &mut metadata,
            "idempotency_key",
            request.idempotency_key.as_deref(),
        );
        insert_optional(&mut metadata, "session_id", request.session_id.as_deref());
        insert_optional(
            &mut metadata,
            "privacy_class",
            request.privacy_class.as_deref(),
        );
        metadata.insert("event_count".to_string(), request.events.len().to_string());
        let scalars = BTreeMap::from([(
            "origin.event_count".to_string(),
            request.events.len() as f64,
        )]);
        let stored = self.commit_origin_row(OriginCommit {
            kind: KIND_SIGNAL_BATCH,
            primary_id: request.batch_id.clone(),
            ledger_kind: EntryKind::Ingest,
            metadata,
            scalars,
            slot_values: [1.0, request.events.len() as f32, 0.0, 0.0],
            anchors: Vec::new(),
        })?;
        self.metrics.record_write(KIND_SIGNAL_BATCH, "accepted");
        Ok(OriginResponse::json(
            STATUS_CREATED,
            json!({
                "accepted": true,
                "duplicate": false,
                "batchId": request.batch_id,
                "learnerId": request.learner_id,
                "cxId": stored.cx_id,
                "ledgerSeq": stored.ledger_seq,
                "ledgerHash": stored.ledger_hash
            }),
        ))
    }

    pub(super) fn handle_decision(&self, body: &[u8]) -> Result<OriginResponse, OriginError> {
        let value = parse_body(body)?;
        reject_private_material(&value)
            .map_err(|detail| OriginError::bad_request("CALYX_ORIGIN_PRIVATE_FIELD", detail))?;
        let request: DecisionRequest = serde_json::from_value(value)
            .map_err(|error| OriginError::bad_request("CALYX_ORIGIN_JSON_INVALID", error))?;
        ensure_nonempty("learnerId", &request.learner_id)?;
        ensure_nonempty("conceptId", &request.concept_id)?;
        let body_hash = sha256_hex(body);
        let decision_id = request.decision_id.clone().unwrap_or_else(|| {
            stable_id(
                "decision",
                [
                    request.learner_id.as_str(),
                    request.concept_id.as_str(),
                    body_hash.as_str(),
                ],
            )
        });
        if let Some(existing) = self.find_by_metadata(KIND_DECISION, "decision_id", &decision_id)? {
            return self.duplicate_response(
                KIND_DECISION,
                "decisionId",
                &decision_id,
                &body_hash,
                existing,
            );
        }

        let now = request.now_millis.unwrap_or_else(now_millis);
        let cooldown_until = request.cooldown_until.unwrap_or(0);
        let no_action = cooldown_until > now;
        let allowed_widgets = if no_action {
            Vec::new()
        } else if request.allowed_widget_kinds.is_empty() {
            vec!["concept_nudge".to_string()]
        } else {
            request.allowed_widget_kinds.clone()
        };
        let need = if no_action {
            "none".to_string()
        } else {
            request.need.unwrap_or_else(|| "review".to_string())
        };
        let trigger = if no_action {
            "cooldown".to_string()
        } else {
            request
                .trigger
                .unwrap_or_else(|| "learner_signal".to_string())
        };
        let confidence = if no_action {
            0.0
        } else {
            request.confidence.unwrap_or(0.5).clamp(0.0, 1.0)
        };

        let mut metadata = base_metadata(KIND_DECISION, &body_hash);
        metadata.insert("decision_id".to_string(), decision_id.clone());
        metadata.insert("learner_id".to_string(), request.learner_id.clone());
        metadata.insert("concept_id".to_string(), request.concept_id.clone());
        metadata.insert("need".to_string(), need.clone());
        metadata.insert("trigger".to_string(), trigger.clone());
        metadata.insert("cooldown_until".to_string(), cooldown_until.to_string());
        metadata.insert(
            "allowed_widget_count".to_string(),
            allowed_widgets.len().to_string(),
        );
        insert_optional(
            &mut metadata,
            "idempotency_key",
            request.idempotency_key.as_deref(),
        );
        insert_optional(&mut metadata, "session_id", request.session_id.as_deref());
        insert_optional(
            &mut metadata,
            "privacy_class",
            request.privacy_class.as_deref(),
        );
        let scalars = BTreeMap::from([
            ("origin.confidence".to_string(), confidence),
            (
                "origin.evidence_count".to_string(),
                request.evidence_ids.len() as f64,
            ),
        ]);
        let stored = self.commit_origin_row(OriginCommit {
            kind: KIND_DECISION,
            primary_id: decision_id.clone(),
            ledger_kind: EntryKind::Answer,
            metadata,
            scalars,
            slot_values: [
                2.0,
                confidence as f32,
                allowed_widgets.len() as f32,
                cooldown_until as f32,
            ],
            anchors: Vec::new(),
        })?;
        self.metrics.record_write(KIND_DECISION, "accepted");
        Ok(OriginResponse::json(
            STATUS_CREATED,
            json!({
                "decisionId": decision_id,
                "learnerId": request.learner_id,
                "conceptId": request.concept_id,
                "need": need,
                "trigger": trigger,
                "confidence": confidence,
                "evidenceIds": request.evidence_ids,
                "cooldownUntil": cooldown_until,
                "privacyClass": request.privacy_class.unwrap_or_else(|| "standard".to_string()),
                "allowedWidgetKinds": allowed_widgets,
                "cxId": stored.cx_id,
                "ledgerSeq": stored.ledger_seq,
                "ledgerHash": stored.ledger_hash
            }),
        ))
    }

    pub(super) fn handle_outcome(
        &self,
        decision_id: &str,
        body: &[u8],
    ) -> Result<OriginResponse, OriginError> {
        ensure_nonempty("decisionId", decision_id)?;
        let value = parse_body(body)?;
        reject_private_material(&value)
            .map_err(|detail| OriginError::bad_request("CALYX_ORIGIN_PRIVATE_FIELD", detail))?;
        let request: OutcomeRequest = serde_json::from_value(value)
            .map_err(|error| OriginError::bad_request("CALYX_ORIGIN_JSON_INVALID", error))?;
        ensure_nonempty("learnerId", &request.learner_id)?;
        if let Some(body_decision_id) = request.decision_id.as_deref()
            && body_decision_id != decision_id
        {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_DECISION_MISMATCH",
                "body decisionId does not match request path",
            ));
        }
        let decision = self
            .find_by_metadata(KIND_DECISION, "decision_id", decision_id)?
            .ok_or_else(|| {
                OriginError::new(
                    STATUS_NOT_FOUND,
                    "CALYX_ORIGIN_DECISION_UNKNOWN",
                    "decisionId is not present in the learner vault",
                )
            })?;
        if decision.metadata_value("learner_id") != Some(request.learner_id.as_str()) {
            return Err(OriginError::new(
                STATUS_FORBIDDEN,
                "CALYX_ORIGIN_WRONG_LEARNER",
                "outcome learnerId does not match the stored decision",
            ));
        }

        let body_hash = sha256_hex(body);
        let outcome_value = request
            .outcome
            .or(request.status)
            .unwrap_or_else(|| "shown".to_string());
        ensure_nonempty("outcome", &outcome_value)?;
        let outcome_id = request.outcome_id.unwrap_or_else(|| {
            stable_id("outcome", [decision_id, &request.learner_id, &body_hash])
        });
        if let Some(existing) = self.find_by_metadata(KIND_OUTCOME, "outcome_id", &outcome_id)? {
            return self.duplicate_response(
                KIND_OUTCOME,
                "outcomeId",
                &outcome_id,
                &body_hash,
                existing,
            );
        }

        let mut metadata = base_metadata(KIND_OUTCOME, &body_hash);
        metadata.insert("decision_id".to_string(), decision_id.to_string());
        metadata.insert("outcome_id".to_string(), outcome_id.clone());
        metadata.insert("learner_id".to_string(), request.learner_id.clone());
        metadata.insert("outcome".to_string(), outcome_value.clone());
        if let Some(concept_id) = decision.metadata_value("concept_id") {
            metadata.insert("concept_id".to_string(), concept_id.to_string());
        }
        insert_optional(
            &mut metadata,
            "privacy_class",
            request.privacy_class.as_deref(),
        );
        let evidence_count = match &request.evidence {
            Value::Array(items) => items.len(),
            Value::Null => 0,
            _ => 1,
        };
        let scalars =
            BTreeMap::from([("origin.evidence_count".to_string(), evidence_count as f64)]);
        let stored = self.commit_origin_row(OriginCommit {
            kind: KIND_OUTCOME,
            primary_id: outcome_id.clone(),
            ledger_kind: EntryKind::Anneal,
            metadata,
            scalars,
            slot_values: [3.0, evidence_count as f32, 0.0, 0.0],
            anchors: vec![Anchor {
                kind: AnchorKind::Reward,
                value: AnchorValue::Enum(outcome_value.clone()),
                source: "calyx-website-worker".to_string(),
                observed_at: now_millis(),
                confidence: 1.0,
            }],
        })?;
        self.metrics.record_write(KIND_OUTCOME, "accepted");
        Ok(OriginResponse::json(
            STATUS_CREATED,
            json!({
                "accepted": true,
                "duplicate": false,
                "decisionId": decision_id,
                "outcomeId": outcome_id,
                "learnerId": request.learner_id,
                "outcome": outcome_value,
                "cxId": stored.cx_id,
                "ledgerSeq": stored.ledger_seq,
                "ledgerHash": stored.ledger_hash
            }),
        ))
    }

    fn duplicate_response(
        &self,
        kind: &'static str,
        id_field: &str,
        id_value: &str,
        body_hash: &str,
        existing: Constellation,
    ) -> Result<OriginResponse, OriginError> {
        if existing.metadata_value("payload_sha256") != Some(body_hash) {
            return Err(OriginError::new(
                STATUS_CONFLICT,
                "CALYX_ORIGIN_IDEMPOTENCY_CONFLICT",
                "existing idempotency key or object id has different payload bytes",
            ));
        }
        self.metrics.record_write(kind, "duplicate");
        let mut body = json!({
            "accepted": true,
            "duplicate": true,
            "cxId": existing.cx_id.to_string(),
            "ledgerSeq": existing.provenance.seq,
            "ledgerHash": hex(&existing.provenance.hash)
        });
        body.as_object_mut()
            .expect("duplicate response is object")
            .insert(id_field.to_string(), json!(id_value));
        Ok(OriginResponse::json(STATUS_OK, body))
    }
}
