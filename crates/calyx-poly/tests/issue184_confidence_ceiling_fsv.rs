mod fsv_support;

use std::fs;
use std::path::Path;

use calyx_core::FixedClock;
use calyx_ledger::{DirectoryLedgerStore, LedgerAppender, LedgerCfStore, LedgerEntry, decode};
use calyx_poly::{
    AgentForecastArtifactRequest, AgentSourceSnapshotRef, DeepSeekSecretMetadata,
    ForecastScoreRequest, ForecastSource, PolyError, ResolvedOutcome,
    write_agent_forecast_artifacts, write_forecast_score_artifacts,
};
use serde_json::{Value, json};

use fsv_support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const TEST_TS: u64 = 1_786_401_184;
const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn issue184_confidence_ceiling_fsv() {
    let (root, _env_supplied) =
        named_fsv_root("POLY_ISSUE184_FSV_ROOT", "issue184-confidence-ceiling-fsv");
    reset_dir(&root);

    let score_evidence = score_boundary_fsv(&root.join("score-boundary"));
    let agent_evidence = agent_boundary_fsv(&root.join("agent-boundary"));

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "schema_version": "poly.issue184.confidence_ceiling_fsv.v1",
        "source_of_truth": {
            "score": "physical forecast.json plus physical score ledger rows",
            "agent": "physical parsed-forecast.json plus manifest.json"
        },
        "score_boundary": score_evidence,
        "agent_boundary": agent_evidence,
        "files": files
    });
    write_json(
        &root.join("issue184-confidence-ceiling-fsv-summary.json"),
        &summary,
    );
    write_blake3sums(&root);
}

fn score_boundary_fsv(root: &Path) -> Value {
    reset_dir(root);
    let score_root = root.join("scores");
    let ledger_dir = root.join("ledger");
    let mut ledger = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).expect("open score ledger"),
        FixedClock::new(TEST_TS),
    )
    .expect("open score ledger appender");

    let happy = score_request("score184happy", 0.999_999);
    let happy_dir = score_root.join(&happy.score_id);
    let happy_before = score_state(&ledger_dir, &happy_dir);
    let manifest = write_forecast_score_artifacts(&score_root, &mut ledger, &happy)
        .expect("confidence below 1 should persist");
    assert_eq!(manifest.ledger_ref.seq, 0);
    let happy_after = score_state(&ledger_dir, &happy_dir);
    let forecast_readback = read_json(&happy_dir.join("forecast.json")).expect("read forecast");
    let manifest_readback = read_json(&happy_dir.join("manifest.json")).expect("read manifest");
    assert_eq!(forecast_readback["confidence"], json!(0.999999));
    assert_eq!(happy_after["ledger_rows"], json!(1));

    let exact_one = score_reject_case(
        "score_confidence_exactly_one",
        "CALYX_POLY_SCORE_CONFIDENCE_CEILING",
        &score_root,
        &ledger_dir,
        &mut ledger,
        score_request("score184exactone", 1.0),
    );
    let negative = score_reject_case(
        "score_confidence_negative",
        "CALYX_POLY_SCORE_CONFIDENCE_CEILING",
        &score_root,
        &ledger_dir,
        &mut ledger,
        score_request("score184negative", -0.01),
    );
    let infinite = score_reject_case(
        "score_confidence_infinite",
        "CALYX_POLY_SCORE_CONFIDENCE_CEILING",
        &score_root,
        &ledger_dir,
        &mut ledger,
        score_request("score184infinite", f64::INFINITY),
    );

    let final_entries = read_ledger_entries(&ledger_dir);
    assert_eq!(final_entries.len(), 1);
    json!({
        "happy_path": {
            "trigger": "write score artifacts with confidence just below 1",
            "before": happy_before,
            "after": happy_after,
            "forecast": forecast_readback,
            "manifest": manifest_readback,
            "ledger_payload": ledger_payload(final_entries.first().expect("score ledger row"))
        },
        "edge_cases": [exact_one, negative, infinite],
        "final": {
            "ledger_rows": final_entries.len(),
            "last_ledger_hash": hex(&final_entries.last().expect("last ledger row").entry_hash),
            "score_artifact_dirs": dirs(&score_root)
        }
    })
}

