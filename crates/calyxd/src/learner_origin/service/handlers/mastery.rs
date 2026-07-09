use std::collections::BTreeMap;

use calyx_core::SystemClock;
use calyx_ledger::EntryKind;
use calyx_oracle::{
    ShortCircuit, SuperIntelligenceRequest, VaultSufficiencyAssay, complete,
    super_intelligence_with_ledger,
};
use serde_json::json;

use crate::learner_origin::model::{KIND_MASTERY_ESTIMATE, MasteryEstimateRequest};
use crate::learner_origin::privacy::reject_private_material;

use super::super::storage::OriginCommit;
use super::super::{
    LearnerOriginService, OriginError, OriginResponse, STATUS_CREATED, base_metadata,
    ensure_nonempty, hex, insert_optional, now_millis, parse_body, sha256_hex, stable_id,
    storage_error,
};
use super::mastery_plan::MasteryPlan;
use super::mastery_support::{
    MASTERY_CERTIFICATION_BLOCKED_REASON, MASTERY_CLIENT_ATTESTED, MASTERY_MEASUREMENT_PROVENANCE,
    MasteryAnneal,
};
use super::shared::oracle_origin_error;

impl LearnerOriginService {
    pub(in crate::learner_origin::service) fn handle_mastery_estimate(
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
        let client_attested = MASTERY_CLIENT_ATTESTED;
        let certification_eligible =
            !client_attested && trust_report.overall && provisional_count == 0;
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
        metadata.insert("client_attested".to_string(), client_attested.to_string());
        metadata.insert(
            "measurement_provenance".to_string(),
            MASTERY_MEASUREMENT_PROVENANCE.to_string(),
        );
        metadata.insert(
            "certification_blocked_reason".to_string(),
            MASTERY_CERTIFICATION_BLOCKED_REASON.to_string(),
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
                "mastery.energy_score".to_string(),
                completion.energy_score as f64,
            ),
            (
                "mastery.trust_overall".to_string(),
                if trust_report.overall { 1.0 } else { 0.0 },
            ),
            (
                "mastery.certification_eligible".to_string(),
                if certification_eligible { 1.0 } else { 0.0 },
            ),
            (
                "mastery.client_attested".to_string(),
                if client_attested { 1.0 } else { 0.0 },
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
                completion.energy_score,
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
                    "assayRows": assay_rows,
                    "clientAttested": client_attested,
                    "measurementProvenance": MASTERY_MEASUREMENT_PROVENANCE
                },
                "completion": {
                    "energyScore": completion.energy_score,
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
                    "ledgerHash": hex(&trust_ledger.hash),
                    "clientAttested": client_attested,
                    "measurementProvenance": MASTERY_MEASUREMENT_PROVENANCE,
                    "certificationBlockedReason": MASTERY_CERTIFICATION_BLOCKED_REASON
                },
                "clientAttested": client_attested,
                "measurementProvenance": MASTERY_MEASUREMENT_PROVENANCE,
                "certificationBlockedReason": MASTERY_CERTIFICATION_BLOCKED_REASON,
                "certificationEligible": certification_eligible,
                "cxId": stored.cx_id,
                "ledgerSeq": stored.ledger_seq,
                "ledgerHash": stored.ledger_hash
            }),
        ))
    }
}
