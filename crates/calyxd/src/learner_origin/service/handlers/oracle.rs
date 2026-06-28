use std::collections::BTreeMap;

use calyx_aster::cf::ColumnFamily;
use calyx_core::{Clock, SystemClock, content_address};
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};
use calyx_oracle::{build_tree, oracle_predict, reverse_query, select};
use serde_json::json;

use crate::learner_origin::model::{KIND_ORACLE_FORECAST, OracleForecastRequest};
use crate::learner_origin::privacy::reject_private_material;

use super::super::storage::OriginCommit;
use super::super::{
    LearnerOriginService, ORIGIN_ACTOR, OriginError, OriginResponse, STATUS_CREATED,
    STATUS_UNPROCESSABLE, base_metadata, ensure_nonempty, insert_optional, now_millis, parse_body,
    sha256_hex, stable_id, storage_error,
};
use super::oracle_graph::tree_ledger_seq;
use super::oracle_plan::OracleForecastPlan;
use super::shared::oracle_origin_error;
use super::{ORACLE_FORECAST_EVIDENCE_KIND, ORACLE_FORECAST_GRAPH_KIND};

impl LearnerOriginService {
    pub(in crate::learner_origin::service) fn handle_oracle_forecast(
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
}

struct OracleGraphCommit {
    ledger_seq: u64,
    base_rows: usize,
    recurrence_rows: usize,
}
