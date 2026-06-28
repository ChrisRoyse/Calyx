use super::*;

#[test]
fn track_spines_builds_scoped_kernels_and_propagates_mastery() {
    let service = service("track-spines");
    let response = post(
        &service,
        "/v1/kernel/track-spines",
        track_spines_request("track-spines-a"),
    );
    assert_eq!(response.status, STATUS_CREATED, "{}", response.body);
    let body: Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(body["accepted"], true);
    assert_eq!(body["source"]["nodeCount"], 8);
    assert_eq!(body["source"]["trackCount"], 2);
    assert_eq!(body["source"]["masteryLabelCount"], 2);
    assert_eq!(body["tracks"].as_array().unwrap().len(), 2);
    assert!(
        body["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|track| track["drilldownCount"].as_u64().unwrap() > 0)
    );
    assert!(
        body["labelPropagation"]["provisionalPositiveCount"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        body["labelPropagation"]["labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |label| label["provisional"] == true && label["confidence"].as_f64().unwrap() > 0.0
            )
    );

    let rows = service.base_rows();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some("track_spines_evidence")
            && row.metadata_value("request_id") == Some("track-spines-a")
    }));
    assert!(rows.iter().any(|row| {
        row.metadata_value("origin_kind") == Some(KIND_TRACK_SPINES)
            && row.metadata_value("request_id") == Some("track-spines-a")
            && row
                .metadata_value("provisional_positive_count")
                .and_then(|value| value.parse::<usize>().ok())
                .is_some_and(|count| count > 0)
    }));

    let duplicate = post(
        &service,
        "/v1/kernel/track-spines",
        track_spines_request("track-spines-a"),
    );
    assert_eq!(duplicate.status, STATUS_OK, "{}", duplicate.body);
    assert_eq!(service.base_rows().len(), 2);
}

#[test]
fn track_spines_rejects_unknown_mastery_concept_without_write() {
    let service = service("track-spines-reject");
    let mut request = track_spines_request("track-spines-bad");
    request["masteryLabels"] = json!([{"conceptId": "not-in-graph", "mastery": 0.5}]);
    let response = post(&service, "/v1/kernel/track-spines", request);
    assert_eq!(response.status, STATUS_BAD_REQUEST, "{}", response.body);
    assert!(response.body.contains("CALYX_ORIGIN_UNKNOWN_CONCEPT"));
    assert!(service.base_rows().is_empty());
}