fn agent_boundary_fsv(root: &Path) -> Value {
    reset_dir(root);
    let artifacts_root = root.join("artifacts");

    let happy = agent_request("agent184happy", 0.999_999);
    let happy_dir = artifacts_root.join(&happy.run_id);
    let happy_before = agent_state(&happy_dir);
    let manifest = write_agent_forecast_artifacts(&artifacts_root, &happy)
        .expect("agent confidence below 1 should persist");
    let happy_after = agent_state(&happy_dir);
    let parsed_readback =
        read_json(&happy_dir.join("forecast/parsed-forecast.json")).expect("read parsed forecast");
    let manifest_readback = read_json(&happy_dir.join("manifest.json")).expect("read manifest");
    assert_eq!(manifest.parsed_forecast.confidence, 0.999_999);
    assert_eq!(parsed_readback["confidence"], json!(0.999999));

    let exact_one = agent_reject_case(
        "agent_confidence_exactly_one",
        "POLY_AGENT_RESPONSE_CONFIDENCE_CEILING",
        &artifacts_root,
        agent_request("agent184exactone", 1.0),
    );
    let negative = agent_reject_case(
        "agent_confidence_negative",
        "POLY_AGENT_RESPONSE_CONFIDENCE_CEILING",
        &artifacts_root,
        agent_request("agent184negative", -0.01),
    );
    let missing = agent_reject_case(
        "agent_confidence_missing",
        "POLY_AGENT_RESPONSE_MISSING_CONFIDENCE",
        &artifacts_root,
        agent_request_with_raw(
            "agent184missing",
            json!({
                "probability": 0.61,
                "rationale": "Known-truth local-only analysis with no trading action.",
                "constraints": ["local-only forecast artifact", "no trading action"],
                "no_trade_policy_assertion": true
            })
            .to_string(),
        ),
    );

    json!({
        "happy_path": {
            "trigger": "write agent artifacts with confidence just below 1",
            "before": happy_before,
            "after": happy_after,
            "parsed_forecast": parsed_readback,
            "manifest": manifest_readback
        },
        "edge_cases": [exact_one, negative, missing],
        "final": {
            "artifact_dirs": dirs(&artifacts_root)
        }
    })
}

fn score_request(score_id: &str, confidence: f64) -> ForecastScoreRequest {
    ForecastScoreRequest {
        score_id: score_id.to_string(),
        forecast_id: "forecast184".to_string(),
        forecast_version: 3,
        current_forecast_version: 3,
        market_id: "market184".to_string(),
        outcome_id: "outcome184".to_string(),
        source: ForecastSource::DeepSeekAgent,
        provider: Some("deepseek-v4-pro".to_string()),
        probability: 0.61,
        confidence,
        forecast_ts: TEST_TS - 3_600,
        scored_ts: TEST_TS,
        horizon_secs: 3_600,
        sufficiency_state: "sufficient-known-truth".to_string(),
        previous_probability: Some(0.58),
        forecast_artifact_hash: HASH_A.to_string(),
        outcome: ResolvedOutcome {
            outcome_id: "outcome184".to_string(),
            resolved: true,
            actual_win: true,
            resolved_ts: TEST_TS - 60,
            source: "uma".to_string(),
            version: 1,
        },
        calibration_bin_count: 10,
    }
}

fn agent_request(run_id: &str, confidence: f64) -> AgentForecastArtifactRequest {
    agent_request_with_raw(
        run_id,
        json!({
            "probability": 0.61,
            "confidence": confidence,
            "rationale": "Known-truth local-only analysis with no trading action.",
            "constraints": ["local-only forecast artifact", "no trading action"],
            "no_trade_policy_assertion": true
        })
        .to_string(),
    )
}

fn agent_request_with_raw(run_id: &str, raw_response_json: String) -> AgentForecastArtifactRequest {
    AgentForecastArtifactRequest {
        run_id: run_id.to_string(),
        created_at: "2026-07-04T03:30:00Z".to_string(),
        source_snapshot_refs: vec![AgentSourceSnapshotRef {
            cx_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
            role: "known_truth_fixture".to_string(),
            snapshot: TEST_TS - 120,
        }],
        prompt_template_id: "issue184-confidence-ceiling".to_string(),
        prompt_template_version: "v1".to_string(),
        rendered_prompt: "Return local-only JSON with confidence strictly less than 1.".to_string(),
        provider: provider_metadata(),
        raw_response_json,
        markdown_prediction: "# Local forecast\n\nKnown-truth local-only prediction artifact."
            .to_string(),
    }
}

