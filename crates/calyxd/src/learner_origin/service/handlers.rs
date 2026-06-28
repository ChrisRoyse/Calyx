use std::collections::{BTreeMap, BTreeSet};

use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, DEFAULT_TE_BOOTSTRAP_RESAMPLES,
    DEFAULT_TE_BOOTSTRAP_SEED, DEFAULT_TE_K, DEFAULT_TE_LAGS, DEFAULT_TE_WINDOW, Direction,
    EstimatorKind, MiEstimate, TEResult, TransferEntropyConfig, TrustTag,
    transfer_entropy_sweep_with_config,
};
use calyx_aster::cf::{ColumnFamily, base_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::encode;
use calyx_core::{
    AbsentReason, Anchor, AnchorKind, AnchorValue, Asymmetry, Clock, Constellation, CxFlags, CxId,
    InputRef, LedgerRef, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotShape, SlotState,
    SlotVector, SystemClock, content_address,
};
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};
use calyx_lodestar::{LodestarError, RecallReport};
use calyx_oracle::{
    Action, AnnealConfig, CalibrationMeasurement, CalibrationSource, CompletionRegion, Consequence,
    ConsequenceTree, DomainId, GoodhartDefenseMeasurement, GoodhartDefenseSource, HeldOutSplit,
    KernelRecallSource, MistakeClosureMeasurement, MistakeClosureSource,
    ORACLE_ACTION_METADATA_KEY, ORACLE_DOMAIN_METADATA_KEY, OracleConsistencySource, OracleError,
    OracleSelfConsistency, Prediction, ShortCircuit, SlotSet, SuperIntelligenceRequest,
    VaultSufficiencyAssay, build_tree, complete, oracle_predict, reverse_query, select,
    super_intelligence_with_ledger,
};
use serde_json::{Value, json};

use crate::learner_origin::model::{
    DecisionRequest, KIND_DECISION, KIND_MASTERY_ESTIMATE, KIND_ORACLE_FORECAST, KIND_OUTCOME,
    KIND_SIGNAL_BATCH, MasteryEstimateRequest, MasteryTrustGateRequest, OracleForecastRequest,
    OutcomeRequest, SignalBatchRequest, TransferEntropyRequest,
};
use crate::learner_origin::privacy::reject_private_material;