#[test]
#[ignore = "manual FSV for #1242; set CALYX_ISSUE1242_FSV_ROOT"]
fn issue1242_track_spines_manual_fsv() {
    let root = std::env::var("CALYX_ISSUE1242_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .expect("CALYX_ISSUE1242_FSV_ROOT is required");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let service = LearnerOriginService::open(
        root.join("learner-origin-vault"),
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        b"origin-test-salt".to_vec(),
        "secret-token".to_string(),
        32 * 1024,
    )
    .expect("open service");

    let accepted = post(
        &service,
        "/v1/kernel/track-spines",
        track_spines_request("issue1242-track-spines"),
    );
    assert_eq!(accepted.status, STATUS_CREATED, "{}", accepted.body);
    let accepted_body: Value = serde_json::from_str(&accepted.body).unwrap();
    let rejected = post(
        &service,
        "/v1/kernel/track-spines",
        json!({
            "requestId": "issue1242-track-spines-reject",
            "learnerId": "learner-1242",
            "nodes": [{"conceptId": "solo"}],
            "edges": [],
            "tracks": [],
            "masteryLabels": []
        }),
    );
    assert_eq!(rejected.status, STATUS_BAD_REQUEST, "{}", rejected.body);

    let rows = service.base_rows();
    let row_readback = rows
        .iter()
        .map(|row| {
            json!({
                "cxId": row.cx_id.to_string(),
                "originKind": row.metadata_value("origin_kind"),
                "requestId": row.metadata_value("request_id"),
                "ledgerSeq": row.provenance.seq,
                "ledgerHash": hex(&row.provenance.hash),
                "trackCount": row.metadata_value("track_count"),
                "labelCount": row.metadata_value("label_count"),
                "provisionalPositiveCount": row.metadata_value("provisional_positive_count"),
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    assert!(
        accepted_body["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|track| track["drilldownCount"].as_u64().unwrap() > 0)
    );
    assert!(
        accepted_body["labelPropagation"]["provisionalPositiveCount"]
            .as_u64()
            .unwrap()
            > 0
    );

    let readback = json!({
        "issue": 1242,
        "surface": "hierarchical_per_track_kernels_plus_label_propagation",
        "acceptedStatus": accepted.status,
        "accepted": accepted_body,
        "rejectedStatus": rejected.status,
        "rejected": serde_json::from_str::<Value>(&rejected.body).unwrap(),
        "originRows": row_readback,
        "metrics": {
            "acceptedWrites": service.origin_metrics().write_count(KIND_TRACK_SPINES, "accepted"),
            "rejectedWrites": service.origin_metrics().write_count(KIND_TRACK_SPINES, "rejected"),
            "trackSpineRequests201": service.origin_metrics().request_count("kernel_track_spines", "201"),
            "trackSpineRequests400": service.origin_metrics().request_count("kernel_track_spines", "400"),
        }
    });
    let out = root.join("issue1242-track-spines-fsv.json");
    std::fs::write(&out, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("ISSUE1242_FSV_READBACK={}", out.display());
}

fn track_spines_request(request_id: &str) -> Value {
    json!({
        "requestId": request_id,
        "idempotencyKey": format!("{request_id}-idem"),
        "learnerId": "learner-1242",
        "domain": "calyxweb-track-spines",
        "nodes": [
            {"conceptId": "meaning-store"},
            {"conceptId": "lens-panel"},
            {"conceptId": "assay-bits"},
            {"conceptId": "grounding-kernel"},
            {"conceptId": "ward-guard"},
            {"conceptId": "ledger-provenance"},
            {"conceptId": "mcp-tools"},
            {"conceptId": "memory-app"}
        ],
        "edges": [
            {"fromConceptId": "meaning-store", "toConceptId": "lens-panel"},
            {"fromConceptId": "lens-panel", "toConceptId": "meaning-store"},
            {"fromConceptId": "lens-panel", "toConceptId": "assay-bits"},
            {"fromConceptId": "assay-bits", "toConceptId": "lens-panel"},
            {"fromConceptId": "assay-bits", "toConceptId": "grounding-kernel"},
            {"fromConceptId": "grounding-kernel", "toConceptId": "assay-bits"},
            {"fromConceptId": "grounding-kernel", "toConceptId": "ward-guard"},
            {"fromConceptId": "ward-guard", "toConceptId": "grounding-kernel"},
            {"fromConceptId": "ward-guard", "toConceptId": "ledger-provenance"},
            {"fromConceptId": "ledger-provenance", "toConceptId": "ward-guard"},
            {"fromConceptId": "ledger-provenance", "toConceptId": "mcp-tools"},
            {"fromConceptId": "mcp-tools", "toConceptId": "ledger-provenance"},
            {"fromConceptId": "mcp-tools", "toConceptId": "memory-app"},
            {"fromConceptId": "memory-app", "toConceptId": "mcp-tools"}
        ],
        "tracks": [
            {
                "trackId": "foundations",
                "label": "Foundations",
                "regions": [
                    {
                        "regionId": "measure",
                        "centroidConceptId": "meaning-store",
                        "conceptIds": ["meaning-store", "lens-panel", "assay-bits"]
                    },
                    {
                        "regionId": "compose",
                        "centroidConceptId": "grounding-kernel",
                        "conceptIds": ["grounding-kernel", "ward-guard", "ledger-provenance"]
                    }
                ]
            },
            {
                "trackId": "builder",
                "label": "Builder",
                "regions": [
                    {
                        "regionId": "ship",
                        "centroidConceptId": "mcp-tools",
                        "conceptIds": ["ledger-provenance", "mcp-tools", "memory-app"]
                    }
                ]
            }
        ],
        "masteryLabels": [
            {"conceptId": "meaning-store", "mastery": 0.92},
            {"conceptId": "grounding-kernel", "mastery": 0.72}
        ],
        "params": {
            "maxRegions": 4,
            "drillRadius": 2,
            "minRegionSize": 1,
            "maxIter": 256,
            "tol": 0.0001,
            "decayLambda": 0.5
        }
    })
}
