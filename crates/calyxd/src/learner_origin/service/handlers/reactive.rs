use std::collections::BTreeMap;
use std::sync::Arc;

use calyx_core::{Clock, CxId, FixedClock, SystemClock};
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};
use calyx_loom::{AgreementDriftTracker, ReactiveEngine, ReactiveSignalSet, TriggerCondition};
use calyx_ward::{
    Domain as WardDomain, classify_novelty, novelty_action_for_signal, surprise_bits,
};
use serde_json::json;

use crate::learner_origin::model::{KIND_REACTIVE_AFFECT, ReactiveAffectRequest};
use crate::learner_origin::privacy::reject_private_material;

use super::super::storage::OriginCommit;
use super::super::{
    LearnerOriginService, ORIGIN_ACTOR, OriginError, OriginResponse, STATUS_CREATED,
    STATUS_UNPROCESSABLE, base_metadata, ensure_nonempty, insert_optional, now_millis, parse_body,
    sha256_hex, stable_id, storage_error,
};
use super::REACTIVE_AFFECT_RECURRENCE_KIND;
use super::reactive_plan::{ReactiveAffectPlan, ReactiveRows};
use super::reactive_support::{
    novelty_action_name, reactive_interventions, reactive_origin_error, stored_ledger_ref,
    trigger_events_json, trigger_id_for_subscription, ward_origin_error,
};

struct ReactiveRecurrenceCommit {
    ledger_seq: u64,
    rows: usize,
}

