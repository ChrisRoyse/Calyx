use serde_json::{Value, json};

use calyx_assay::{AssayStore, TrustTag};
use calyx_aster::cf::ColumnFamily;

use super::*;

mod reactive;
mod track_spines;

fn service(name: &str) -> LearnerOriginService {
    let dir = std::env::temp_dir().join(format!(
        "calyxd-origin-{name}-{}-{}",
        std::process::id(),
        now_millis()
    ));
    std::fs::create_dir_all(&dir).expect("create temp origin vault");
    LearnerOriginService::open(
        dir,
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        b"origin-test-salt".to_vec(),
        "secret-token".to_string(),
        32 * 1024,
    )
    .expect("open service")
}

fn post(service: &LearnerOriginService, path: &str, body: Value) -> OriginResponse {
    let bytes = serde_json::to_vec(&body).expect("serialize request");
    service.handle("POST", path, Some("Bearer secret-token"), &bytes)
}

#[test]
fn happy_path_writes_three_origin_rows() {
    let service = service("happy");
    let signal = post(
        &service,
        "/v1/learner-signals/batches",
        json!({
            "batchId": "batch-a",
            "idempotencyKey": "idem-a",
            "learnerId": "learner-a",
            "events": [{"conceptId": "fractions", "score": 0.8}]
        }),
    );
    assert_eq!(signal.status, STATUS_CREATED, "{}", signal.body);
    let decision = post(
        &service,
        "/v1/interventions/decide",
        json!({
            "decisionId": "decision-a",
            "learnerId": "learner-a",
            "conceptId": "fractions",
            "confidence": 0.7,
            "evidenceIds": ["batch-a"]
        }),
    );
    assert_eq!(decision.status, STATUS_CREATED, "{}", decision.body);
    let outcome = post(
        &service,
        "/v1/interventions/decision-a/outcomes",
        json!({
            "outcomeId": "outcome-a",
            "learnerId": "learner-a",
            "outcome": "accepted"
        }),
    );
    assert_eq!(outcome.status, STATUS_CREATED, "{}", outcome.body);
    let rows = service.base_rows();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.iter().map(|row| row.anchors.len()).sum::<usize>(), 1);
    assert_eq!(
        service
            .vault
            .scan_cf_at(service.latest_seq(), ColumnFamily::Anchors)
            .expect("scan anchors")
            .len(),
        1
    );
    let seqs = rows
        .iter()
        .map(|row| row.provenance.seq)
        .collect::<Vec<_>>();
    assert_eq!(seqs, vec![0, 1, 2]);
    assert!(rows.iter().all(|row| row.provenance.hash != [0; 32]));
    assert!(service.latest_seq() >= 3);
}

#[test]
fn private_payload_rejected_before_vault_write() {
    let service = service("private");
    let before = service.latest_seq();
    let response = post(
        &service,
        "/v1/learner-signals/batches",
        json!({
            "batchId": "batch-private",
            "learnerId": "learner-a",
            "events": [{"password": "do-not-store"}]
        }),
    );
    assert_eq!(response.status, STATUS_BAD_REQUEST);
    assert_eq!(service.latest_seq(), before);
    assert!(service.base_rows().is_empty());
}

#[test]
fn duplicate_idempotency_does_not_append() {
    let service = service("duplicate");
    let body = json!({
        "batchId": "batch-dup",
        "idempotencyKey": "idem-dup",
        "learnerId": "learner-a",
        "events": [{"conceptId": "fractions"}]
    });
    let first = post(&service, "/v1/learner-signals/batches", body.clone());
    assert_eq!(first.status, STATUS_CREATED, "{}", first.body);
    let after_first = service.latest_seq();
    let duplicate = post(&service, "/v1/learner-signals/batches", body);
    assert_eq!(duplicate.status, STATUS_OK);
    assert_eq!(service.latest_seq(), after_first);
    assert_eq!(service.base_rows().len(), 1);
}

#[test]
fn cooldown_decision_returns_no_widgets() {
    let service = service("cooldown");
    let response = post(
        &service,
        "/v1/interventions/decide",
        json!({
            "decisionId": "decision-cooldown",
            "learnerId": "learner-a",
            "conceptId": "fractions",
            "nowMillis": 10,
            "cooldownUntil": 99
        }),
    );
    assert_eq!(response.status, STATUS_CREATED, "{}", response.body);
    let body: Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(body["need"], "none");
    assert_eq!(body["allowedWidgetKinds"].as_array().unwrap().len(), 0);
}

