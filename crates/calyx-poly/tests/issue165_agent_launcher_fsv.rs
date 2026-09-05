// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=fsv_support visibility=private
use crate::__calyx_shared_fsv_support_rs as fsv_support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, FixedClock, VaultId, VaultStore};
use calyx_ledger::{DirectoryLedgerStore, LedgerAppender, LedgerCfStore, LedgerEntry, decode};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::MarketSnapshot;
use calyx_poly::pipeline::ingest_snapshot;
use calyx_poly::{
    AgentEvidenceSnapshot, AgentLauncherRequest, AgentSourceSnapshotRef, DeepSeekRuntimeSecrets,
    LocalOnlyPolicy, PolyAction, PolyError, launch_deepseek_forecast_agent,
};
use serde_json::{Value, json};

use fsv_support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const TEST_TS: u64 = 1_786_400_165;
const VAULT_SALT: &[u8] = b"poly-issue165-agent-launcher";

#[test]
#[ignore = "requires infisical run with real Poly DeepSeek secrets"]
fn issue165_real_deepseek_agent_launcher_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE165_FSV_ROOT", "issue165-agent-launcher");
    reset_dir(&root);
    let fixture = setup_fixture(&root.join("vault"));
    let secrets = DeepSeekRuntimeSecrets::from_env().expect("load real Infisical DeepSeek secrets");
    let policy = LocalOnlyPolicy::default();

    let happy = happy_path(&root, &fixture, &secrets, &policy);
    let empty_evidence = edge_case(
        &root,
        "edge-empty-evidence",
        &fixture,
        &secrets,
        &policy,
        |request| request.evidence.clear(),
        "POLY_AGENT_LAUNCH_EMPTY_EVIDENCE",
    );
    let stale_evidence = edge_case(
        &root,
        "edge-stale-evidence",
        &fixture,
        &secrets,
        &policy,
        |request| request.evidence[0].expires_ts = TEST_TS,
        "POLY_AGENT_LAUNCH_STALE_EVIDENCE",
    );
    let forbidden_action = edge_case(
        &root,
        "edge-forbidden-action",
        &fixture,
        &secrets,
        &policy,
        |request| request.requested_actions.push(PolyAction::SubmitOrder),
        "CALYX_POLY_POLICY_ORDER_SUBMISSION_FORBIDDEN",
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("issue165-agent-launcher-fsv-summary.json"),
        &json!({
            "issue": 165,
            "source_of_truth": {
                "vault": root.join("vault").display().to_string(),
                "agent_artifacts": root.join("artifacts").display().to_string(),
                "ledger_dirs": root.join("ledgers").display().to_string()
            },
            "happy_path": happy,
            "edge_cases": {
                "empty_evidence": empty_evidence,
                "stale_evidence": stale_evidence,
                "forbidden_action": forbidden_action
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);
    if keep_root {
        println!("poly_issue165_fsv_root={}", root.display());
    }
}

fn happy_path(
    root: &Path,
    fixture: &Fixture,
    secrets: &DeepSeekRuntimeSecrets,
    policy: &LocalOnlyPolicy,
) -> Value {
    let artifact_root = root.join("artifacts").join("happy");
    let ledger_dir = root.join("ledgers").join("happy");
    let mut ledger = ledger_appender(&ledger_dir);
    let request = request_for(fixture, "issue165happy");
    let run_dir = artifact_root.join(&request.run_id);
    let before = state_snapshot(fixture, &ledger_dir, &run_dir);
    let receipt =
        launch_deepseek_forecast_agent(&artifact_root, &mut ledger, policy, secrets, &request)
            .expect("real DeepSeek agent launch should persist");
    let after = state_snapshot(fixture, &ledger_dir, &run_dir);

    assert!(after["artifact_dir"]["exists"].as_bool().unwrap());
    assert_eq!(after["ledger_rows"].as_u64(), Some(3));
    assert_eq!(
        after["manifest"]["readback"]["parsed_forecast"]["probability"],
        json!(0.64)
    );
    assert_eq!(after["ledger"]["last"]["kind"], json!("agent_forecast"));
    assert_eq!(
        after["ledger"]["last"]["payload"]["manifest"]["run_id"],
        json!("issue165happy")
    );
    assert_eq!(
        after["ledger"]["last"]["payload"]["manifest"]["probability"],
        json!(0.64)
    );

    let evidence = json!({
        "trigger": "launch real DeepSeek forecast agent from Calyx-stored synthetic known-truth evidence",
        "receipt": {
            "schema_version": receipt.schema_version,
            "run_id": receipt.run_id,
            "policy_ledger_ref_count": receipt.policy_ledger_refs.len(),
            "forecast_ledger_ref": {
                "seq": receipt.forecast_ledger_ref.seq,
                "hash": hex(&receipt.forecast_ledger_ref.hash)
            },
            "provider_response_id": receipt.provider_response_id,
            "provider_finish_reason": receipt.provider_finish_reason,
            "provider_usage": receipt.provider_usage
        },
        "before": before,
        "after": after
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_case<F>(
    root: &Path,
    name: &str,
    fixture: &Fixture,
    secrets: &DeepSeekRuntimeSecrets,
    policy: &LocalOnlyPolicy,
    mutate: F,
    expected_code: &str,
) -> Value
where
    F: FnOnce(&mut AgentLauncherRequest),
{
    let artifact_root = root.join("artifacts").join(name);
    let ledger_dir = root.join("ledgers").join(name);
    let mut ledger = ledger_appender(&ledger_dir);
    let mut request = request_for(fixture, name);
    mutate(&mut request);
    let run_dir = artifact_root.join(&request.run_id);
    let before = state_snapshot(fixture, &ledger_dir, &run_dir);
    let error =
        launch_deepseek_forecast_agent(&artifact_root, &mut ledger, policy, secrets, &request)
            .expect_err("edge case must fail closed");
    let code = error_code(error);
    assert_eq!(code, expected_code);
    let after = state_snapshot(fixture, &ledger_dir, &run_dir);
    assert!(!after["artifact_dir"]["exists"].as_bool().unwrap());
    if name != "edge-forbidden-action" {
        assert_eq!(after["ledger_rows"], before["ledger_rows"]);
    }

    let evidence = json!({
        "trigger": name,
        "expected_code": expected_code,
        "actual_code": code,
        "before": before,
        "after": after
    });
    write_json(&root.join(format!("{name}-readback.json")), &evidence);
    evidence
}

fn request_for(fixture: &Fixture, run_id: &str) -> AgentLauncherRequest {
    let source = AgentSourceSnapshotRef {
        cx_id: fixture.candidate_id.to_string(),
        role: "candidate_market_snapshot".to_string(),
        snapshot: fixture.vault.snapshot(),
    };
    AgentLauncherRequest {
        run_id: run_id.to_string(),
        created_at: "2026-07-03T20:30:00Z".to_string(),
        run_ts: TEST_TS,
        market_id: "market165".to_string(),
        outcome_id: "outcome165_yes".to_string(),
        question: "For the synthetic known-truth FSV market, what is the local probability of YES?"
            .to_string(),
        source_snapshot_refs: vec![source.clone()],
        evidence: vec![AgentEvidenceSnapshot {
            source,
            title: "synthetic known-truth forecast evidence".to_string(),
            observed_ts: TEST_TS - 60,
            expires_ts: TEST_TS + 3_600,
            content: "This is a deterministic FSV task. The expected JSON forecast is probability 0.64 and confidence 0.74. Rationale must say it copied the known-truth evidence for a local-only forecast. Constraints must include local-only forecast artifact and no trading action."
                .to_string(),
        }],
        requested_actions: vec![
            PolyAction::LaunchForecastAgent,
            PolyAction::WriteForecastArtifact,
        ],
        prompt_template_id: "poly.deepseek.launcher.v1".to_string(),
        prompt_template_version: "2026-07-03".to_string(),
        max_tokens: 512,
        timeout_secs: 60,
    }
}

fn state_snapshot(fixture: &Fixture, ledger_dir: &Path, run_dir: &Path) -> Value {
    let snapshot = fixture.vault.snapshot();
    let base_row = fixture
        .vault
        .read_cf_at(
            snapshot,
            ColumnFamily::Base,
            &base_key(fixture.candidate_id),
        )
        .expect("read candidate base row");
    let entries = read_ledger_entries(ledger_dir);
    json!({
        "vault_snapshot": snapshot,
        "candidate_base": {
            "present": base_row.is_some(),
            "bytes": base_row.as_ref().map(Vec::len),
            "row_hash": base_row.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string())
        },
        "artifact_dir": dir_state(run_dir),
        "manifest": manifest_state(run_dir),
        "ledger_rows": entries.len(),
        "ledger": ledger_state(entries)
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
    let path = run_dir.join("manifest.json");
    if !path.exists() {
        return json!({"present": false});
    }
    let bytes = fs::read(&path).expect("read manifest");
    let readback: Value = serde_json::from_slice(&bytes).expect("decode manifest");
    json!({
        "present": true,
        "bytes": bytes.len(),
        "blake3": blake3::hash(&bytes).to_hex().to_string(),
        "readback": readback,
        "required_files": {
            "prompt": file_state(&run_dir.join("prompt").join("rendered-prompt.md")),
            "raw_response": file_state(&run_dir.join("response").join("raw-response.json")),
            "parsed_forecast": file_state(&run_dir.join("forecast").join("parsed-forecast.json")),
            "markdown_prediction": file_state(&run_dir.join("prediction.md"))
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

fn ledger_state(entries: Vec<LedgerEntry>) -> Value {
    let rows = entries
        .iter()
        .map(|entry| {
            json!({
                "seq": entry.seq,
                "kind": entry.kind.as_str(),
                "entry_hash": hex(&entry.entry_hash),
                "payload": serde_json::from_slice::<Value>(&entry.payload).ok()
            })
        })
        .collect::<Vec<_>>();
    let last = rows
        .last()
        .cloned()
        .unwrap_or_else(|| json!({"present": false}));
    json!({"rows": rows, "last": last})
}

fn read_ledger_entries(ledger_dir: &Path) -> Vec<LedgerEntry> {
    if !ledger_dir.exists() {
        return Vec::new();
    }
    let store = DirectoryLedgerStore::open(ledger_dir).expect("open ledger for readback");
    store
        .scan()
        .expect("scan physical ledger rows")
        .into_iter()
        .map(|row| decode(&row.bytes).expect("decode physical ledger row"))
        .collect()
}

fn setup_fixture(root: &Path) -> Fixture {
    reset_dir(root);
    let vault = AsterVault::new_durable(
        root,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open issue165 vault");
    let panel = default_panel(1, vec!["global".to_string()]);
    let candidate_id = ingest_snapshot(&vault, &panel, &snapshot(), vault_id(), VAULT_SALT)
        .expect("ingest candidate evidence snapshot");
    vault.flush().expect("flush candidate evidence");
    Fixture {
        vault,
        candidate_id,
        _root: root.to_path_buf(),
    }
}

fn snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "issue165-token".to_string(),
        condition_id: "issue165-condition".to_string(),
        outcome_index: 0,
        slug: "issue165-known-truth-market".to_string(),
        question: Some("Issue 165 known truth market?".to_string()),
        event_id: Some("issue165-event".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue165".to_string(), "known-truth".to_string()],
        resolution_source: Some("synthetic-fsv".to_string()),
        neg_risk: false,
        snapshot_ts: TEST_TS - 60,
        price: Some(0.64),
        mid: Some(0.64),
        best_bid: Some(0.63),
        best_ask: Some(0.65),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(50_000.0),
        liquidity: Some(25_000.0),
        one_hour_change: Some(0.0),
        one_day_change: Some(0.01),
        ofi: Some(0.1),
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

fn ledger_appender(ledger_dir: &Path) -> LedgerAppender<DirectoryLedgerStore, FixedClock> {
    LedgerAppender::open(
        DirectoryLedgerStore::open(ledger_dir).expect("open launcher ledger"),
        FixedClock::new(TEST_TS),
    )
    .expect("open launcher ledger appender")
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

fn error_code(error: PolyError) -> String {
    match error {
        PolyError::AgentLaunch { code, .. }
        | PolyError::AgentArtifact { code, .. }
        | PolyError::Policy { code, .. } => code,
        other => panic!("unexpected error variant: {other:?}"),
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