use super::storage::OriginCommit;
use super::{
    LearnerOriginService, ORIGIN_ACTOR, OriginError, OriginResponse, STATUS_CONFLICT,
    STATUS_CREATED, STATUS_FORBIDDEN, STATUS_NOT_FOUND, STATUS_OK, STATUS_UNPROCESSABLE,
    base_metadata, ensure_nonempty, hex, insert_optional, now_millis, parse_body, sha256_array,
    sha256_hex, stable_id, storage_error,
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

    pub(super) fn handle_mastery_estimate(
        &self,
        body: &[u8],
    ) -> Result<OriginResponse, OriginError> {
        let value = parse_body(body)?;
        reject_private_material(&value)
            .map_err(|detail| OriginError::bad_request("CALYX_ORIGIN_PRIVATE_FIELD", detail))?;
        let request: MasteryEstimateRequest = serde_json::from_value(value)
            .map_err(|error| OriginError::bad_request("CALYX_ORIGIN_JSON_INVALID", error))?;
        ensure_nonempty("learnerId", &request.learner_id)?;
        let body_hash = sha256_hex(body);
        let request_id = request.request_id.clone().unwrap_or_else(|| {
            stable_id(
                "mastery",
                [
                    request.learner_id.as_str(),
                    request.domain.as_deref().unwrap_or("calyxweb-learner"),
                    body_hash.as_str(),
                ],
            )
        });
        if let Some(existing) = self.find_by_idempotency(
            KIND_MASTERY_ESTIMATE,
            "request_id",
            &request_id,
            request.idempotency_key.as_deref(),
        )? {
            return self.duplicate_response(
                KIND_MASTERY_ESTIMATE,
                "requestId",
                &request_id,
                &body_hash,
                existing,
            );
        }

        let now = request.now_millis.unwrap_or_else(now_millis);
        let plan = MasteryPlan::from_request(&request, &request_id, &body_hash, now, &self.vault)?;
        let source_row = self.commit_constellation_row(
            plan.cx.clone(),
            "mastery_evidence",
            &request_id,
            EntryKind::Ingest,
            &body_hash,
        )?;
        let assay_rows = plan.persist_assay_rows(&self.vault, now)?;
        let clock = SystemClock;
        let completion = complete(
            &self.vault,
            &plan.cx,
            &plan.panel,
            plan.domain.clone(),
            plan.clamp.clone(),
            plan.free.clone(),
            &plan.region,
            plan.oracle.clone(),
            &MasteryAnneal,
            &clock,
        )
        .map_err(oracle_origin_error)?;

        let trust = plan.trust_sources();
        let assay = VaultSufficiencyAssay::new(&self.vault);
        let trust_request = SuperIntelligenceRequest {
            oracle: &trust.oracle,
            assay: &assay,
            kernel: &trust.kernel,
            calibration: &trust.calibration,
            goodhart: &trust.goodhart,
            mistakes: &trust.mistakes,
            panel: &plan.panel,
            domain: plan.domain.clone(),
            held_out: &plan.held_out,
            clock: &clock,
            short_circuit: ShortCircuit::MeasureAll,
        };
        let (trust_report, trust_ledger) =
            super_intelligence_with_ledger(&self.vault, trust_request)
                .map_err(oracle_origin_error)?;
        self.vault.flush().map_err(storage_error)?;

        let provisional_count = completion.provisional_slots().len();
        let inferred_count = completion.inferred_slots().len();
        let certification_eligible = trust_report.overall && provisional_count == 0;
        let mut metadata = base_metadata(KIND_MASTERY_ESTIMATE, &body_hash);
        metadata.insert("request_id".to_string(), request_id.clone());
        metadata.insert("learner_id".to_string(), request.learner_id.clone());
        metadata.insert("domain".to_string(), plan.domain.to_string());
        metadata.insert("source_cx_id".to_string(), source_row.cx_id.clone());
        metadata.insert(
            "completion_ledger_seq".to_string(),
            completion.provenance.seq.to_string(),
        );
        metadata.insert("trust_ledger_seq".to_string(), trust_ledger.seq.to_string());
        metadata.insert("concept_count".to_string(), plan.concepts.len().to_string());
        metadata.insert("inferred_count".to_string(), inferred_count.to_string());
        metadata.insert(
            "provisional_count".to_string(),
            provisional_count.to_string(),
        );
        metadata.insert(
            "certification_eligible".to_string(),
            certification_eligible.to_string(),
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
            (
                "mastery.confidence".to_string(),
                completion.confidence as f64,
            ),
            (
                "mastery.trust_overall".to_string(),
                if trust_report.overall { 1.0 } else { 0.0 },
            ),
            (
                "mastery.certification_eligible".to_string(),
                if certification_eligible { 1.0 } else { 0.0 },
            ),
            ("mastery.inferred_count".to_string(), inferred_count as f64),
        ]);
        let stored = self.commit_origin_row(OriginCommit {
            kind: KIND_MASTERY_ESTIMATE,
            primary_id: request_id.clone(),
            ledger_kind: EntryKind::Assay,
            metadata,
            scalars,
            slot_values: [
                4.0,
                completion.confidence,
                if trust_report.overall { 1.0 } else { 0.0 },
                if certification_eligible { 1.0 } else { 0.0 },
            ],
            anchors: Vec::new(),
        })?;
        self.metrics.record_write(KIND_MASTERY_ESTIMATE, "accepted");
        Ok(OriginResponse::json(
            STATUS_CREATED,
            json!({
                "accepted": true,
                "duplicate": false,
                "requestId": request_id,
                "learnerId": request.learner_id,
                "domain": plan.domain.to_string(),
                "source": {
                    "cxId": source_row.cx_id,
                    "ledgerSeq": source_row.ledger_seq,
                    "ledgerHash": source_row.ledger_hash,
                    "assayRows": assay_rows
                },
                "completion": {
                    "confidence": completion.confidence,
                    "converged": completion.converged,
                    "energy": completion.energy,
                    "ledgerSeq": completion.provenance.seq,
                    "ledgerHash": hex(&completion.provenance.hash),
                    "slots": plan.slot_readbacks(&completion)
                },
                "trust": {
                    "overall": trust_report.overall,
                    "failingTier": trust_report.failing_tier,
                    "cheapestFix": trust_report.cheapest_fix,
                    "tiers": trust_report.tiers,
                    "ledgerSeq": trust_ledger.seq,
                    "ledgerHash": hex(&trust_ledger.hash)
                },
                "certificationEligible": certification_eligible,
                "cxId": stored.cx_id,
                "ledgerSeq": stored.ledger_seq,
                "ledgerHash": stored.ledger_hash
            }),
        ))
    }

    pub(super) fn handle_oracle_forecast(
        &self,
        body: &[u8],
    ) -> Result<OriginResponse, OriginError> {
        let value = parse_body(body)?;
        reject_private_material(&value)
            .map_err(|detail| OriginError::bad_request("CALYX_ORIGIN_PRIVATE_FIELD", detail))?;
        let request: OracleForecastRequest = serde_json::from_value(value)
            .map_err(|error| OriginError::bad_request("CALYX_ORIGIN_JSON_INVALID", error))?;
        ensure_nonempty("learnerId", &request.learner_id)?;
        ensure_nonempty("actionId", &request.action_id)?;
        let body_hash = sha256_hex(body);
        let request_id = request.request_id.clone().unwrap_or_else(|| {
            stable_id(
                "oracle-forecast",
                [
                    request.learner_id.as_str(),
                    request
                        .domain
                        .as_deref()
                        .unwrap_or("calyxweb-learner-oracle"),
                    request.action_id.as_str(),
                    body_hash.as_str(),
                ],
            )
        });
        if let Some(existing) = self.find_by_idempotency(
            KIND_ORACLE_FORECAST,
            "request_id",
            &request_id,
            request.idempotency_key.as_deref(),
        )? {
            return self.duplicate_response(
                KIND_ORACLE_FORECAST,
                "requestId",
                &request_id,
                &body_hash,
                existing,
            );
        }

        let now = request.now_millis.unwrap_or_else(now_millis);
        let plan =
            OracleForecastPlan::from_request(&request, &request_id, &body_hash, now, &self.vault)?;
        let source_row = self.commit_constellation_row(
            plan.source_cx.clone(),
            ORACLE_FORECAST_EVIDENCE_KIND,
            &request_id,
            EntryKind::Ingest,
            &body_hash,
        )?;
        let clock = SystemClock;
        let transfer_entropy = plan.transfer_entropy_readback(&clock)?;
        if transfer_entropy
            .results
            .iter()
            .all(|result| result.provisional)
        {
            return Err(OriginError::new(
                STATUS_UNPROCESSABLE,
                "CALYX_ORIGIN_TRANSFER_ENTROPY_REJECTED",
                "transfer entropy did not reach the minimum non-provisional quorum",
            ));
        }
        let assay_rows = plan.persist_assay_rows(&self.vault, now)?;
        let graph_commit = self.commit_oracle_graph_rows(
            plan.graph_rows.clone(),
            &request_id,
            &body_hash,
            plan.graph_base_count,
            plan.recurrence_count,
        )?;

        let prediction = oracle_predict(&self.vault, &plan.action, plan.domain.clone(), &clock)
            .map_err(oracle_origin_error)?;
        let tree = build_tree(&self.vault, plan.root_consequence(&prediction), &clock)
            .map_err(oracle_origin_error)?;
        let expansion_ledger_seq =
            tree_ledger_seq(&tree).unwrap_or_else(|| self.vault.latest_seq());
        let selected = plan
            .desired_outcome
            .as_ref()
            .and_then(|desired| select(&tree, desired).map(|node| node.root.clone()));
        let causes = reverse_query(
            &self.vault,
            &plan.reverse_answer,
            plan.domain.clone(),
            &clock,
        )
        .map_err(oracle_origin_error)?;
        let reverse_ledger_seq = causes
            .first()
            .map(|cause| cause.provenance.seq)
            .unwrap_or_else(|| self.vault.latest_seq());
        self.vault.flush().map_err(storage_error)?;

        let mut metadata = base_metadata(KIND_ORACLE_FORECAST, &body_hash);
        metadata.insert("request_id".to_string(), request_id.clone());
        metadata.insert("learner_id".to_string(), request.learner_id.clone());
        metadata.insert("domain".to_string(), plan.domain.to_string());
        metadata.insert("action_id".to_string(), request.action_id.clone());
        metadata.insert("source_cx_id".to_string(), source_row.cx_id.clone());
        metadata.insert(
            "graph_ledger_seq".to_string(),
            graph_commit.ledger_seq.to_string(),
        );
        metadata.insert(
            "prediction_ledger_seq".to_string(),
            prediction.provenance.seq.to_string(),
        );
        metadata.insert(
            "expansion_ledger_seq".to_string(),
            expansion_ledger_seq.to_string(),
        );
        metadata.insert(
            "reverse_ledger_seq".to_string(),
            reverse_ledger_seq.to_string(),
        );
        metadata.insert(
            "recurrence_count".to_string(),
            plan.recurrence_count.to_string(),
        );
        metadata.insert(
            "transfer_entropy_result_count".to_string(),
            transfer_entropy.results.len().to_string(),
        );
        metadata.insert(
            "prereq_edge_count".to_string(),
            transfer_entropy.prereq_edges.len().to_string(),
        );
        if let Some(max_lag) = transfer_entropy.max_lag {
            metadata.insert("transfer_entropy_max_lag".to_string(), max_lag.to_string());
        }
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
            (
                "oracle.prediction_confidence".to_string(),
                prediction.confidence as f64,
            ),
            (
                "oracle.tree_children".to_string(),
                tree.children.len() as f64,
            ),
            ("oracle.reverse_causes".to_string(), causes.len() as f64),
            (
                "oracle.prereq_edges".to_string(),
                transfer_entropy.prereq_edges.len() as f64,
            ),
        ]);
        let stored = self.commit_origin_row(OriginCommit {
            kind: KIND_ORACLE_FORECAST,
            primary_id: request_id.clone(),
            ledger_kind: EntryKind::Assay,
            metadata,
            scalars,
            slot_values: [
                5.0,
                prediction.confidence,
                tree.children.len() as f32,
                transfer_entropy.prereq_edges.len() as f32,
            ],
            anchors: Vec::new(),
        })?;
        self.metrics.record_write(KIND_ORACLE_FORECAST, "accepted");
        Ok(OriginResponse::json(
            STATUS_CREATED,
            json!({
                "accepted": true,
                "duplicate": false,
                "requestId": request_id,
                "learnerId": request.learner_id,
                "domain": plan.domain.to_string(),
                "actionId": request.action_id,
                "source": {
                    "cxId": source_row.cx_id,
                    "ledgerSeq": source_row.ledger_seq,
                    "ledgerHash": source_row.ledger_hash,
                    "assayRows": assay_rows,
                    "graphBaseRows": graph_commit.base_rows,
                    "recurrenceRows": graph_commit.recurrence_rows,
                    "graphLedgerSeq": graph_commit.ledger_seq
                },
                "prediction": prediction,
                "consequenceTree": tree,
                "selectedConsequence": selected,
                "reverse": {
                    "answer": plan.reverse_answer,
                    "causes": causes,
                    "ledgerSeq": reverse_ledger_seq
                },
                "transferEntropy": {
                    "sourceConceptId": transfer_entropy.source_concept_id,
                    "targetConceptId": transfer_entropy.target_concept_id,
                    "results": transfer_entropy.results,
                    "maxLag": transfer_entropy.max_lag,
                    "prereqEdges": transfer_entropy.prereq_edges
                },
                "cxId": stored.cx_id,
                "ledgerSeq": stored.ledger_seq,
                "ledgerHash": stored.ledger_hash
            }),
        ))
    }

    fn commit_oracle_graph_rows(
        &self,
        rows: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
        request_id: &str,
        body_hash: &str,
        base_rows: usize,
        recurrence_rows: usize,
    ) -> Result<OracleGraphCommit, OriginError> {
        if rows.is_empty() {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_EMPTY_ORACLE_GRAPH",
                "oracle forecast requires at least one recurrence row",
            ));
        }
        let mut payload = PayloadBuilder::default();
        payload
            .insert_str("request_id", request_id)
            .insert_str("kind", ORACLE_FORECAST_GRAPH_KIND)
            .insert_str("input_hash", body_hash)
            .insert_u64("base_rows", base_rows as u64)
            .insert_u64("recurrence_rows", recurrence_rows as u64)
            .insert_u64("ts", SystemClock.now());
        let ledger_payload = RedactionPolicy::default().apply_to_payload(&payload);
        let ledger_seq = self
            .vault
            .write_cf_batch_with_ledger_entry(
                rows,
                EntryKind::Ingest,
                SubjectId::Query(
                    content_address([
                        ORACLE_FORECAST_GRAPH_KIND.as_bytes(),
                        request_id.as_bytes(),
                        body_hash.as_bytes(),
                    ])
                    .to_vec(),
                ),
                ledger_payload,
                ActorId::Service(ORIGIN_ACTOR.to_string()),
            )
            .map_err(storage_error)?;
        self.vault.flush().map_err(storage_error)?;
        Ok(OracleGraphCommit {
            ledger_seq,
            base_rows,
            recurrence_rows,
        })
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

