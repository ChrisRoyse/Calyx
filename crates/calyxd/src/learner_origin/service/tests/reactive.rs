use super::*;

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