#[test]
fn mastery_estimate_imputes_unprobed_concept_and_persists_trust_gate() {
    let service = service("mastery");
    let response = post(
        &service,
        "/v1/mastery/estimate",
        json!({
            "requestId": "mastery-a",
            "idempotencyKey": "mastery-idem-a",
            "learnerId": "learner-a",
            "domain": "calyxweb-learner",
            "concepts": [
                {"conceptId": "ward-gate", "mastery": 0.91},
                {"conceptId": "oracle-complete", "trustedMastery": 0.84}
            ],
            "panelBits": 1.2,
            "anchorEntropyBits": 1.0,
            "oracleSelfConsistency": {"flakiness": 0.01, "validity": 0.95},
            "trustGate": {
                "heldOutCount": 3,
                "kernelRecallRatio": 0.97,
                "calibrationError": 0.05,
                "goodhartPassRate": 1.0,
                "recurringMistakes": 0,
                "replayedMistakes": 4
            }
        }),
    );
    assert_eq!(response.status, STATUS_CREATED, "{}", response.body);
    let body: Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(body["completion"]["slots"].as_array().unwrap().len(), 2);
    assert_eq!(body["completion"]["slots"][0]["tag"], "measured");
    assert_eq!(body["completion"]["slots"][1]["tag"], "inferred");
    assert_eq!(body["trust"]["overall"], true);
    assert_eq!(body["trust"]["clientAttested"], true);
    assert_eq!(body["source"]["clientAttested"], true);
    assert_eq!(body["clientAttested"], true);
    assert_eq!(body["measurementProvenance"], "client_attested");
    assert_eq!(
        body["certificationBlockedReason"],
        "client_attested_metrics"
    );
    assert_eq!(body["certificationEligible"], false);
    assert_eq!(body["source"]["assayRows"], 4);

    let assay_store = AssayStore::load_from_vault(&service.vault).unwrap();
    let assay_rows = assay_store.rows();
    assert_eq!(assay_rows.len(), 4);
    assert!(
        assay_rows
            .iter()
            .all(|row| row.estimate.trust == TrustTag::Provisional)
    );
    assert!(assay_rows.iter().all(|row| {
        row.payload.as_ref().is_some_and(|payload| {
            payload["clientAttested"].as_bool() == Some(true)
                && payload["measurementProvenance"].as_str() == Some("client_attested")
        })
    }));

    let rows = service.base_rows();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some("mastery_evidence")
            && row.metadata_value("request_id") == Some("mastery-a")
            && row.metadata_value("client_attested") == Some("true")
            && row.metadata_value("measurement_provenance") == Some("client_attested")
    }));
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some(KIND_MASTERY_ESTIMATE)
            && row.metadata_value("completion_ledger_seq").is_some()
            && row.metadata_value("trust_ledger_seq").is_some()
            && row.metadata_value("certification_eligible") == Some("false")
            && row.metadata_value("client_attested") == Some("true")
            && row.metadata_value("certification_blocked_reason") == Some("client_attested_metrics")
    }));
}

#[test]
fn mastery_estimate_insufficient_panel_fails_closed_without_certifying() {
    let service = service("mastery-insufficient");
    let response = post(
        &service,
        "/v1/mastery/estimate",
        json!({
            "requestId": "mastery-low-signal",
            "learnerId": "learner-a",
            "concepts": [
                {"conceptId": "ward-gate", "mastery": 0.51},
                {"conceptId": "oracle-complete", "trustedMastery": 0.84}
            ],
            "panelBits": 0.2,
            "anchorEntropyBits": 1.0,
            "oracleSelfConsistency": {"flakiness": 0.01, "validity": 0.95},
            "trustGate": {
                "heldOutCount": 3,
                "kernelRecallRatio": 0.97,
                "calibrationError": 0.05,
                "goodhartPassRate": 1.0,
                "recurringMistakes": 0
            }
        }),
    );
    assert_eq!(response.status, STATUS_UNPROCESSABLE, "{}", response.body);
    assert!(response.body.contains("CALYX_ORACLE_INSUFFICIENT"));
    let rows = service.base_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].metadata_value("origin_kind"),
        Some("mastery_evidence")
    );
    assert_eq!(
        service
            .origin_metrics()
            .write_count(KIND_MASTERY_ESTIMATE, "rejected"),
        1
    );
}

#[test]
fn oracle_forecast_predicts_tree_reverse_cause_and_prereq_edge() {
    let service = service("oracle-forecast");
    let response = post(
        &service,
        "/v1/oracle/forecast",
        oracle_forecast_request(1.2, 1.0),
    );
    assert_eq!(response.status, STATUS_CREATED, "{}", response.body);
    let body: Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(body["prediction"]["outcome"], text_anchor("ready"));
    assert!(body["prediction"]["confidence"].as_f64().unwrap() > 0.5);
    assert_eq!(
        body["consequenceTree"]["children"][0]["root"]["action_or_event"],
        "unlock-systems"
    );
    assert_eq!(
        body["selectedConsequence"]["action_or_event"],
        "solve-systems"
    );
    assert_eq!(
        body["reverse"]["causes"][0]["action_or_event"],
        "learn-linear-equations"
    );
    assert_eq!(body["transferEntropy"]["results"][0]["provisional"], false);
    assert!(
        !body["transferEntropy"]["prereqEdges"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(body["source"]["assayRows"], 4);
    assert!(body["source"]["recurrenceRows"].as_u64().unwrap() >= 61);

    let rows = service.base_rows();
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some("oracle_forecast_evidence")
            && row.metadata_value("request_id") == Some("oracle-forecast-a")
    }));
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some("oracle_forecast_recurrence")
            && row.metadata_value("oracle.action") == Some("learn-linear-equations")
    }));
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some(KIND_ORACLE_FORECAST)
            && row.metadata_value("prediction_ledger_seq").is_some()
            && row.metadata_value("reverse_ledger_seq").is_some()
            && row.metadata_value("prereq_edge_count").is_some()
    }));
    assert_eq!(
        service
            .origin_metrics()
            .write_count(KIND_ORACLE_FORECAST, "accepted"),
        1
    );
}