const ORACLE_FORECAST_PANEL_VERSION: u32 = 1240;
const ORACLE_FORECAST_EVIDENCE_KIND: &str = "oracle_forecast_evidence";
const ORACLE_FORECAST_GRAPH_KIND: &str = "oracle_forecast_recurrence";

struct OracleGraphCommit {
    ledger_seq: u64,
    base_rows: usize,
    recurrence_rows: usize,
}

struct OracleForecastPlan {
    domain: DomainId,
    action: Action,
    source_cx: Constellation,
    graph_rows: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
    graph_base_count: usize,
    recurrence_count: usize,
    reverse_answer: AnchorValue,
    desired_outcome: Option<AnchorValue>,
    transfer_entropy: TransferEntropyJob,
}

impl OracleForecastPlan {
    fn from_request(
        request: &OracleForecastRequest,
        request_id: &str,
        body_hash: &str,
        now: u64,
        vault: &calyx_aster::vault::AsterVault<SystemClock>,
    ) -> Result<Self, OriginError> {
        let base_domain = request
            .domain
            .as_deref()
            .unwrap_or("calyxweb-learner-oracle");
        ensure_nonempty("domain", base_domain)?;
        let domain = DomainId::from(base_domain.to_string());
        let panel_bits = require_nonnegative_bits("panelBits", request.panel_bits)?;
        let anchor_entropy_bits =
            require_nonnegative_bits("anchorEntropyBits", request.anchor_entropy_bits)?;
        let panel = build_oracle_panel(request, now)?;
        let action = Action {
            action_id: request.action_id.clone(),
            panel: panel.clone(),
            guard: None,
        };
        validate_anchor_value("reverseAnswer", &request.reverse_answer)?;
        if let Some(desired) = &request.desired_outcome {
            validate_anchor_value("desiredOutcome", desired)?;
        }
        let source_cx =
            build_oracle_source_constellation(vault, request, request_id, &domain, body_hash, now)?;
        let (graph_rows, graph_base_count, recurrence_count) =
            build_oracle_graph_rows(vault, request, request_id, &domain, body_hash, now)?;
        let transfer_entropy =
            TransferEntropyJob::from_request(&request.transfer_entropy, request_id)?;
        Ok(Self {
            domain,
            action,
            source_cx,
            graph_rows,
            graph_base_count,
            recurrence_count,
            reverse_answer: request.reverse_answer.clone(),
            desired_outcome: request.desired_outcome.clone(),
            transfer_entropy: transfer_entropy.with_sufficiency(panel_bits, anchor_entropy_bits),
        })
    }

