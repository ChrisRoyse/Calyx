use serde_json::{Value, json};

use calyx_aster::cf::ColumnFamily;

use super::*;

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
    assert_eq!(body["certificationEligible"], true);
    assert_eq!(body["source"]["assayRows"], 4);

    let rows = service.base_rows();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some("mastery_evidence")
            && row.metadata_value("request_id") == Some("mastery-a")
    }));
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some(KIND_MASTERY_ESTIMATE)
            && row.metadata_value("completion_ledger_seq").is_some()
            && row.metadata_value("trust_ledger_seq").is_some()
            && row.metadata_value("certification_eligible") == Some("true")
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
fn reactive_affect_fires_new_region_drift_and_mmd_interventions() {
    let service = service("reactive-affect");
    let response = post(
        &service,
        "/v1/reactive/affect-signals",
        reactive_affect_request(true),
    );
    assert_eq!(response.status, STATUS_CREATED, "{}", response.body);
    let body: Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(body["novelty"]["action"], "new_region");
    assert!(
        !body["reactive"]["newRegionEvents"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !body["reactive"]["driftEvents"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(body["mmd"]["drift"]["significant"], true);
    assert!(
        body["interventions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["reason"] == "new_region")
    );

    let rows = service.base_rows();
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some("reactive_affect_evidence")
            && row.metadata_value("request_id") == Some("reactive-affect-a")
    }));
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some(KIND_REACTIVE_AFFECT)
            && row.metadata_value("new_region_event_count") == Some("1")
            && row.metadata_value("drift_event_count") == Some("1")
            && row.metadata_value("recurrence_kind") == Some("reactive_affect_recurrence")
            && row
                .metadata_value("intervention_reasons")
                .is_some_and(|value| value.contains("mmd_distribution_shift"))
    }));
    assert!(
        !service
            .vault
            .scan_cf_at(service.latest_seq(), ColumnFamily::Reactive)
            .expect("scan reactive")
            .is_empty()
    );
    assert!(
        !service
            .vault
            .scan_cf_at(service.latest_seq(), ColumnFamily::Recurrence)
            .expect("scan recurrence")
            .is_empty()
    );
}

#[test]
fn reactive_affect_known_pattern_without_shift_fails_closed_without_final_row() {
    let service = service("reactive-affect-quiet");
    let response = post(
        &service,
        "/v1/reactive/affect-signals",
        reactive_affect_request(false),
    );
    assert_eq!(response.status, STATUS_UNPROCESSABLE, "{}", response.body);
    assert!(response.body.contains("CALYX_ORIGIN_REACTIVE_NO_TRIGGER"));
    let rows = service.base_rows();
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some("reactive_affect_evidence")
            && row.metadata_value("request_id") == Some("reactive-affect-a")
    }));
    assert!(
        !rows
            .iter()
            .any(|row| row.metadata_value("origin_kind") == Some(KIND_REACTIVE_AFFECT))
    );
    assert_eq!(
        service
            .origin_metrics()
            .write_count(KIND_REACTIVE_AFFECT, "rejected"),
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

fn reactive_affect_request(fire: bool) -> Value {
    let (current_vector, occurrences, now_millis, recent_shift) = if fire {
        (vec![0.0, 1.0], vec![1_000_u64], 1_060_000_u64, 3.0)
    } else {
        (
            vec![1.0, 0.0],
            vec![1_000_u64, 1_060_u64, 1_120_u64],
            1_180_000_u64,
            0.0,
        )
    };
    let baseline_samples: Vec<Value> = (0..8).map(|index| json!([index as f64 * 0.1])).collect();
    let recent_samples: Vec<Value> = (0..8)
        .map(|index| json!([recent_shift + index as f64 * 0.1]))
        .collect();
    json!({
        "requestId": "reactive-affect-a",
        "idempotencyKey": "reactive-affect-idem-a",
        "learnerId": "learner-a",
        "domain": "calyxweb-g6-affect",
        "conceptId": "linear-equations",
        "slotId": 13,
        "matchedVector": [1.0, 0.0],
        "baselineVector": [1.0, 0.0],
        "currentVector": current_vector,
        "tau": 0.8,
        "driftThreshold": 0.2,
        "recurrence": {
            "currentOccurrencesSecs": occurrences,
            "knownPatternFrequency": 20
        },
        "mmd": {
            "baselineSamples": baseline_samples,
            "recentSamples": recent_samples,
            "minWindow": 8,
            "bandwidth": 1.0,
            "permutations": 19,
            "seed": 1244,
            "alpha": 0.2
        },
        "nowMillis": now_millis
    })
}

fn text_anchor(value: &str) -> Value {
    json!({"text": value})
}
