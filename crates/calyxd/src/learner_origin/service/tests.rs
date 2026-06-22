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