fn provider_metadata() -> DeepSeekSecretMetadata {
    DeepSeekSecretMetadata {
        project_id: "11b7ea63-6375-43ec-93ed-946505ef683a".to_string(),
        environment: "dev".to_string(),
        secret_path: "/agents/deepseek".to_string(),
        api_key_name: "POLY_DEEPSEEK_API_KEY".to_string(),
        key_present: true,
        key_length: 35,
        key_has_sk_prefix: true,
        key_sha256_prefix: "0123456789ab".to_string(),
        base_url: "https://api.deepseek.com".to_string(),
        model: "deepseek-v4-pro".to_string(),
        chat_completions_url: "https://api.deepseek.com/chat/completions".to_string(),
    }
}

fn score_reject_case(
    name: &str,
    expected_code: &str,
    score_root: &Path,
    ledger_dir: &Path,
    ledger: &mut LedgerAppender<DirectoryLedgerStore, FixedClock>,
    request: ForecastScoreRequest,
) -> Value {
    let dir = score_root.join(&request.score_id);
    let before = score_state(ledger_dir, &dir);
    let err = write_forecast_score_artifacts(score_root, ledger, &request)
        .expect_err("score confidence edge must fail closed");
    let (code, message) = score_error(err);
    assert_eq!(code, expected_code);
    let after = score_state(ledger_dir, &dir);
    assert_eq!(after["artifact_exists"], json!(false));
    assert_eq!(after["ledger_rows"], before["ledger_rows"]);
    json!({
        "name": name,
        "expected_code": expected_code,
        "error_code": code,
        "error_message": message,
        "before": before,
        "after": after
    })
}

fn agent_reject_case(
    name: &str,
    expected_code: &str,
    artifacts_root: &Path,
    request: AgentForecastArtifactRequest,
) -> Value {
    let dir = artifacts_root.join(&request.run_id);
    let before = agent_state(&dir);
    let err = write_agent_forecast_artifacts(artifacts_root, &request)
        .expect_err("agent confidence edge must fail closed");
    let (code, message) = agent_error(err);
    assert_eq!(code, expected_code);
    let after = agent_state(&dir);
    assert_eq!(after["artifact_exists"], json!(false));
    json!({
        "name": name,
        "expected_code": expected_code,
        "error_code": code,
        "error_message": message,
        "before": before,
        "after": after
    })
}

fn score_state(ledger_dir: &Path, artifact_dir: &Path) -> Value {
    json!({
        "ledger_rows": read_ledger_entries(ledger_dir).len(),
        "artifact_dir": artifact_dir.display().to_string(),
        "artifact_exists": artifact_dir.exists(),
        "artifact_file_count": file_count(artifact_dir)
    })
}

fn agent_state(artifact_dir: &Path) -> Value {
    json!({
        "artifact_dir": artifact_dir.display().to_string(),
        "artifact_exists": artifact_dir.exists(),
        "artifact_file_count": file_count(artifact_dir)
    })
}

fn file_count(dir: &Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(path).expect("read artifact dir") {
            let entry = entry.expect("read artifact entry");
            let metadata = entry.metadata().expect("read artifact metadata");
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                count += 1;
            }
        }
    }
    count
}

fn dirs(root: &Path) -> Vec<String> {
    if !root.exists() {
        return Vec::new();
    }
    let mut dirs: Vec<String> = fs::read_dir(root)
        .expect("read root")
        .filter_map(|entry| {
            let entry = entry.expect("read root entry");
            entry
                .file_type()
                .expect("read file type")
                .is_dir()
                .then(|| entry.file_name().to_string_lossy().to_string())
        })
        .filter(|name| !name.starts_with('.'))
        .collect();
    dirs.sort();
    dirs
}

fn read_ledger_entries(ledger_dir: &Path) -> Vec<LedgerEntry> {
    let store = DirectoryLedgerStore::open(ledger_dir).expect("open ledger for readback");
    store
        .scan()
        .expect("scan physical ledger rows")
        .into_iter()
        .map(|row| decode(&row.bytes).expect("decode physical ledger row"))
        .collect()
}

fn ledger_payload(entry: &LedgerEntry) -> Value {
    serde_json::from_slice(&entry.payload).expect("decode ledger payload")
}

fn read_json(path: &Path) -> Option<Value> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn score_error(err: PolyError) -> (String, String) {
    match err {
        PolyError::Score { code, message } => (code, message),
        other => panic!("expected score error, got {other:?}"),
    }
}

fn agent_error(err: PolyError) -> (String, String) {
    match err {
        PolyError::AgentArtifact { code, message } => (code, message),
        other => panic!("expected agent artifact error, got {other:?}"),
    }
}
