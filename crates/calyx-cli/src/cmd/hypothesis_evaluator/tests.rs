use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use calyx_core::CxId;
use calyx_lodestar::{HypothesisEvaluationInput, RetrievedEvidence};

use super::*;

#[test]
fn driver_hits_stub_endpoint_and_persists_known_aggregate_readback() {
    let root = temp_root("happy");
    let input = root.join("input.json");
    let out = root.join("driver.json");
    write_input(&input);
    let (endpoint, handle) = stub_endpoint(4, StubMode::Valid);

    run_hypothesis_evaluator(HypothesisEvaluatorArgs {
        input,
        out: out.clone(),
        endpoint,
        auth_env: None,
        model: "stub-model-v1".to_string(),
        temperatures: vec![20, 80],
        timeout_ms: 2_000,
        preflight: DiscoveryRunPreflightArgs::default(),
    })
    .unwrap();
    assert_eq!(handle.join().unwrap(), 4);

    let artifact: DriverArtifact = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(artifact.inputs[0].evaluator_runs.len(), 4);
    assert!(!artifact.prompt_set_sha256.is_empty());
    let eval = &artifact.report.evaluations[0];
    assert_eq!(eval.run_count, 4);
    assert_eq!(eval.prompt_variant_count, 2);
    assert_eq!(eval.temperature_variant_count, 2);
    assert!((eval.plausible_mean - 0.8).abs() < 0.0001);
    assert!((eval.novelty_mean - 0.6).abs() < 0.0001);
    assert!((eval.testability_mean - 0.7).abs() < 0.0001);
    assert!((eval.falsifiability_mean - 0.5).abs() < 0.0001);
    cleanup(root);
}

#[test]
fn endpoint_down_fails_without_artifact() {
    let root = temp_root("down");
    let input = root.join("input.json");
    let out = root.join("driver.json");
    write_input(&input);
    let port = unused_port();

    let err = run_hypothesis_evaluator(HypothesisEvaluatorArgs {
        input,
        out: out.clone(),
        endpoint: format!("http://127.0.0.1:{port}/judge"),
        auth_env: None,
        model: "stub-model-v1".to_string(),
        temperatures: vec![20],
        timeout_ms: 100,
        preflight: DiscoveryRunPreflightArgs::default(),
    })
    .unwrap_err();

    assert_eq!(
        err.code(),
        "CALYX_HYPOTHESIS_EVALUATOR_ENDPOINT_UNREACHABLE"
    );
    assert!(!out.exists());
    cleanup(root);
}

#[test]
fn malformed_variant_fails_without_artifact() {
    let root = temp_root("malformed");
    let input = root.join("input.json");
    let out = root.join("driver.json");
    write_input(&input);
    let (endpoint, handle) = stub_endpoint(1, StubMode::Malformed);

    let err = run_hypothesis_evaluator(HypothesisEvaluatorArgs {
        input,
        out: out.clone(),
        endpoint,
        auth_env: None,
        model: "stub-model-v1".to_string(),
        temperatures: vec![20],
        timeout_ms: 2_000,
        preflight: DiscoveryRunPreflightArgs::default(),
    })
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_HYPOTHESIS_EVALUATOR_MALFORMED_RESPONSE");
    assert!(err.message().contains("clinical_plausibility_v1"));
    assert_eq!(handle.join().unwrap(), 1);
    assert!(!out.exists());
    cleanup(root);
}

#[test]
fn bogus_citation_fails_without_artifact() {
    let root = temp_root("citation");
    let input = root.join("input.json");
    let out = root.join("driver.json");
    write_input(&input);
    let (endpoint, handle) = stub_endpoint(1, StubMode::BadCitation);

    let err = run_hypothesis_evaluator(HypothesisEvaluatorArgs {
        input,
        out: out.clone(),
        endpoint,
        auth_env: None,
        model: "stub-model-v1".to_string(),
        temperatures: vec![20],
        timeout_ms: 2_000,
        preflight: DiscoveryRunPreflightArgs::default(),
    })
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_HYPOTHESIS_EVALUATOR_BAD_CITATION");
    assert_eq!(handle.join().unwrap(), 1);
    assert!(!out.exists());
    cleanup(root);
}