#[test]
fn oracle_forecast_insufficient_panel_fails_closed_without_final_row() {
    let service = service("oracle-forecast-insufficient");
    let response = post(
        &service,
        "/v1/oracle/forecast",
        oracle_forecast_request(0.2, 1.0),
    );
    assert_eq!(response.status, STATUS_UNPROCESSABLE, "{}", response.body);
    assert!(response.body.contains("CALYX_ORACLE_INSUFFICIENT"));
    let rows = service.base_rows();
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some("oracle_forecast_evidence")
            && row.metadata_value("request_id") == Some("oracle-forecast-a")
    }));
    assert!(
        !rows
            .iter()
            .any(|row| row.metadata_value("origin_kind") == Some(KIND_ORACLE_FORECAST))
    );
    assert_eq!(
        service
            .origin_metrics()
            .write_count(KIND_ORACLE_FORECAST, "rejected"),
        1
    );
}

#[test]
fn wrong_learner_outcome_rejected_without_ledger_append() {
    let service = service("wrong-learner");
    let decision = post(
        &service,
        "/v1/interventions/decide",
        json!({
            "decisionId": "decision-owner",
            "learnerId": "learner-a",
            "conceptId": "fractions"
        }),
    );
    assert_eq!(decision.status, STATUS_CREATED, "{}", decision.body);
    let before = service.latest_seq();
    let rejected = post(
        &service,
        "/v1/interventions/decision-owner/outcomes",
        json!({
            "outcomeId": "outcome-wrong",
            "learnerId": "learner-b",
            "outcome": "accepted"
        }),
    );
    assert_eq!(rejected.status, STATUS_FORBIDDEN);
    assert_eq!(service.latest_seq(), before);
    assert_eq!(service.base_rows().len(), 1);
}

#[test]
fn authorization_required() {
    let service = service("auth");
    let response = service.handle(
        "POST",
        "/v1/learner-signals/batches",
        Some("Bearer wrong"),
        br#"{"batchId":"a","learnerId":"l","events":[{}]}"#,
    );
    assert_eq!(response.status, STATUS_UNAUTHORIZED);
    assert!(service.base_rows().is_empty());
}

fn oracle_forecast_request(panel_bits: f32, anchor_entropy_bits: f32) -> Value {
    let mut observations = Vec::new();
    for _ in 0..60 {
        observations.push(json!({
            "actionId": "learn-linear-equations",
            "outcome": text_anchor("ready"),
            "groundTruth": text_anchor("ready"),
            "consequences": [{
                "actionOrEvent": "unlock-systems",
                "outcome": text_anchor("unlock-systems"),
                "grounded": true
            }]
        }));
    }
    observations.push(json!({
        "actionId": "unlock-systems",
        "outcome": text_anchor("linked"),
        "consequences": [{
            "actionOrEvent": "solve-systems",
            "outcome": text_anchor("target-ready"),
            "grounded": true
        }]
    }));

    let mut source_series = Vec::new();
    let mut target_series = Vec::new();
    for t in 0..90_u64 {
        let source = ((t * 37 % 29) as f32 / 29.0) + (t as f32 * 0.0001);
        let previous_source = if t == 0 {
            0.0
        } else {
            (((t - 1) * 37 % 29) as f32 / 29.0) + ((t - 1) as f32 * 0.0001)
        };
        let target = previous_source + ((t * 11 % 7) as f32 * 0.001);
        source_series.push(json!({"t": t, "value": source}));
        target_series.push(json!({"t": t, "value": target}));
    }

    json!({
        "requestId": "oracle-forecast-a",
        "idempotencyKey": "oracle-forecast-idem-a",
        "learnerId": "learner-a",
        "domain": "calyxweb-g2-forecast",
        "actionId": "learn-linear-equations",
        "panelConcepts": ["linear-equations", "systems"],
        "panelBits": panel_bits,
        "anchorEntropyBits": anchor_entropy_bits,
        "observations": observations,
        "desiredOutcome": text_anchor("target-ready"),
        "reverseAnswer": text_anchor("unlock-systems"),
        "transferEntropy": {
            "sourceConceptId": "linear-equations",
            "targetConceptId": "systems",
            "sourceSeries": source_series,
            "targetSeries": target_series,
            "lags": [1],
            "bootstrapResamples": 16,
            "bootstrapSeed": 1240
        }
    })
}

fn text_anchor(value: &str) -> Value {
    json!({"text": value})
}
