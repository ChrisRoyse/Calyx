use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::agent_secrets::{
    POLY_DEEPSEEK_API_KEY_NAME, POLY_DEEPSEEK_BASE_URL, POLY_DEEPSEEK_ENVIRONMENT,
    POLY_DEEPSEEK_MODEL_PRO, POLY_DEEPSEEK_PROJECT_ID, POLY_DEEPSEEK_SECRET_PATH,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::MarketSnapshot;
use calyx_poly::pipeline::ingest_snapshot;
use calyx_poly::{
    AgentForecastArtifactRequest, AgentSourceSnapshotRef, DeepSeekSecretMetadata, PolyError,
    write_agent_forecast_artifacts,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_SALT: &[u8] = b"poly-issue164-agent-artifacts";

#[test]
fn issue164_agent_artifact_schema_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE164_FSV_ROOT", "poly-issue164-agent");
    reset_dir(&root);

    let fixture = setup_fixture(&root.join("vault"));
    let happy = happy_writes_all_artifacts_and_ledger(&root, &fixture);
    let invalid_json = edge_refuses_without_success(
        &root,
        &fixture,
        "edge-invalid-json",
        "raw response is not JSON",
        "{not-json".to_string(),
        "POLY_AGENT_RESPONSE_INVALID_JSON",
    );
    let missing_probability = edge_refuses_without_success(
        &root,
        &fixture,
        "edge-missing-probability",
        "raw response omits probability",
        json!({
            "confidence": 0.61,
            "rationale": "Local-only forecast explanation.",
            "constraints": ["No trading action."],
            "no_trade_policy_assertion": true
        })
        .to_string(),
        "POLY_AGENT_RESPONSE_MISSING_PROBABILITY",
    );
    let probability_out_of_range = edge_refuses_without_success(
        &root,
        &fixture,
        "edge-probability-out-of-range",
        "probability exceeds one",
        response_json(1.20, "Local-only forecast explanation."),
        "POLY_AGENT_RESPONSE_PROBABILITY_OUT_OF_RANGE",
    );
    let confidence_ceiling = edge_refuses_without_success(
        &root,
        &fixture,
        "edge-confidence-ceiling",
        "confidence claims perfect certainty (1.0), exceeding the never-reaches-1 ceiling",
        json!({
            "probability": 0.5,
            "confidence": 1.0,
            "rationale": "Local-only forecast explanation.",
            "constraints": ["No trading action."],
            "no_trade_policy_assertion": true
        })
        .to_string(),
        "POLY_AGENT_RESPONSE_CONFIDENCE_CEILING",
    );
    let forbidden_trading_instruction = edge_refuses_without_success(
        &root,
        &fixture,
        "edge-forbidden-trading-instruction",
        "rationale contains a forbidden trading instruction",
        response_json(0.72, "Place a bet on YES after reading this forecast."),
        "POLY_AGENT_RESPONSE_FORBIDDEN_TRADING_INSTRUCTION",
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 164,
            "source_of_truth": "local forecast-agent artifact files plus real AsterVault Ledger CF row",
            "happy_path": happy,
            "edge_cases": {
                "invalid_json": invalid_json,
                "missing_probability": missing_probability,
                "probability_out_of_range": probability_out_of_range,
                "confidence_ceiling": confidence_ceiling,
                "forbidden_trading_instruction": forbidden_trading_instruction
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);

    if keep_root {
        println!("poly_issue164_fsv_root={}", root.display());
    }
}

fn happy_writes_all_artifacts_and_ledger(root: &Path, fixture: &Fixture) -> Value {
    let artifacts_root = root.join("artifacts");
    let request = request_for(
        fixture,
        "happy_agent_forecast",
        response_json(0.72, happy_rationale()),
    );
    let run_dir = artifacts_root.join(&request.run_id);
    let before = source_state(fixture, &run_dir, None);
    let manifest =
        write_agent_forecast_artifacts(&artifacts_root, &request).expect("write agent artifacts");
    let ledger_ref = append_agent_forecast_ledger(fixture, &manifest);
    fixture.vault.flush().expect("flush agent ledger");
    let after = source_state(fixture, &run_dir, Some(ledger_ref.seq));

    assert_eq!(
        after["manifest"]["readback"]["schema_version"],
        json!("poly.agent.forecast.v1")
    );
    assert_eq!(
        after["manifest"]["readback"]["parsed_forecast"]["probability"],
        json!(0.72)
    );
    assert_eq!(after["ledger"]["entry"]["kind"], json!("agent_forecast"));
    assert_eq!(
        after["ledger"]["entry"]["payload"]["run_id"],
        json!("happy_agent_forecast")
    );
    assert_eq!(
        after["ledger"]["entry"]["payload"]["probability"],
        json!(0.72)
    );

    let evidence = json!({
        "trigger": "write prompt, raw response, parsed forecast JSON, markdown prediction, manifest, and ledger row",
        "ledger_ref": {"seq": ledger_ref.seq, "hash": hex(&ledger_ref.hash)},
        "before": before,
        "after": after
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_refuses_without_success(
    root: &Path,
    fixture: &Fixture,
    run_id: &str,
    trigger: &str,
    raw_response_json: String,
    expected_code: &str,
) -> Value {
    let artifacts_root = root.join("artifacts");
    let request = request_for(fixture, run_id, raw_response_json);
    let run_dir = artifacts_root.join(&request.run_id);
    let before = source_state(fixture, &run_dir, None);
    let error = write_agent_forecast_artifacts(&artifacts_root, &request)
        .expect_err("edge case must fail closed");
    let code = match error {
        PolyError::AgentArtifact { code, .. } => code,
        other => panic!("unexpected error variant: {other:?}"),
    };
    assert_eq!(code, expected_code);
    let after = source_state(fixture, &run_dir, None);
    assert_eq!(before["artifact_dir"], after["artifact_dir"]);
    assert_eq!(before["ledger_count"], after["ledger_count"]);
    assert_eq!(after["ledger"], json!({"present": false}));

    let evidence = json!({
        "trigger": trigger,
        "expected_code": expected_code,
        "actual_code": code,
        "before": before,
        "after": after
    });
    write_json(&root.join(format!("{run_id}-readback.json")), &evidence);
    evidence
}

fn request_for(
    fixture: &Fixture,
    run_id: &str,
    raw_response_json: String,
) -> AgentForecastArtifactRequest {
    AgentForecastArtifactRequest {
        run_id: run_id.to_string(),
        created_at: "2026-07-03T19:30:00Z".to_string(),
        source_snapshot_refs: vec![AgentSourceSnapshotRef {
            cx_id: fixture.candidate_id.to_string(),
            role: "candidate_market_snapshot".to_string(),
            snapshot: fixture.vault.snapshot(),
        }],
        prompt_template_id: "poly.deepseek.forecast.v1".to_string(),
        prompt_template_version: "2026-07-03".to_string(),
        rendered_prompt: rendered_prompt(fixture),
        provider: provider_metadata(),
        raw_response_json,
        markdown_prediction: markdown_prediction(),
    }
}

fn rendered_prompt(fixture: &Fixture) -> String {
    format!(
        "# Poly Forecast Agent\n\nReturn strict JSON with probability, confidence, rationale, constraints, and no_trade_policy_assertion.\n\nSource CxId: {}\n\nPolicy: local forecast only. No order signing, no order submission, no bankroll management.",
        fixture.candidate_id
    )
}

fn response_json(probability: f64, rationale: &str) -> String {
    json!({
        "probability": probability,
        "confidence": 0.61,
        "rationale": rationale,
        "constraints": [
            "Local forecast artifact only.",
            "No trading action is allowed."
        ],
        "no_trade_policy_assertion": true
    })
    .to_string()
}

fn happy_rationale() -> &'static str {
    "The source snapshot shows stable liquidity and a narrow spread; this is a local probability forecast only."
}

fn markdown_prediction() -> String {
    "# Forecast\n\nProbability: 0.72\n\nThis markdown artifact is local analysis only and does not instruct any trading action.\n".to_string()
}

fn provider_metadata() -> DeepSeekSecretMetadata {
    DeepSeekSecretMetadata {
        project_id: POLY_DEEPSEEK_PROJECT_ID.to_string(),
        environment: POLY_DEEPSEEK_ENVIRONMENT.to_string(),
        secret_path: POLY_DEEPSEEK_SECRET_PATH.to_string(),
        api_key_name: POLY_DEEPSEEK_API_KEY_NAME.to_string(),
        key_present: true,
        key_length: 35,
        key_has_sk_prefix: true,
        key_sha256_prefix: "8e7788955344".to_string(),
        base_url: POLY_DEEPSEEK_BASE_URL.to_string(),
        model: POLY_DEEPSEEK_MODEL_PRO.to_string(),
        chat_completions_url: format!("{POLY_DEEPSEEK_BASE_URL}/chat/completions"),
    }
}

fn append_agent_forecast_ledger(
    fixture: &Fixture,
    manifest: &calyx_poly::AgentForecastManifest,
) -> calyx_core::LedgerRef {
    fixture
        .vault
        .append_ledger_entry(
            EntryKind::AgentForecast,
            SubjectId::Cx(fixture.candidate_id),
            serde_json::to_vec(&manifest.provenance_payload())
                .expect("encode manifest provenance payload"),
            ActorId::Service("calyx-poly-issue164".to_string()),
        )
        .expect("append agent forecast ledger")
}

fn source_state(fixture: &Fixture, run_dir: &Path, ledger_seq: Option<u64>) -> Value {
    let snapshot = fixture.vault.snapshot();
    let candidate_base = fixture
        .vault
        .read_cf_at(
            snapshot,
            ColumnFamily::Base,
            &base_key(fixture.candidate_id),
        )
        .expect("read candidate base");
    let ledger_rows = fixture
        .vault
        .scan_cf_at(snapshot, ColumnFamily::Ledger)
        .expect("scan ledger");
    json!({
        "snapshot": snapshot,
        "candidate_base": {
            "present": candidate_base.is_some(),
            "bytes": candidate_base.as_ref().map(Vec::len),
            "row_hash": candidate_base.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string())
        },
        "artifact_dir": dir_state(run_dir),
        "manifest": manifest_state(run_dir),
        "ledger_count": ledger_rows.len(),
        "ledger": ledger_seq
            .map(|seq| ledger_state(&fixture.vault, snapshot, seq, fixture.candidate_id))
            .unwrap_or_else(|| json!({"present": false}))
    })
}

fn dir_state(path: &Path) -> Value {
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "file_count": if path.exists() { count_files(path) } else { 0 }
    })
}

fn manifest_state(run_dir: &Path) -> Value {
    let manifest_path = run_dir.join("manifest.json");
    if !manifest_path.exists() {
        return json!({"present": false});
    }
    let bytes = fs::read(&manifest_path).expect("read manifest");
    let readback: Value = serde_json::from_slice(&bytes).expect("decode manifest");
    let prompt_path = run_dir.join("prompt").join("rendered-prompt.md");
    let response_path = run_dir.join("response").join("raw-response.json");
    let parsed_path = run_dir.join("forecast").join("parsed-forecast.json");
    let markdown_path = run_dir.join("prediction.md");
    json!({
        "present": true,
        "bytes": bytes.len(),
        "blake3": blake3::hash(&bytes).to_hex().to_string(),
        "readback": readback,
        "required_files": {
            "prompt": file_state(&prompt_path),
            "raw_response": file_state(&response_path),
            "parsed_forecast": file_state(&parsed_path),
            "markdown_prediction": file_state(&markdown_path)
        }
    })
}

fn file_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len),
        "blake3": bytes.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string())
    })
}

