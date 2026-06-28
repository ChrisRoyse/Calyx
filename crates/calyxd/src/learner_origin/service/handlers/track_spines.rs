use std::collections::BTreeMap;

use calyx_ledger::EntryKind;
use serde_json::json;

use crate::learner_origin::model::{KIND_TRACK_SPINES, TrackSpinesRequest};
use crate::learner_origin::privacy::reject_private_material;

use super::super::storage::OriginCommit;
use super::super::{
    LearnerOriginService, OriginError, OriginResponse, STATUS_CREATED, STATUS_UNPROCESSABLE,
    base_metadata, ensure_nonempty, insert_optional, now_millis, parse_body, sha256_hex, stable_id,
};
use super::TRACK_SPINES_EVIDENCE_KIND;
use super::track_spines_plan::{TrackSpinesOutput, TrackSpinesPlan};
impl LearnerOriginService {
    pub(in crate::learner_origin::service) fn handle_track_spines(
        &self,
        body: &[u8],
    ) -> Result<OriginResponse, OriginError> {
        let value = parse_body(body)?;
        reject_private_material(&value)
            .map_err(|detail| OriginError::bad_request("CALYX_ORIGIN_PRIVATE_FIELD", detail))?;
        let request: TrackSpinesRequest = serde_json::from_value(value)
            .map_err(|error| OriginError::bad_request("CALYX_ORIGIN_JSON_INVALID", error))?;
        ensure_nonempty("learnerId", &request.learner_id)?;
        let domain = request
            .domain
            .as_deref()
            .unwrap_or("calyxweb-learner-kernels");
        ensure_nonempty("domain", domain)?;
        let body_hash = sha256_hex(body);
        let request_id = request.request_id.clone().unwrap_or_else(|| {
            stable_id(
                "track-spines",
                [request.learner_id.as_str(), domain, body_hash.as_str()],
            )
        });
        if let Some(existing) = self.find_by_idempotency(
            KIND_TRACK_SPINES,
            "request_id",
            &request_id,
            request.idempotency_key.as_deref(),
        )? {
            return self.duplicate_response(
                KIND_TRACK_SPINES,
                "requestId",
                &request_id,
                &body_hash,
                existing,
            );
        }

        let now = request.now_millis.unwrap_or_else(now_millis);
        let plan = TrackSpinesPlan::from_request(&request, domain, &request_id, now)?;
        let source_row =
            self.commit_track_spines_source(&request, &plan, &request_id, &body_hash)?;
        let output = plan.run()?;
        if output.provisional_positive_count == 0 {
            return Err(OriginError::new(
                STATUS_UNPROCESSABLE,
                "CALYX_ORIGIN_TRACK_SPINES_UNGROUNDED",
                "label propagation produced no positive provisional mastery labels",
            ));
        }

        let stored = self.commit_track_spines_result(
            &request,
            &plan,
            &output,
            &source_row.cx_id,
            &body_hash,
        )?;
        self.metrics.record_write(KIND_TRACK_SPINES, "accepted");
        Ok(OriginResponse::json(
            STATUS_CREATED,
            json!({
                "accepted": true,
                "duplicate": false,
                "requestId": request_id,
                "learnerId": request.learner_id,
                "domain": domain,
                "source": {
                    "cxId": source_row.cx_id,
                    "ledgerSeq": source_row.ledger_seq,
                    "ledgerHash": source_row.ledger_hash,
                    "nodeCount": plan.graph.node_count(),
                    "edgeCount": plan.graph.edge_count(),
                    "trackCount": plan.track_count(),
                    "masteryLabelCount": plan.kernel_labels.len()
                },
                "tracks": output.track_reports,
                "labelPropagation": {
                    "kernelLabelCount": plan.kernel_labels.len(),
                    "labelCount": output.label_count,
                    "provisionalPositiveCount": output.provisional_positive_count,
                    "maxHopDistance": output.max_hop_distance,
                    "decayLambda": plan.decay_lambda,
                    "labels": output.label_rows
                },
                "cxId": stored.cx_id,
                "ledgerSeq": stored.ledger_seq,
                "ledgerHash": stored.ledger_hash
            }),
        ))
    }

    fn commit_track_spines_source(
        &self,
        request: &TrackSpinesRequest,
        plan: &TrackSpinesPlan,
        request_id: &str,
        body_hash: &str,
    ) -> Result<super::super::storage::StoredRow, OriginError> {
        let mut metadata = base_metadata(TRACK_SPINES_EVIDENCE_KIND, body_hash);
        metadata.insert("request_id".to_string(), request_id.to_string());
        metadata.insert("learner_id".to_string(), request.learner_id.clone());
        metadata.insert("domain".to_string(), plan.domain.clone());
        metadata.insert(
            "graph_node_count".to_string(),
            plan.graph.node_count().to_string(),
        );
        metadata.insert(
            "graph_edge_count".to_string(),
            plan.graph.edge_count().to_string(),
        );
        metadata.insert("track_count".to_string(), plan.track_count().to_string());
        metadata.insert(
            "mastery_label_count".to_string(),
            plan.kernel_labels.len().to_string(),
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
        self.commit_origin_row(OriginCommit {
            kind: TRACK_SPINES_EVIDENCE_KIND,
            primary_id: request_id.to_string(),
            ledger_kind: EntryKind::Ingest,
            metadata,
            scalars: BTreeMap::from([
                (
                    "track_spines.graph_nodes".to_string(),
                    plan.graph.node_count() as f64,
                ),
                (
                    "track_spines.graph_edges".to_string(),
                    plan.graph.edge_count() as f64,
                ),
                ("track_spines.tracks".to_string(), plan.track_count() as f64),
            ]),
            slot_values: [
                7.0,
                plan.graph.node_count() as f32,
                plan.track_count() as f32,
                plan.kernel_labels.len() as f32,
            ],
            anchors: Vec::new(),
        })
    }

    fn commit_track_spines_result(
        &self,
        request: &TrackSpinesRequest,
        plan: &TrackSpinesPlan,
        output: &TrackSpinesOutput,
        source_cx_id: &str,
        body_hash: &str,
    ) -> Result<super::super::storage::StoredRow, OriginError> {
        let mut metadata = base_metadata(KIND_TRACK_SPINES, body_hash);
        metadata.insert("request_id".to_string(), output.request_id.clone());
        metadata.insert("learner_id".to_string(), request.learner_id.clone());
        metadata.insert("domain".to_string(), plan.domain.clone());
        metadata.insert("source_cx_id".to_string(), source_cx_id.to_string());
        metadata.insert("track_count".to_string(), plan.track_count().to_string());
        metadata.insert("label_count".to_string(), output.label_count.to_string());
        metadata.insert(
            "provisional_positive_count".to_string(),
            output.provisional_positive_count.to_string(),
        );
        metadata.insert(
            "max_hop_distance".to_string(),
            output.max_hop_distance.to_string(),
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
        self.commit_origin_row(OriginCommit {
            kind: KIND_TRACK_SPINES,
            primary_id: output.request_id.clone(),
            ledger_kind: EntryKind::Kernel,
            metadata,
            scalars: BTreeMap::from([
                (
                    "track_spines.track_count".to_string(),
                    plan.track_count() as f64,
                ),
                (
                    "track_spines.label_count".to_string(),
                    output.label_count as f64,
                ),
                (
                    "track_spines.provisional_positive_count".to_string(),
                    output.provisional_positive_count as f64,
                ),
            ]),
            slot_values: [
                8.0,
                plan.track_count() as f32,
                output.label_count as f32,
                output.provisional_positive_count as f32,
            ],
            anchors: Vec::new(),
        })
    }
}