    fn persist_assay_rows(
        &self,
        vault: &calyx_aster::vault::AsterVault<SystemClock>,
        now: u64,
    ) -> Result<usize, OriginError> {
        let mut store = AssayStore::default();
        let key = AssayCacheKey::scoped(
            self.action.panel.version,
            self.domain.as_str(),
            vault.vault_id(),
            AnchorKind::Reward,
        );
        store.put(
            key.clone(),
            AssaySubject::Panel,
            MiEstimate::point(
                self.transfer_entropy.panel_bits,
                self.transfer_entropy.sample_count,
                EstimatorKind::PanelSufficiency,
                TrustTag::Trusted,
            ),
            "learner-origin oracle forecast panel sufficiency",
            now,
        );
        store.put(
            key.clone(),
            AssaySubject::OutcomeEntropy,
            MiEstimate::point(
                self.transfer_entropy.anchor_entropy_bits,
                self.transfer_entropy.sample_count,
                EstimatorKind::OutcomeEntropy,
                TrustTag::Trusted,
            ),
            "learner-origin oracle forecast outcome entropy",
            now,
        );
        let per_slot_bits = if self.action.panel.slots.is_empty() {
            0.0
        } else {
            self.transfer_entropy.panel_bits / self.action.panel.slots.len() as f32
        };
        for slot in &self.action.panel.slots {
            store.put(
                key.clone(),
                AssaySubject::Lens { slot: slot.slot_id },
                MiEstimate::point(
                    per_slot_bits,
                    self.transfer_entropy.sample_count,
                    EstimatorKind::Ksg,
                    TrustTag::Trusted,
                ),
                format!("learner-origin oracle forecast slot {}", slot.slot_id.get()),
                now,
            );
        }
        store.persist_to_vault(vault).map_err(storage_error)
    }

    fn transfer_entropy_readback(
        &self,
        clock: &dyn Clock,
    ) -> Result<TransferEntropyReadback, OriginError> {
        self.transfer_entropy.readback(clock)
    }

    fn root_consequence(&self, prediction: &Prediction) -> Consequence {
        Consequence {
            action_or_event: self.action.action_id.clone(),
            domain: self.domain.clone(),
            outcome: prediction.outcome.clone(),
            confidence: prediction.confidence,
            hop: 0,
            provenance: prediction.provenance.clone(),
        }
    }
}

#[derive(Clone)]
struct TransferEntropyJob {
    source_concept_id: String,
    target_concept_id: String,
    source_series: Vec<(u64, f32)>,
    target_series: Vec<(u64, f32)>,
    lags: Vec<usize>,
    config: TransferEntropyConfig,
    panel_bits: f32,
    anchor_entropy_bits: f32,
    sample_count: usize,
}