impl LearnerOriginService {
    pub(in crate::learner_origin::service) fn handle_reactive_affect(
        &self,
        body: &[u8],
    ) -> Result<OriginResponse, OriginError> {
        let value = parse_body(body)?;
        reject_private_material(&value)
            .map_err(|detail| OriginError::bad_request("CALYX_ORIGIN_PRIVATE_FIELD", detail))?;
        let request: ReactiveAffectRequest = serde_json::from_value(value)
            .map_err(|error| OriginError::bad_request("CALYX_ORIGIN_JSON_INVALID", error))?;
        ensure_nonempty("learnerId", &request.learner_id)?;
        ensure_nonempty("conceptId", &request.concept_id)?;
        let body_hash = sha256_hex(body);
        let request_id = request.request_id.clone().unwrap_or_else(|| {
            stable_id(
                "reactive-affect",
                [
                    request.learner_id.as_str(),
                    request
                        .domain
                        .as_deref()
                        .unwrap_or("calyxweb-reactive-affect"),
                    request.concept_id.as_str(),
                    body_hash.as_str(),
                ],
            )
        });
        if let Some(existing) = self.find_by_idempotency(
            KIND_REACTIVE_AFFECT,
            "request_id",
            &request_id,
            request.idempotency_key.as_deref(),
        )? {
            return self.duplicate_response(
                KIND_REACTIVE_AFFECT,
                "requestId",
                &request_id,
                &body_hash,
                existing,
            );
        }

        let now = request.now_millis.unwrap_or_else(now_millis);
        let plan =
            ReactiveAffectPlan::from_request(&request, &request_id, &body_hash, now, &self.vault)?;
        let matched_row = self.commit_constellation_row(
            plan.matched_cx.clone(),
            super::REACTIVE_AFFECT_MATCHED_KIND,
            &request_id,
            EntryKind::Ingest,
            &body_hash,
        )?;
        let baseline_row = self.commit_constellation_row(
            plan.baseline_cx.clone(),
            super::REACTIVE_AFFECT_BASELINE_KIND,
            &request_id,
            EntryKind::Ingest,
            &body_hash,
        )?;
        let current_row = self.commit_constellation_row(
            plan.current_cx.clone(),
            super::REACTIVE_AFFECT_EVIDENCE_KIND,
            &request_id,
            EntryKind::Ingest,
            &body_hash,
        )?;
        let recurrence_commit = self.commit_reactive_recurrence_rows(
            plan.recurrence_rows.clone(),
            &request_id,
            &body_hash,
            plan.current_cx.cx_id,
        )?;

        let clock = FixedClock::new(now);
        let novelty_signal = classify_novelty(plan.current_cx.cx_id, &self.vault, &clock)
            .map_err(ward_origin_error)?;
        let surprise_domain = WardDomain::new(
            format!("reactive-affect:{request_id}"),
            vec![plan.current_cx.cx_id, plan.matched_cx.cx_id],
        );
        let surprise = surprise_bits(plan.current_cx.cx_id, &surprise_domain, &self.vault)
            .map_err(ward_origin_error)?;
        let novelty_action = novelty_action_for_signal(&novelty_signal);
        let mmd = plan.mmd.readback()?;

        let mut engine = ReactiveEngine::new(Arc::new(FixedClock::new(now)));
        let tracker = AgreementDriftTracker::new();
        let owner = Some(format!("learner:{}", request.learner_id));
        let drift_sub = engine
            .subscribe_durable(
                &self.vault,
                TriggerCondition::DriftDetected {
                    slot: plan.slot,
                    drift_threshold: plan.drift_threshold,
                },
                owner.clone(),
            )
            .map_err(reactive_origin_error)?;
        let drift_trigger_id = trigger_id_for_subscription(&engine, drift_sub)?;
        let baseline_signals = ReactiveSignalSet::new(&self.vault)
            .with_ward_novelty(plan.profile.clone(), plan.matched_cx.cx_id, false)
            .with_agreement_drift(plan.baseline_cx.cx_id, &tracker);
        engine
            .evaluate_post_ingest_durable(
                &self.vault,
                plan.baseline_cx.cx_id,
                stored_ledger_ref(baseline_row.ledger_seq, &baseline_row.ledger_hash)?,
                &baseline_signals,
            )
            .map_err(reactive_origin_error)?;
        let baseline_reactive_ledger_seq = self.vault.latest_seq();

        let new_region_sub = engine
            .subscribe_durable(
                &self.vault,
                TriggerCondition::NewRegion {
                    tau_override: Some(plan.tau),
                },
                owner,
            )
            .map_err(reactive_origin_error)?;
        let new_region_trigger_id = trigger_id_for_subscription(&engine, new_region_sub)?;
        let current_signals = ReactiveSignalSet::new(&self.vault)
            .with_ward_novelty(plan.profile.clone(), plan.matched_cx.cx_id, false)
            .with_agreement_drift(plan.current_cx.cx_id, &tracker);
        let fired_count = engine
            .evaluate_post_ingest_durable(
                &self.vault,
                plan.current_cx.cx_id,
                stored_ledger_ref(current_row.ledger_seq, &current_row.ledger_hash)?,
                &current_signals,
            )
            .map_err(reactive_origin_error)?;
        let current_reactive_ledger_seq = self.vault.latest_seq();
        let drift_events = engine
            .observe_delta(drift_sub)
            .map_err(reactive_origin_error)?;
        let new_region_events = engine
            .observe_delta(new_region_sub)
            .map_err(reactive_origin_error)?;
        self.vault.flush().map_err(storage_error)?;

        let interventions = reactive_interventions(
            &new_region_events,
            &drift_events,
            &mmd,
            novelty_action.as_ref(),
        );
        if interventions.is_empty() {
            return Err(OriginError::new(
                STATUS_UNPROCESSABLE,
                "CALYX_ORIGIN_REACTIVE_NO_TRIGGER",
                "reactive novelty/drift/MMD evidence did not fire an affect intervention",
            ));
        }

        let mut metadata = base_metadata(KIND_REACTIVE_AFFECT, &body_hash);
        metadata.insert("request_id".to_string(), request_id.clone());
        metadata.insert("learner_id".to_string(), request.learner_id.clone());
        metadata.insert("domain".to_string(), plan.domain.clone());
        metadata.insert("concept_id".to_string(), request.concept_id.clone());
        metadata.insert("source_cx_id".to_string(), current_row.cx_id.clone());
        metadata.insert("baseline_cx_id".to_string(), baseline_row.cx_id.clone());
        metadata.insert("matched_cx_id".to_string(), matched_row.cx_id.clone());
        metadata.insert(
            "recurrence_ledger_seq".to_string(),
            recurrence_commit.ledger_seq.to_string(),
        );
        metadata.insert(
            "recurrence_kind".to_string(),
            REACTIVE_AFFECT_RECURRENCE_KIND.to_string(),
        );
        metadata.insert(
            "baseline_reactive_ledger_seq".to_string(),
            baseline_reactive_ledger_seq.to_string(),
        );
        metadata.insert(
            "current_reactive_ledger_seq".to_string(),
            current_reactive_ledger_seq.to_string(),
        );
        metadata.insert("drift_trigger_id".to_string(), drift_trigger_id.clone());
        metadata.insert(
            "new_region_trigger_id".to_string(),
            new_region_trigger_id.clone(),
        );
        metadata.insert(
            "new_region_event_count".to_string(),
            new_region_events.len().to_string(),
        );
        metadata.insert(
            "drift_event_count".to_string(),
            drift_events.len().to_string(),
        );
        metadata.insert(
            "mmd_significant".to_string(),
            mmd.drift.significant.to_string(),
        );
        metadata.insert(
            "change_point_significant".to_string(),
            mmd.change_point.report.significant.to_string(),
        );
        metadata.insert(
            "intervention_reasons".to_string(),
            interventions
                .iter()
                .map(|intervention| intervention.reason)
                .collect::<Vec<_>>()
                .join(","),
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
                "reactive.new_region_events".to_string(),
                new_region_events.len() as f64,
            ),
            (
                "reactive.drift_events".to_string(),
                drift_events.len() as f64,
            ),
            ("reactive.surprise_bits".to_string(), surprise.get() as f64),
            ("reactive.mmd2".to_string(), mmd.drift.mmd2),
            (
                "reactive.mmd_significant".to_string(),
                if mmd.drift.significant { 1.0 } else { 0.0 },
            ),
            (
                "reactive.change_point_split".to_string(),
                mmd.change_point.split_index as f64,
            ),
            (
                "reactive.interventions".to_string(),
                interventions.len() as f64,
            ),
        ]);
        let stored = self.commit_origin_row(OriginCommit {
            kind: KIND_REACTIVE_AFFECT,
            primary_id: request_id.clone(),
            ledger_kind: EntryKind::Guard,
            metadata,
            scalars,
            slot_values: [
                6.0,
                surprise.get(),
                mmd.drift.mmd2 as f32,
                interventions.len() as f32,
            ],
            anchors: Vec::new(),
        })?;
        self.metrics.record_write(KIND_REACTIVE_AFFECT, "accepted");
        Ok(OriginResponse::json(
            STATUS_CREATED,
            json!({
                "accepted": true,
                "duplicate": false,
                "requestId": request_id,
                "learnerId": request.learner_id,
                "domain": plan.domain,
                "conceptId": request.concept_id,
                "source": {
                    "matched": {
                        "cxId": matched_row.cx_id,
                        "ledgerSeq": matched_row.ledger_seq,
                        "ledgerHash": matched_row.ledger_hash
                    },
                    "baseline": {
                        "cxId": baseline_row.cx_id,
                        "ledgerSeq": baseline_row.ledger_seq,
                        "ledgerHash": baseline_row.ledger_hash,
                        "reactiveLedgerSeq": baseline_reactive_ledger_seq
                    },
                    "current": {
                        "cxId": current_row.cx_id,
                        "ledgerSeq": current_row.ledger_seq,
                        "ledgerHash": current_row.ledger_hash,
                        "reactiveLedgerSeq": current_reactive_ledger_seq
                    },
                    "recurrenceRows": recurrence_commit.rows,
                    "recurrenceLedgerSeq": recurrence_commit.ledger_seq
                },
                "novelty": {
                    "signal": novelty_signal,
                    "action": novelty_action.map(novelty_action_name),
                    "surpriseBits": surprise.get()
                },
                "reactive": {
                    "firedCount": fired_count,
                    "driftSubscriptionId": drift_sub.to_string(),
                    "driftTriggerId": drift_trigger_id,
                    "newRegionSubscriptionId": new_region_sub.to_string(),
                    "newRegionTriggerId": new_region_trigger_id,
                    "driftEvents": trigger_events_json(&drift_events),
                    "newRegionEvents": trigger_events_json(&new_region_events)
                },
                "mmd": {
                    "drift": mmd.drift,
                    "changePoint": mmd.change_point
                },
                "interventions": interventions,
                "cxId": stored.cx_id,
                "ledgerSeq": stored.ledger_seq,
                "ledgerHash": stored.ledger_hash
            }),
        ))
    }

    fn commit_reactive_recurrence_rows(
        &self,
        rows: ReactiveRows,
        request_id: &str,
        body_hash: &str,
        current_cx: CxId,
    ) -> Result<ReactiveRecurrenceCommit, OriginError> {
        if rows.is_empty() {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_EMPTY_REACTIVE_RECURRENCE",
                "reactive affect evidence requires at least one recurrence row",
            ));
        }
        let row_count = rows.len();
        let mut payload = PayloadBuilder::default();
        payload
            .insert_str("request_id", request_id)
            .insert_str("kind", REACTIVE_AFFECT_RECURRENCE_KIND)
            .insert_str("input_hash", body_hash)
            .insert_u64("rows", row_count as u64)
            .insert_u64("ts", SystemClock.now());
        let ledger_payload = RedactionPolicy::default().apply_to_payload(&payload);
        let ledger_seq = self
            .vault
            .write_cf_batch_with_ledger_entry(
                rows,
                EntryKind::Ingest,
                SubjectId::Cx(current_cx),
                ledger_payload,
                ActorId::Service(ORIGIN_ACTOR.to_string()),
            )
            .map_err(storage_error)?;
        self.vault.flush().map_err(storage_error)?;
        Ok(ReactiveRecurrenceCommit {
            ledger_seq,
            rows: row_count,
        })
    }
}
