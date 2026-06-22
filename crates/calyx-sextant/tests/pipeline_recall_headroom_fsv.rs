use std::fs;
use std::time::Duration;

use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::{
    FusionStrategy, HnswIndex, InvertedIndex, Query, RerankerClient, SearchEngine, SlotIndexMap,
};
use serde_json::json;

mod reranker_support;
#[path = "sextant_support/mod.rs"]
mod sextant_support;
use reranker_support::spawn_reranker;
use sextant_support::cx_u8_fill as cx;

#[test]
fn pipeline_recall_k_headroom_recovers_dense_candidate() {
    let engine = sample_engine();
    let sparse_top1 = sparse_ids(&engine, 1);
    let sparse_recall3 = sparse_ids(&engine, 3);

    let narrow = engine.search(&pipeline_query(1)).unwrap();
    let wide = engine.search(&pipeline_query(3)).unwrap();

    assert_eq!(sparse_top1, vec![cx(1)]);
    assert!(sparse_recall3.contains(&cx(2)));
    assert_eq!(narrow[0].cx_id, cx(1));
    assert_eq!(wide[0].cx_id, cx(2));
    assert_eq!(wide.len(), 1);
}

#[test]
#[ignore = "manual FSV writes PH25 Pipeline recall headroom source-of-truth artifacts"]
fn pipeline_recall_headroom_manual_fsv() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-pipeline-recall-headroom-fsv"));
    fs::create_dir_all(&root).unwrap();

    let engine = sample_engine();
    let sparse_top1 = sparse_ids(&engine, 1);
    let sparse_recall3 = sparse_ids(&engine, 3);
    let narrow = engine.search(&pipeline_query(1)).unwrap();
    let wide = engine.search(&pipeline_query(3)).unwrap();

    let server = spawn_reranker("HTTP/1.1 200 OK", r#"{"scores":[1.0,0.5,0.25]}"#);
    let reranked = engine
        .search_with_reranker(
            &pipeline_query(3),
            &RerankerClient::new(server.endpoint.clone(), Duration::from_secs(1)),
        )
        .unwrap();
    server.join();
    let request = server.request();
    let request_texts = request_texts(request_body(&request));

    let readback = json!({
        "query_k": 1,
        "narrow_recall_k": 1,
        "wide_recall_k": 3,
        "sparse_top1": ids(&sparse_top1),
        "sparse_recall3": ids(&sparse_recall3),
        "narrow_top": narrow[0].cx_id.to_string(),
        "wide_top": wide[0].cx_id.to_string(),
        "wide_final_len": wide.len(),
        "recovered_outside_sparse_top_k": !sparse_top1.contains(&wide[0].cx_id)
            && sparse_recall3.contains(&wide[0].cx_id),
        "reranker_request_text_count": request_texts.len(),
        "reranker_request_contains_recovery": request_texts.contains(&"alpha recovery".to_string()),
        "reranked_top": reranked[0].cx_id.to_string(),
        "reranked_final_len": reranked.len(),
    });

    fs::write(root.join("reranker-http-request.txt"), request).unwrap();
    fs::write(
        root.join("pipeline-recall-headroom-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    println!(
        "pipeline_recall_headroom_readback={}",
        root.join("pipeline-recall-headroom-readback.json")
            .display()
    );
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["wide_final_len"], 1);
    assert_eq!(readback["recovered_outside_sparse_top_k"], true);
    assert_eq!(readback["reranker_request_text_count"], 3);
    assert_eq!(readback["reranker_request_contains_recovery"], true);
    assert_eq!(readback["reranked_final_len"], 1);
}

fn sample_engine() -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(InvertedIndex::new(SlotId::new(1))).unwrap();
    map.register(HnswIndex::new(SlotId::new(8), 3, 42)).unwrap();
    let engine = SearchEngine::new(map);
    let rows = [
        (cx(1), "alpha alpha alpha", basis_vec(0)),
        (cx(2), "alpha recovery", basis_vec(2)),
        (cx(3), "alpha neutral", basis_vec(1)),
    ];
    for (seq, (id, text, vector)) in rows.into_iter().enumerate() {
        engine
            .indexes
            .insert_text(SlotId::new(1), id, text, seq as u64 + 1)
            .unwrap();
        engine
            .indexes
            .insert(SlotId::new(8), id, vector, seq as u64 + 1)
            .unwrap();
    }
    engine
}

fn pipeline_query(recall_k: usize) -> Query {
    Query {
        k: 1,
        fusion: Some(FusionStrategy::Pipeline),
        ..Query::new("alpha")
            .with_vector(basis_vec(2))
            .with_slots(vec![SlotId::new(1), SlotId::new(8)])
            .with_recall_k(recall_k)
            .explain(true)
    }
}

fn sparse_ids(engine: &SearchEngine, k: usize) -> Vec<CxId> {
    engine
        .indexes
        .search_text(SlotId::new(1), "alpha", k)
        .unwrap()
        .into_iter()
        .map(|hit| hit.cx_id)
        .collect()
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

fn ids(ids: &[CxId]) -> Vec<String> {
    ids.iter().map(ToString::to_string).collect()
}

fn basis_vec(index: usize) -> SlotVector {
    let mut data = vec![0.0; 3];
    data[index % 3] = 1.0;
    SlotVector::Dense { dim: 3, data }
}