impl TransferEntropyJob {
    fn from_request(
        request: &TransferEntropyRequest,
        request_id: &str,
    ) -> Result<Self, OriginError> {
        ensure_nonempty(
            "transferEntropy.sourceConceptId",
            &request.source_concept_id,
        )?;
        ensure_nonempty(
            "transferEntropy.targetConceptId",
            &request.target_concept_id,
        )?;
        let source_series =
            transfer_entropy_stream("transferEntropy.sourceSeries", &request.source_series)?;
        let target_series =
            transfer_entropy_stream("transferEntropy.targetSeries", &request.target_series)?;
        let lags = if request.lags.is_empty() {
            DEFAULT_TE_LAGS.to_vec()
        } else {
            request.lags.clone()
        };
        if lags.iter().any(|lag| *lag == 0) {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_INVALID_TRANSFER_ENTROPY",
                "transferEntropy.lags must be positive",
            ));
        }
        let config = TransferEntropyConfig {
            window_size: request.window_size.unwrap_or(DEFAULT_TE_WINDOW),
            k: request.k.unwrap_or(DEFAULT_TE_K),
            bootstrap_resamples: request
                .bootstrap_resamples
                .unwrap_or(DEFAULT_TE_BOOTSTRAP_RESAMPLES),
            bootstrap_seed: request.bootstrap_seed.unwrap_or_else(|| {
                let digest = content_address([
                    b"oracle-forecast-transfer-entropy".as_slice(),
                    request_id.as_bytes(),
                ]);
                u64::from_be_bytes(digest[..8].try_into().expect("digest slice is u64"))
                    ^ DEFAULT_TE_BOOTSTRAP_SEED
            }),
        };
        if config.window_size == 0 || config.k == 0 || config.bootstrap_resamples == 0 {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_INVALID_TRANSFER_ENTROPY",
                "transferEntropy windowSize, k, and bootstrapResamples must be positive",
            ));
        }
        let sample_count = source_series.len().min(target_series.len()).max(1);
        Ok(Self {
            source_concept_id: request.source_concept_id.clone(),
            target_concept_id: request.target_concept_id.clone(),
            source_series,
            target_series,
            lags,
            config,
            panel_bits: 0.0,
            anchor_entropy_bits: 0.0,
            sample_count,
        })
    }

    fn with_sufficiency(mut self, panel_bits: f32, anchor_entropy_bits: f32) -> Self {
        self.panel_bits = panel_bits;
        self.anchor_entropy_bits = anchor_entropy_bits;
        self
    }

    fn readback(&self, clock: &dyn Clock) -> Result<TransferEntropyReadback, OriginError> {
        let results = transfer_entropy_sweep_with_config(
            &self.source_series,
            &self.target_series,
            &self.lags,
            clock,
            &self.config,
        );
        let max_lag = calyx_assay::max_transfer_entropy_lag(&results);
        let prereq_edges =
            prerequisite_edges(&self.source_concept_id, &self.target_concept_id, &results);
        Ok(TransferEntropyReadback {
            source_concept_id: self.source_concept_id.clone(),
            target_concept_id: self.target_concept_id.clone(),
            results,
            max_lag,
            prereq_edges,
        })
    }
}

struct TransferEntropyReadback {
    source_concept_id: String,
    target_concept_id: String,
    results: Vec<TEResult>,
    max_lag: Option<usize>,
    prereq_edges: Vec<Value>,
}