fn ledger_state(vault: &AsterVault, snapshot: u64, seq: u64, candidate_id: CxId) -> Value {
    let row = vault
        .read_cf_at(snapshot, ColumnFamily::Ledger, &ledger_key(seq))
        .expect("read ledger")
        .expect("ledger row exists");
    let entry = decode_ledger(&row).expect("decode ledger");
    let payload: Value = serde_json::from_slice(&entry.payload).expect("decode payload");
    json!({
        "present": true,
        "bytes": row.len(),
        "row_hash": blake3::hash(&row).to_hex().to_string(),
        "entry": {
            "seq": entry.seq,
            "kind": entry.kind.as_str(),
            "subject_is_candidate": matches!(&entry.subject, SubjectId::Cx(id) if *id == candidate_id),
            "entry_hash": hex(&entry.entry_hash),
            "payload": payload
        }
    })
}

fn count_files(path: &Path) -> usize {
    let mut count = 0;
    for entry in fs::read_dir(path).expect("read artifact dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            count += count_files(&path);
        } else {
            count += 1;
        }
    }
    count
}

fn setup_fixture(root: &Path) -> Fixture {
    reset_dir(root);
    let vault = AsterVault::new_durable(
        root,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open issue164 vault");
    let panel = default_panel(1, vec!["global".to_string()]);
    let candidate_id = ingest_snapshot(
        &vault,
        &panel,
        &snapshot("candidate"),
        vault_id(),
        VAULT_SALT,
    )
    .expect("ingest candidate");
    vault.flush().expect("flush setup");
    Fixture {
        vault,
        candidate_id,
        _root: root.to_path_buf(),
    }
}

fn snapshot(slug: &str) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("{slug}-token"),
        condition_id: format!("{slug}-condition"),
        outcome_index: 0,
        slug: slug.to_string(),
        question: Some(format!("Agent artifact schema {slug}?")),
        event_id: Some(format!("{slug}-event")),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue164".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(0.61),
        mid: Some(0.61),
        best_bid: Some(0.79),
        best_ask: Some(0.80),
        spread: Some(0.01),
        tick_size: Some(0.01),
        volume_24h: Some(125_000.0),
        liquidity: Some(40_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.02),
        ofi: Some(0.2),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

struct Fixture {
    vault: AsterVault,
    candidate_id: CxId,
    _root: PathBuf,
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
