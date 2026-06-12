use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::{
    FusionStrategy, HnswIndex, InvertedIndex, Query, RerankerClient, SearchEngine, SlotIndexMap,
};
use serde_json::json;

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
#[ignore = "aiwonder FSV writes PH25 Pipeline recall headroom source-of-truth artifacts"]
fn pipeline_recall_headroom_aiwonder_fsv() {
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

struct TestServer {
    endpoint: String,
    request: Arc<Mutex<String>>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl TestServer {
    fn request(&self) -> String {
        self.request.lock().unwrap().clone()
    }

    fn join(&self) {
        if let Some(handle) = self.handle.lock().unwrap().take() {
            handle.join().unwrap();
        }
    }
}

fn spawn_reranker(status: &'static str, body: &'static str) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let request = Arc::new(Mutex::new(String::new()));
    let request_for_thread = Arc::clone(&request);
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0_u8; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    bytes.extend_from_slice(&chunk[..read]);
                    if http_request_complete(&bytes) {
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => panic!("read reranker request: {error}"),
            }
        }
        *request_for_thread.lock().unwrap() = String::from_utf8(bytes).unwrap();
        let response = format!(
            "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    TestServer {
        endpoint,
        request,
        handle: Mutex::new(Some(handle)),
    }
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

fn http_request_complete(bytes: &[u8]) -> bool {
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_len = headers
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    bytes.len() >= header_end + 4 + content_len
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

fn cx(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}