#[test]
fn https_requires_explicit_auth_env_without_persisting_artifact() {
    let root = temp_root("https-missing-auth");
    let input = root.join("input.json");
    let out = root.join("driver.json");
    write_input(&input);

    let err = run_hypothesis_evaluator(HypothesisEvaluatorArgs {
        input,
        out: out.clone(),
        endpoint: "https://token@example.invalid/v1/chat/completions?api_key=secret".to_string(),
        auth_env: None,
        model: "stub-model-v1".to_string(),
        temperatures: vec![20],
        timeout_ms: 2_000,
        preflight: DiscoveryRunPreflightArgs::default(),
    })
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_HYPOTHESIS_EVALUATOR_AUTH_MISSING");
    assert!(!err.message().contains("secret"));
    assert!(!out.exists());
    cleanup(root);
}

#[test]
fn https_mock_transport_uses_bearer_auth_and_redacts_endpoint() {
    let endpoint = "https://user:pass@example.invalid/v1/chat/completions?api_key=secret";
    let auth = http::auth_for_endpoint_with_lookup_for_test(endpoint, Some("EVAL_TOKEN"), |name| {
        assert_eq!(name, "EVAL_TOKEN");
        Some("secret-token".to_string())
    })
    .unwrap();

    let raw = http::post_https_json_with_sender_for_test(
        endpoint,
        &json!({"request": "body"}),
        Duration::from_millis(25),
        &auth,
        |safe_endpoint, authorization, timeout, body| {
            assert_eq!(safe_endpoint, "https://example.invalid/v1/chat/completions");
            assert_eq!(authorization, "Bearer secret-token");
            assert_eq!(timeout, Duration::from_millis(25));
            assert_eq!(body["request"], "body");
            Ok(openai_body("evidence-1"))
        },
    )
    .unwrap();

    assert_eq!(
        http::artifact_endpoint(endpoint),
        "https://example.invalid/v1/chat/completions"
    );
    assert_eq!(
        raw.pointer("/choices/0/message/content")
            .and_then(Value::as_str),
        Some(valid_body("evidence-1").as_str())
    );
}

#[derive(Clone, Copy)]
enum StubMode {
    Valid,
    Malformed,
    BadCitation,
}

fn stub_endpoint(expected: usize, mode: StubMode) -> (String, thread::JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let mut handled = 0;
        for stream in listener.incoming().take(expected) {
            let mut stream = stream.unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(500)))
                .ok();
            read_http_request(&mut stream);
            let body = match mode {
                StubMode::Valid => valid_body("evidence-1"),
                StubMode::Malformed => "{\"plausible_score\":".to_string(),
                StubMode::BadCitation => valid_body("missing-evidence"),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            handled += 1;
        }
        handled
    });
    (format!("http://{addr}/judge"), handle)
}

fn read_http_request(stream: &mut std::net::TcpStream) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let n = stream.read(&mut chunk).unwrap();
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(header_end) = find_header_end(&buffer) {
            let content_length = content_length(&buffer[..header_end]).unwrap_or(0);
            let body_read = buffer.len().saturating_sub(header_end + 4);
            if body_read >= content_length {
                break;
            }
        }
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(headers: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(headers);
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn valid_body(evidence_id: &str) -> String {
    serde_json::to_string(&json!({
        "plausible_score": 0.8,
        "novelty_score": 0.6,
        "testability_score": 0.7,
        "falsifiability_score": 0.5,
        "justification": "known stub justification",
        "falsification_test": "known stub falsification",
        "cited_evidence_ids": [evidence_id],
    }))
    .unwrap()
}

fn openai_body(evidence_id: &str) -> Value {
    json!({"choices": [{"message": {"content": valid_body(evidence_id)}}]})
}

fn write_input(path: &PathBuf) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let input = InputFile {
        schema_version: Some(1),
        inputs: vec![HypothesisEvaluationInput {
            hypothesis_id: "h1".to_string(),
            a: cx(1),
            b: cx(2),
            c: cx(3),
            claim: "synthetic hypothesis".to_string(),
            grounded_confidence: 0.9,
            chain_provenance: vec!["chain=synthetic".to_string()],
            retrieved_evidence: vec![RetrievedEvidence {
                evidence_id: "evidence-1".to_string(),
                source_cx_id: cx(1),
                title: "Evidence 1".to_string(),
                abstract_text: "Persisted evidence text".to_string(),
                grounding_confidence: 0.9,
                provenance: vec!["source_sha256=sha".to_string()],
            }],
            evaluator_runs: Vec::new(),
        }],
    };
    fs::write(path, serde_json::to_vec_pretty(&input).unwrap()).unwrap();
}

fn unused_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "calyx-hypothesis-evaluator-{name}-{}-{}",
        std::process::id(),
        crate::cmd::vault::now_ms()
    ));
    let _ = fs::remove_dir_all(&root);
    root
}

fn cleanup(path: PathBuf) {
    fs::remove_dir_all(path).unwrap();
}