fn build_oracle_panel(request: &OracleForecastRequest, now: u64) -> Result<Panel, OriginError> {
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

fn build_oracle_source_constellation(
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

fn build_oracle_graph_rows(
    vault: &calyx_aster::vault::AsterVault<SystemClock>,
    request: &OracleForecastRequest,
    request_id: &str,
    domain: &DomainId,
    body_hash: &str,
    now: u64,
) -> Result<(Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>, usize, usize), OriginError> {
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
        let base = oracle_graph_constellation(
            vault,
            cx_id,
            request,
            request_id,
            domain,
            &action_id,
            observations.len(),
            body_hash,
            now,
        )?;
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

fn oracle_graph_constellation(
    vault: &calyx_aster::vault::AsterVault<SystemClock>,
    cx_id: CxId,
    request: &OracleForecastRequest,
    request_id: &str,
    domain: &DomainId,
    action_id: &str,
    observation_count: usize,
    body_hash: &str,
    now: u64,
) -> Result<Constellation, OriginError> {
    let input_bytes = serde_json::to_vec(&json!({
        "kind": ORACLE_FORECAST_GRAPH_KIND,
        "requestId": request_id,
        "domain": domain.to_string(),
        "actionId": action_id,
        "observationCount": observation_count,
        "payloadSha256": body_hash
    }))
    .map_err(|error| OriginError::internal(error.to_string()))?;
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
        scalars: BTreeMap::from([("recurrence.frequency".to_string(), observation_count as f64)]),
        metadata: BTreeMap::from([
            (
                "origin_kind".to_string(),
                ORACLE_FORECAST_GRAPH_KIND.to_string(),
            ),
            ("origin_version".to_string(), "1".to_string()),
            ("payload_sha256".to_string(), body_hash.to_string()),
            ("request_id".to_string(), request_id.to_string()),
            ("learner_id".to_string(), request.learner_id.clone()),
            (ORACLE_DOMAIN_METADATA_KEY.to_string(), domain.to_string()),
            (
                ORACLE_ACTION_METADATA_KEY.to_string(),
                action_id.to_string(),
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
                "outcome": {"value": consequence.outcome}
            });
            if !consequence.grounded {
                edge["grounded"] = json!(false);
            }
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

fn transfer_entropy_stream(
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

fn prerequisite_edges(source: &str, target: &str, results: &[TEResult]) -> Vec<Value> {
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

fn tree_ledger_seq(tree: &ConsequenceTree) -> Option<u64> {
    tree.children
        .iter()
        .find_map(|child| {
            (child.root.provenance.seq != u64::MAX).then_some(child.root.provenance.seq)
        })
        .or_else(|| tree.children.iter().find_map(tree_ledger_seq))
}

fn validate_anchor_value(field: &str, value: &AnchorValue) -> Result<(), OriginError> {
    value.validate_schema().map_err(|error| {
        OriginError::bad_request(
            "CALYX_ORIGIN_INVALID_ANCHOR",
            format!("{field}: {}", error.message),
        )
    })
}

struct MasteryPlan {
    domain: DomainId,
    panel: Panel,
    cx: Constellation,
    clamp: SlotSet,
    free: SlotSet,
    region: MasteryRegion,
    oracle: OracleSelfConsistency,
    trust_gate: MasteryTrustGate,
    held_out: HeldOutSplit,
    concepts: Vec<MasteryConcept>,
}

impl MasteryPlan {
    fn from_request(
        request: &MasteryEstimateRequest,
        request_id: &str,
        body_hash: &str,
        now: u64,
        vault: &calyx_aster::vault::AsterVault<SystemClock>,
    ) -> Result<Self, OriginError> {
        let base_domain = request
            .domain
            .as_deref()
            .unwrap_or("calyxweb-learner-mastery");
        ensure_nonempty("domain", base_domain)?;
        let domain = DomainId::from(format!("{base_domain}:{request_id}"));
        let panel_bits = require_nonnegative_bits("panelBits", request.panel_bits)?;
        let anchor_entropy_bits =
            require_nonnegative_bits("anchorEntropyBits", request.anchor_entropy_bits)?;
        let oracle = request.oracle_self_consistency.to_oracle()?;
        let trust_gate = MasteryTrustGate::from_request(&request.trust_gate)?;
        let concepts = build_mastery_concepts(&request.concepts)?;
        let panel = build_mastery_panel(&concepts, now);
        let input_bytes = serde_json::to_vec(&json!({
            "kind": "mastery_evidence",
            "requestId": request_id,
            "learnerId": request.learner_id,
            "domain": domain.to_string(),
            "concepts": concepts.iter().map(MasteryConcept::input_readback).collect::<Vec<_>>(),
            "payloadSha256": body_hash
        }))
        .map_err(|error| OriginError::internal(error.to_string()))?;
        let cx_id = vault.cx_id_for_input(&input_bytes, panel.version);
        let cx = build_mastery_constellation(
            vault,
            cx_id,
            request,
            request_id,
            &domain,
            &concepts,
            &input_bytes,
            body_hash,
            now,
        );
        let clamp = concepts
            .iter()
            .filter(|concept| concept.measured)
            .map(|concept| concept.lens_id)
            .collect::<SlotSet>();
        let free = concepts
            .iter()
            .filter(|concept| !concept.measured)
            .map(|concept| concept.lens_id)
            .collect::<SlotSet>();
        let held_out = trust_gate.held_out_split(request_id, cx_id);
        Ok(Self {
            domain,
            panel,
            cx,
            clamp,
            free,
            region: MasteryRegion::new(&concepts),
            oracle,
            trust_gate: trust_gate.with_sufficiency(panel_bits, anchor_entropy_bits),
            held_out,
            concepts,
        })
    }

    fn persist_assay_rows(
        &self,
        vault: &calyx_aster::vault::AsterVault<SystemClock>,
        now: u64,
    ) -> Result<usize, OriginError> {
        let mut store = AssayStore::default();
        let key = AssayCacheKey::scoped(
            self.panel.version,
            self.domain.as_str(),
            vault.vault_id(),
            AnchorKind::Reward,
        );
        store.put(
            key.clone(),
            AssaySubject::Panel,
            MiEstimate::point(
                self.trust_gate.panel_bits,
                self.trust_gate.sample_count,
                EstimatorKind::PanelSufficiency,
                TrustTag::Trusted,
            ),
            "learner-origin mastery panel sufficiency",
            now,
        );
        store.put(
            key.clone(),
            AssaySubject::OutcomeEntropy,
            MiEstimate::point(
                self.trust_gate.anchor_entropy_bits,
                self.trust_gate.sample_count,
                EstimatorKind::OutcomeEntropy,
                TrustTag::Trusted,
            ),
            "learner-origin mastery outcome entropy",
            now,
        );
        let per_slot_bits = if self.concepts.is_empty() {
            0.0
        } else {
            self.trust_gate.panel_bits / self.concepts.len() as f32
        };
        for concept in &self.concepts {
            store.put(
                key.clone(),
                AssaySubject::Lens {
                    slot: concept.slot_id,
                },
                MiEstimate::point(
                    per_slot_bits,
                    self.trust_gate.sample_count,
                    EstimatorKind::Ksg,
                    TrustTag::Trusted,
                ),
                format!("learner-origin mastery lens {}", concept.concept_id),
                now,
            );
        }
        store.persist_to_vault(vault).map_err(storage_error)
    }

    fn trust_sources(&self) -> MasteryTrustSources {
        MasteryTrustSources {
            oracle: MasteryOracleSource(self.oracle.clone()),
            kernel: MasteryKernelSource {
                ratio: self.trust_gate.kernel_recall_ratio,
            },
            calibration: MasteryCalibrationSource(CalibrationMeasurement {
                calibration_error: self.trust_gate.calibration_error,
                held_out_count: self.held_out.held_out_count(),
                calibrated_slots: self.concepts.len().max(1),
            }),
            goodhart: MasteryGoodhartSource(GoodhartDefenseMeasurement {
                pass_rate: self.trust_gate.goodhart_pass_rate,
                held_out_count: self.held_out.held_out_count(),
                report_passed: self.trust_gate.goodhart_passed,
                violation_count: self.trust_gate.goodhart_violations,
            }),
            mistakes: MasteryMistakeSource(MistakeClosureMeasurement {
                recurring_mistakes: self.trust_gate.recurring_mistakes,
                replayed_mistakes: self.trust_gate.replayed_mistakes,
            }),
        }
    }

    fn slot_readbacks(&self, completion: &calyx_oracle::CompletionResult) -> Vec<Value> {
        completion
            .filled_cx
            .iter()
            .filter_map(|slot| {
                self.concepts
                    .iter()
                    .find(|concept| concept.lens_id == slot.lens_id)
                    .map(|concept| {
                        json!({
                            "conceptId": concept.concept_id,
                            "measured": concept.measured,
                            "tag": slot.tag,
                            "mastery": slot.vector.first().copied().unwrap_or(0.0),
                            "lensId": slot.lens_id,
                            "slotId": concept.slot_id
                        })
                    })
            })
            .collect()
    }
}

#[derive(Clone)]
struct MasteryConcept {
    concept_id: String,
    slot_id: SlotId,
    lens_id: LensId,
    measured: bool,
    mastery: f32,
    trusted_mastery: f32,
}

impl MasteryConcept {
    fn input_readback(&self) -> Value {
        json!({
            "conceptId": self.concept_id,
            "measured": self.measured,
            "mastery": self.mastery,
            "trustedMastery": self.trusted_mastery
        })
    }
}

#[derive(Clone)]
struct MasteryTrustGate {
    panel_bits: f32,
    anchor_entropy_bits: f32,
    sample_count: usize,
    held_out_count: usize,
    kernel_recall_ratio: f32,
    calibration_error: f32,
    goodhart_pass_rate: f32,
    goodhart_passed: bool,
    goodhart_violations: usize,
    recurring_mistakes: usize,
    replayed_mistakes: usize,
}

impl MasteryTrustGate {
    fn from_request(request: &MasteryTrustGateRequest) -> Result<Self, OriginError> {
        let held_out_count = request.held_out_count;
        Ok(Self {
            panel_bits: 0.0,
            anchor_entropy_bits: 0.0,
            sample_count: held_out_count.max(1),
            held_out_count,
            kernel_recall_ratio: require_unit_interval(
                "trustGate.kernelRecallRatio",
                request.kernel_recall_ratio,
            )?,
            calibration_error: require_unit_interval(
                "trustGate.calibrationError",
                request.calibration_error,
            )?,
            goodhart_pass_rate: require_unit_interval(
                "trustGate.goodhartPassRate",
                request.goodhart_pass_rate,
            )?,
            goodhart_passed: request
                .goodhart_passed
                .unwrap_or(request.goodhart_pass_rate >= calyx_oracle::GOODHART_THRESHOLD),
            goodhart_violations: request.goodhart_violations.unwrap_or(0),
            recurring_mistakes: request.recurring_mistakes,
            replayed_mistakes: request
                .replayed_mistakes
                .unwrap_or(request.recurring_mistakes),
        })
    }

    fn with_sufficiency(mut self, panel_bits: f32, anchor_entropy_bits: f32) -> Self {
        self.panel_bits = panel_bits;
        self.anchor_entropy_bits = anchor_entropy_bits;
        self
    }

    fn held_out_split(&self, request_id: &str, source_cx: CxId) -> HeldOutSplit {
        let held_out_ids = (0..self.held_out_count)
            .map(|index| {
                CxId::from_bytes(content_address([
                    b"mastery-held-out".as_slice(),
                    request_id.as_bytes(),
                    &index.to_be_bytes(),
                ]))
            })
            .collect();
        HeldOutSplit::new(
            format!("mastery-estimate:{request_id}"),
            vec![source_cx],
            held_out_ids,
        )
    }
}

struct MasteryTrustSources {
    oracle: MasteryOracleSource,
    kernel: MasteryKernelSource,
    calibration: MasteryCalibrationSource,
    goodhart: MasteryGoodhartSource,
    mistakes: MasteryMistakeSource,
}

#[derive(Clone)]
struct MasteryOracleSource(OracleSelfConsistency);

impl OracleConsistencySource for MasteryOracleSource {
    fn oracle_self_consistency(
        &self,
        _domain: DomainId,
        _clock: &dyn Clock,
    ) -> Result<OracleSelfConsistency, OracleError> {
        Ok(self.0.clone())
    }
}

struct MasteryKernelSource {
    ratio: f32,
}

impl KernelRecallSource for MasteryKernelSource {
    fn kernel_recall_report(
        &self,
        held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<RecallReport, LodestarError> {
        Ok(RecallReport {
            kernel_only: self.ratio,
            full: 1.0,
            ratio: self.ratio,
            approx_factor: 1.0,
            tau_star_estimate: held_out.held_out_count(),
            tau_star_exact: true,
            recall_test_params: None,
            corpus_name: Some("calyxweb-learner-mastery".to_string()),
            n_queries_tested: held_out.held_out_count(),
            held_out: held_out.held_out_ids.clone(),
            warning: None,
        })
    }
}

struct MasteryCalibrationSource(CalibrationMeasurement);

impl CalibrationSource for MasteryCalibrationSource {
    fn calibration_measurement(
        &self,
        _domain: &DomainId,
        _held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<CalibrationMeasurement, OracleError> {
        Ok(self.0.clone())
    }
}

struct MasteryGoodhartSource(GoodhartDefenseMeasurement);

impl GoodhartDefenseSource for MasteryGoodhartSource {
    fn goodhart_defense_measurement(
        &self,
        _domain: &DomainId,
        _held_out: &HeldOutSplit,
        _clock: &dyn Clock,
    ) -> Result<GoodhartDefenseMeasurement, OracleError> {
        Ok(self.0.clone())
    }
}

struct MasteryMistakeSource(MistakeClosureMeasurement);

impl MistakeClosureSource for MasteryMistakeSource {
    fn mistake_closure_measurement(
        &self,
        _domain: &DomainId,
        _clock: &dyn Clock,
    ) -> Result<MistakeClosureMeasurement, OracleError> {
        Ok(self.0.clone())
    }
}

#[derive(Default)]
struct MasteryRegion {
    members: BTreeMap<LensId, Vec<Vec<f32>>>,
}

impl MasteryRegion {
    fn new(concepts: &[MasteryConcept]) -> Self {
        let members = concepts
            .iter()
            .map(|concept| (concept.lens_id, vec![vec![concept.trusted_mastery]]))
            .collect();
        Self { members }
    }
}

impl CompletionRegion for MasteryRegion {
    fn members_for_lens(
        &self,
        _domain: &DomainId,
        _cx: &Constellation,
        lens_id: LensId,
    ) -> Result<Vec<Vec<f32>>, OracleError> {
        Ok(self.members.get(&lens_id).cloned().unwrap_or_default())
    }
}

struct MasteryAnneal;

impl AnnealConfig for MasteryAnneal {
    fn energy_beta(&self, _domain: &DomainId) -> Option<f32> {
        Some(1.0)
    }
}

impl crate::learner_origin::model::OracleSelfConsistencyRequest {
    fn to_oracle(&self) -> Result<OracleSelfConsistency, OriginError> {
        let flakiness = require_unit_interval("oracleSelfConsistency.flakiness", self.flakiness)?;
        let validity = require_unit_interval("oracleSelfConsistency.validity", self.validity)?;
        Ok(OracleSelfConsistency::with_provenance(
            flakiness,
            validity,
            self.provisional,
            None,
        ))
    }
}

fn build_mastery_concepts(
    inputs: &[crate::learner_origin::model::MasteryConceptRequest],
) -> Result<Vec<MasteryConcept>, OriginError> {
    if inputs.is_empty() {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_FIELD_REQUIRED",
            "concepts must contain at least one measured and one un-probed concept",
        ));
    }
    if inputs.len() > 256 {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_TOO_MANY_CONCEPTS",
            "mastery estimate accepts at most 256 concepts",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut measured_count = 0_usize;
    let mut free_count = 0_usize;
    let mut out = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        ensure_nonempty("conceptId", &input.concept_id)?;
        if !seen.insert(input.concept_id.as_str()) {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_DUPLICATE_CONCEPT",
                format!("duplicate conceptId {}", input.concept_id),
            ));
        }
        let measured = input.mastery.is_some();
        let mastery = match input.mastery {
            Some(value) => {
                measured_count += 1;
                require_unit_interval("concept.mastery", value)?
            }
            None => {
                free_count += 1;
                0.0
            }
        };
        let trusted_mastery = match input.trusted_mastery {
            Some(value) => require_unit_interval("concept.trustedMastery", value)?,
            None if measured => mastery,
            None => {
                return Err(OriginError::bad_request(
                    "CALYX_ORIGIN_FIELD_REQUIRED",
                    format!(
                        "un-probed concept {} requires trustedMastery",
                        input.concept_id
                    ),
                ));
            }
        };
        let slot_id = SlotId::new((index + 1) as u16);
        out.push(MasteryConcept {
            concept_id: input.concept_id.clone(),
            slot_id,
            lens_id: LensId::from_bytes(content_address([
                b"calyxweb-mastery-lens".as_slice(),
                input.concept_id.as_bytes(),
                &(index as u64).to_be_bytes(),
            ])),
            measured,
            mastery,
            trusted_mastery,
        });
    }
    if measured_count == 0 || free_count == 0 {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_FIELD_REQUIRED",
            "mastery imputation requires at least one measured concept and one un-probed concept",
        ));
    }
    Ok(out)
}

fn build_mastery_panel(concepts: &[MasteryConcept], now: u64) -> Panel {
    Panel {
        version: 1247,
        slots: concepts
            .iter()
            .map(|concept| Slot {
                slot_id: concept.slot_id,
                slot_key: concept
                    .slot_id
                    .with_key(format!("mastery-{}", concept.slot_id.get())),
                lens_id: concept.lens_id,
                shape: SlotShape::Dense(1),
                modality: Modality::Structured,
                asymmetry: Asymmetry::None,
                quant: QuantPolicy::None,
                resource: Default::default(),
                axis: Some(format!("mastery:{}", concept.concept_id)),
                retrieval_only: false,
                excluded_from_dedup: false,
                bits_about: BTreeMap::new(),
                state: SlotState::Active,
                added_at_panel_version: 1247,
            })
            .collect(),
        created_at: now,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn build_mastery_constellation(
    vault: &calyx_aster::vault::AsterVault<SystemClock>,
    cx_id: CxId,
    request: &MasteryEstimateRequest,
    request_id: &str,
    domain: &DomainId,
    concepts: &[MasteryConcept],
    input_bytes: &[u8],
    body_hash: &str,
    now: u64,
) -> Constellation {
    let slots = concepts
        .iter()
        .map(|concept| {
            let vector = if concept.measured {
                SlotVector::Dense {
                    dim: 1,
                    data: vec![concept.mastery],
                }
            } else {
                SlotVector::Absent {
                    reason: AbsentReason::Deferred,
                }
            };
            (concept.slot_id, vector)
        })
        .collect();
    let metadata = BTreeMap::from([
        ("origin_kind".to_string(), "mastery_evidence".to_string()),
        ("origin_version".to_string(), "1".to_string()),
        ("payload_sha256".to_string(), body_hash.to_string()),
        ("request_id".to_string(), request_id.to_string()),
        ("learner_id".to_string(), request.learner_id.clone()),
        ("domain".to_string(), domain.to_string()),
        ("concept_count".to_string(), concepts.len().to_string()),
    ]);
    Constellation {
        cx_id,
        vault_id: vault.vault_id(),
        panel_version: 1247,
        created_at: now,
        input_ref: InputRef {
            hash: sha256_array(input_bytes),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Structured,
        slots,
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            redacted_input: true,
            ..CxFlags::default()
        },
    }
}

fn require_unit_interval(field: &str, value: f32) -> Result<f32, OriginError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(OriginError::bad_request(
            "CALYX_ORIGIN_INVALID_NUMBER",
            format!("{field} must be finite and within [0, 1]"),
        ))
    }
}

fn require_nonnegative_bits(field: &str, value: f32) -> Result<f32, OriginError> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(OriginError::bad_request(
            "CALYX_ORIGIN_INVALID_NUMBER",
            format!("{field} must be finite and non-negative"),
        ))
    }
}

fn oracle_origin_error(error: OracleError) -> OriginError {
    OriginError::new(
        STATUS_UNPROCESSABLE,
        "CALYX_ORIGIN_ORACLE_REJECTED",
        format!("{}: {} ({})", error.code(), error, error.remediation()),
    )
}
