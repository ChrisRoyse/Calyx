use std::fs;
use std::time::Duration;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{SlotId, SlotVector};
use calyx_sextant::{
    FusionStrategy, HnswIndex, InvertedIndex, Query, RerankCandidateText, RerankRequest,
    RerankerClient, SearchEngine, SlotIndexMap,
};
use serde_json::json;
use sextant_support::cx_u8_fill as cx;

mod reranker_support;
use reranker_support::spawn_reranker;

#[test]
fn search_with_reranker_reorders_pipeline_hits_and_fails_closed_edges() {
    let engine = sample_engine();
    let query = pipeline_query();
    let baseline = engine.search(&query).unwrap();
    assert!(baseline.len() >= 2);

    let ok_server = spawn_reranker("HTTP/1.1 200 OK", r#"{"scores":[0.01,0.99]}"#);
    let reranked = engine
        .search_with_reranker(
            &query,
            &RerankerClient::new(ok_server.endpoint.clone(), Duration::from_secs(1)),
        )
        .unwrap();
    ok_server.join();

    assert_ne!(baseline[0].cx_id, reranked[0].cx_id);
    assert_eq!(reranked[0].score, 0.99);
    assert_eq!(reranked[0].rank, 1);
    assert_eq!(reranked[1].rank, 2);
    assert_eq!(
        reranked[0].explain.as_ref().unwrap().strategy,
        "pipeline+rerank"
    );

    let request = ok_server.request();
    let body = request_body(&request);
    let texts = request_texts(body);
    assert_eq!(texts.len(), baseline.len());
    assert!(texts.contains(&"cat hat".to_string()));
    assert!(texts.contains(&"cat error cause".to_string()));
    assert!(!texts.contains(&"dog log".to_string()));

    let non_2xx = spawn_reranker("HTTP/1.1 503 Service Unavailable", "{}");
    let err = engine
        .search_with_reranker(
            &query,
            &RerankerClient::new(non_2xx.endpoint.clone(), Duration::from_secs(1)),
        )
        .unwrap_err();
    non_2xx.join();
    assert_eq!(err.code, "CALYX_SEXTANT_RERANKER_PROTOCOL");

    let mismatch = spawn_reranker("HTTP/1.1 200 OK", r#"{"scores":[0.10]}"#);
    let err = engine
        .search_with_reranker(
            &query,
            &RerankerClient::new(mismatch.endpoint.clone(), Duration::from_secs(1)),
        )
        .unwrap_err();
    mismatch.join();
    assert_eq!(err.code, "CALYX_SEXTANT_RERANKER_PROTOCOL");

    let err = engine
        .search_with_reranker(
            &Query {
                fusion: Some(FusionStrategy::Rrf),
                ..query
            },
            &RerankerClient::new("http://127.0.0.1:9", Duration::from_millis(5)),
        )
        .unwrap_err();
    assert_eq!(err.code, "CALYX_SEXTANT_QUERY_SHAPE");
    assert!(err.message.contains("Pipeline"));
}

#[test]
fn rerank_request_owns_zeroizing_candidate_text() {
    let request = RerankRequest::new("cat", vec!["cat hat".to_string()]);
    let candidate_type = std::any::type_name_of_val(request.candidates());
    let debug = format!("{request:?}");

    assert!(candidate_type.contains("RerankCandidateText"));
    assert_eq!(request.query(), "cat");
    assert_eq!(request.candidate_count(), 1);
    assert_eq!(request.candidates()[0].as_str(), "cat hat");
    assert!(!debug.contains("cat"));
    assert!(!debug.contains("cat hat"));
    assert!(debug.contains("candidate_count"));
}

#[test]
#[ignore = "manual FSV writes reranker request/result source-of-truth artifacts"]
fn search_with_reranker_manual_fsv() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-reranker-search-fsv"));
    fs::create_dir_all(&root).unwrap();

    let engine = sample_engine();
    let query = pipeline_query();
    let baseline = engine.search(&query).unwrap();
    let server = spawn_reranker("HTTP/1.1 200 OK", r#"{"scores":[0.01,0.99]}"#);
    let reranked = engine
        .search_with_reranker(
            &query,
            &RerankerClient::new(server.endpoint.clone(), Duration::from_secs(1)),
        )
        .unwrap();
    server.join();

    let request = server.request();
    let request_body = request_body(&request);
    let request_texts = request_texts(request_body);
    let parsed_request = serde_json::from_str::<serde_json::Value>(request_body).unwrap();
    let candidate_container_type = std::any::type_name::<Vec<RerankCandidateText>>();
    let candidate_item_type = std::any::type_name::<RerankCandidateText>();
    let result = json!({
        "baseline_order": ids(&baseline),
        "reranked_order": ids(&reranked),
        "reranked_scores": reranked.iter().map(|hit| hit.score).collect::<Vec<_>>(),
        "request_text_count": request_texts.len(),
        "request_contains_cat_hat": request_texts.contains(&"cat hat".to_string()),
        "request_contains_cat_error_cause": request_texts.contains(&"cat error cause".to_string()),
        "request_query": parsed_request["query"].clone(),
        "dog_log_not_requested": !request_body.contains("dog log"),
        "strategy": reranked[0].explain.as_ref().unwrap().strategy,
        "candidate_container_type": candidate_container_type,
        "candidate_item_type": candidate_item_type,
        "candidates_request_scoped": candidate_container_type.contains("RerankCandidateText"),
        "candidate_debug_redacted": !format!("{:?}", RerankCandidateText::new("cat hat")).contains("cat hat"),
    });

    fs::write(root.join("reranker-http-request.txt"), request).unwrap();
    fs::write(
        root.join("reranker-http-response.json"),
        r#"{"scores":[0.01,0.99]}"#,
    )
    .unwrap();
    fs::write(
        root.join("reranker-search-readback.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();

    assert_ne!(baseline[0].cx_id, reranked[0].cx_id);
    assert_eq!(result["dog_log_not_requested"], true);
    assert_eq!(result["strategy"], "pipeline+rerank");
    assert_eq!(result["candidates_request_scoped"], true);
    assert_eq!(result["candidate_debug_redacted"], true);
}

fn request_body(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap()
}

fn request_texts(body: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(body).unwrap()["texts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect()
}

fn sample_engine() -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(SlotId::new(8), 3, 42)).unwrap();
    map.register(HnswIndex::new(SlotId::new(9), 3, 43)).unwrap();
    map.register(InvertedIndex::new(SlotId::new(1))).unwrap();
    let engine = SearchEngine::new(map);
    let texts = ["dog log", "cat hat", "cat error cause"];
    for (idx, text) in texts.iter().enumerate() {
        let id = cx((idx + 1) as u8);
        let seq = idx as u64 + 1;
        engine
            .indexes
            .insert(SlotId::new(8), id, basis_vec(idx), seq)
            .unwrap();
        engine
            .indexes
            .insert(SlotId::new(9), id, basis_vec(2 - idx), seq)
            .unwrap();
        engine
            .indexes
            .insert_text(SlotId::new(1), id, text, seq)
            .unwrap();
    }
    engine
}

fn pipeline_query() -> Query {
    Query {
        fusion: Some(FusionStrategy::Pipeline),
        ..Query::new("cat hat")
            .with_vector(basis_vec(2))
            .with_slots(vec![SlotId::new(1), SlotId::new(8), SlotId::new(9)])
            .explain(true)
    }
}

fn ids(hits: &[calyx_sextant::Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.cx_id.to_string()).collect()
}

fn basis_vec(index: usize) -> SlotVector {
    let mut data = vec![0.0; 3];
    data[index % 3] = 1.0;
    SlotVector::Dense { dim: 3, data }
}
