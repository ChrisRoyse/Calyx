use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::admission::{AdmissionDecision, AdmissionInputs, AdmissionParams};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::MarketSnapshot;
use calyx_poly::pipeline::ingest_snapshot;
use calyx_poly::{
    AgentForecastArtifactRequest, AgentForecastManifest, AgentSourceSnapshotRef, LocalOnlyPolicy,
    PolyAction, PolyError, evaluate_admission, write_agent_forecast_artifacts,
};
use serde_json::{Value, json};

use super::issue096_static::{
    assert_agent_paths_exist, assert_no_trade_keys, file_name, hash_file, prefix,
    provider_metadata, read_manifest, snapshot, vault_id,
};
use super::support::{
    hex, known_healthy_market_integrity, known_healthy_oracle_risk, known_healthy_wash_trade,
    reset_dir, write_json,
};

const VAULT_SALT: &[u8] = b"poly-issue096-provenance";

pub fn setup_fixture(root: &Path) -> Fixture {
    reset_dir(root);
    let vault = AsterVault::new_durable(
        root,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open #96 vault");
    let panel = default_panel(1, vec!["global".to_string()]);
    let snapshot = snapshot();
    let candidate_id =
        ingest_snapshot(&vault, &panel, &snapshot, vault_id(), VAULT_SALT).expect("ingest source");
    vault.flush().expect("flush source");
    Fixture {
        vault,
        snapshot,
        candidate_id,
    }
}

pub fn happy_end_to_end_admits_with_full_provenance(root: &Path, fixture: &Fixture) -> Value {
    let case_dir = root.join("happy");
    reset_dir(&case_dir);
    let source_path = case_dir.join("source-snapshot.json");
    write_json(
        &source_path,
        &serde_json::to_value(&fixture.snapshot).expect("snapshot JSON"),
    );
    let association = write_association_artifact(&case_dir, fixture.candidate_id);
    let policy_decision = LocalOnlyPolicy::default().enforce(PolyAction::WriteForecastArtifact);
    assert!(policy_decision.allowed);

    let artifacts_root = case_dir.join("agent-artifacts");
    let agent_request = agent_request(
        fixture,
        "issue096_happy",
        response_json(0.94, 0.74, happy_rationale()),
    );
    let manifest =
        write_agent_forecast_artifacts(&artifacts_root, &agent_request).expect("agent artifacts");
    let run_dir = artifacts_root.join(&manifest.run_id);
    let manifest_path = run_dir.join("manifest.json");
    assert_eq!(read_manifest(&manifest_path), manifest);
    assert_agent_paths_exist(&run_dir, &manifest);

    let decision = admission_decision(
        manifest.parsed_forecast.probability,
        manifest.parsed_forecast.confidence,
    );
    assert!(decision.admitted, "{}", decision.reason);
    let forecast_json = write_forecast_json(
        &case_dir,
        &manifest,
        &association,
        &policy_decision,
        &decision,
    );
    let forecast_md = case_dir.join("forecast.md");
    fs::write(
        &forecast_md,
        "# Forecast\n\nAdmitted local forecast only. No trading action is produced.\n",
    )
    .expect("write forecast markdown");
    let ledger_ref = append_admission_ledger(
        fixture,
        AdmissionLedgerArtifacts {
            manifest: &manifest,
            manifest_path: &manifest_path,
            association: &association,
            policy: &policy_decision,
            decision: &decision,
            forecast_json: &forecast_json,
            forecast_md: &forecast_md,
        },
    );
    fixture.vault.flush().expect("flush #96 ledger");
    let state = source_state(
        fixture,
        Some(ledger_ref.seq),
        &[
            &source_path,
            &association.path,
            &manifest_path,
            &forecast_json,
            &forecast_md,
        ],
    );
    assert_eq!(
        state["admission_ledger"]["entry"]["kind"],
        json!("admission")
    );
    assert_eq!(
        state["admission_ledger"]["entry"]["payload"]["admitted"],
        json!(true)
    );
    assert_no_trade_keys(
        &serde_json::from_slice::<Value>(&fs::read(&forecast_json).unwrap()).unwrap(),
    );
    json!({
        "source_cx_id": fixture.candidate_id.to_string(),
        "agent_run_id": manifest.run_id,
        "decision": decision,
        "ledger_ref": {"seq": ledger_ref.seq, "hash": hex(&ledger_ref.hash)},
        "artifact_hashes": state["artifact_hashes"],
        "state": state
    })
}

pub fn edge_missing_source_refuses_without_admitted_row(root: &Path, fixture: &Fixture) -> Value {
    edge_agent_refusal(
        root,
        fixture,
        "edge-missing-source",
        "missing source references",
        Vec::new(),
        response_json(0.94, 0.74, happy_rationale()),
        "POLY_AGENT_ARTIFACT_MISSING_SOURCES",
    )
}

pub fn edge_malformed_llm_output_refuses_without_admitted_row(
    root: &Path,
    fixture: &Fixture,
) -> Value {
    edge_agent_refusal(
        root,
        fixture,
        "edge-malformed-llm",
        "malformed LLM JSON",
        source_refs(fixture),
        "{not-json".to_string(),
        "POLY_AGENT_RESPONSE_INVALID_JSON",
    )
}

pub fn edge_forbidden_trading_instruction_refuses_without_admitted_row(
    root: &Path,
    fixture: &Fixture,
) -> Value {
    edge_agent_refusal(
        root,
        fixture,
        "edge-forbidden-trading",
        "forbidden trading instruction",
        source_refs(fixture),
        response_json(
            0.94,
            0.74,
            "Place a bet on YES after reading this forecast.",
        ),
        "POLY_AGENT_RESPONSE_FORBIDDEN_TRADING_INSTRUCTION",
    )
}

fn edge_agent_refusal(
    root: &Path,
    fixture: &Fixture,
    run_id: &str,
    trigger: &str,
    source_refs: Vec<AgentSourceSnapshotRef>,
    raw_response_json: String,
    expected_code: &str,
) -> Value {
    let case_dir = root.join(run_id);
    reset_dir(&case_dir);
    let before_count = ledger_count(fixture);
    let request = AgentForecastArtifactRequest {
        source_snapshot_refs: source_refs,
        ..agent_request(fixture, run_id, raw_response_json)
    };
    let err = write_agent_forecast_artifacts(&case_dir.join("agent-artifacts"), &request)
        .expect_err("edge must fail closed");
    let code = match err {
        PolyError::AgentArtifact { code, .. } => code,
        other => panic!("unexpected error: {other:?}"),
    };
    assert_eq!(code, expected_code);
    assert_eq!(before_count, ledger_count(fixture));
    json!({
        "trigger": trigger,
        "expected_code": expected_code,
        "actual_code": code,
        "ledger_count_before": before_count,
        "ledger_count_after": ledger_count(fixture),
        "admitted_row_written": false
    })
}

fn write_association_artifact(root: &Path, candidate_id: CxId) -> AssociationArtifact {
    let path = root.join("association").join("association.json");
    write_json(
        &path,
        &json!({
            "schema_version": "poly.issue096.association.v1",
            "association_kind": "known_truth_scalar_neighbor",
            "candidate_cx_id": candidate_id.to_string(),
            "source": "local Calyx source snapshot scalars",
            "neighbor_count": 1,
            "score": 0.94
        }),
    );
    AssociationArtifact {
        blake3: hash_file(&path),
        path,
    }
}

fn write_forecast_json(
    root: &Path,
    manifest: &AgentForecastManifest,
    association: &AssociationArtifact,
    policy: &calyx_poly::PolicyDecision,
    decision: &AdmissionDecision,
) -> PathBuf {
    let path = root.join("forecast.json");
    write_json(
        &path,
        &json!({
            "schema_version": "poly.issue096.forecast.v1",
            "agent_run_id": manifest.run_id,
            "association_hash": association.blake3,
            "policy_decision": policy,
            "admission": decision,
            "probability": manifest.parsed_forecast.probability,
            "confidence": manifest.parsed_forecast.confidence
        }),
    );
    path
}

fn append_admission_ledger(
    fixture: &Fixture,
    artifacts: AdmissionLedgerArtifacts<'_>,
) -> calyx_core::LedgerRef {
    let payload = json!({
        "schema_version": "poly.issue096.admission_ledger.v1",
        "admitted": artifacts.decision.admitted,
        "code": artifacts.decision.code,
        "reason": artifacts.decision.reason,
        "source_cx_id": fixture.candidate_id.to_string(),
        "association": {"file": file_name(&artifacts.association.path), "hash_prefix": prefix(&artifacts.association.blake3)},
        "agent": {
            "run_id": artifacts.manifest.run_id,
            "manifest_file": file_name(artifacts.manifest_path),
            "manifest_hash_prefix": prefix(&hash_file(artifacts.manifest_path)),
            "probability": artifacts.manifest.parsed_forecast.probability,
            "confidence": artifacts.manifest.parsed_forecast.confidence
        },
        "parser": {
            "schema_version": artifacts.manifest.schema_version,
            "prompt_template_version": artifacts.manifest.prompt.template_version
        },
        "policy_code": artifacts.policy.code,
        "forecast_files": {
            "json_file": file_name(artifacts.forecast_json),
            "json_hash_prefix": prefix(&hash_file(artifacts.forecast_json)),
            "markdown_file": file_name(artifacts.forecast_md),
            "markdown_hash_prefix": prefix(&hash_file(artifacts.forecast_md))
        }
    });
    fixture
        .vault
        .append_ledger_entry(
            EntryKind::Admission,
            SubjectId::Cx(fixture.candidate_id),
            serde_json::to_vec(&payload).expect("encode #96 admission payload"),
            ActorId::Service("calyx-poly-issue096".to_string()),
        )
        .expect("append #96 admission ledger")
}

struct AdmissionLedgerArtifacts<'a> {
    manifest: &'a AgentForecastManifest,
    manifest_path: &'a Path,
    association: &'a AssociationArtifact,
    policy: &'a calyx_poly::PolicyDecision,
    decision: &'a AdmissionDecision,
    forecast_json: &'a Path,
    forecast_md: &'a Path,
}

fn source_state(fixture: &Fixture, ledger_seq: Option<u64>, artifacts: &[&Path]) -> Value {
    let snapshot = fixture.vault.snapshot();
    let base = fixture
        .vault
        .read_cf_at(
            snapshot,
            ColumnFamily::Base,
            &base_key(fixture.candidate_id),
        )
        .expect("read source base row");
    json!({
        "source_base": {
            "present": base.is_some(),
            "bytes": base.as_ref().map(Vec::len),
            "blake3": base.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string())
        },
        "ledger_count": ledger_count(fixture),
        "admission_ledger": ledger_seq
            .map(|seq| ledger_state(&fixture.vault, snapshot, seq))
            .unwrap_or_else(|| json!({"present": false})),
        "artifact_hashes": artifacts.iter().map(|path| json!({
            "path": path.display().to_string(),
            "exists": path.exists(),
            "blake3": hash_file(path)
        })).collect::<Vec<_>>()
    })
}

fn ledger_state(vault: &AsterVault, snapshot: u64, seq: u64) -> Value {
    let row = vault
        .read_cf_at(snapshot, ColumnFamily::Ledger, &ledger_key(seq))
        .expect("read ledger row")
        .expect("ledger row present");
    let entry = decode_ledger(&row).expect("decode ledger row");
    let payload: Value = serde_json::from_slice(&entry.payload).expect("decode payload");
    json!({
        "present": true,
        "bytes": row.len(),
        "row_hash": blake3::hash(&row).to_hex().to_string(),
        "entry": {
            "seq": entry.seq,
            "kind": entry.kind.as_str(),
            "entry_hash": hex(&entry.entry_hash),
            "payload": payload
        }
    })
}

fn ledger_count(fixture: &Fixture) -> usize {
    fixture
        .vault
        .scan_cf_at(fixture.vault.snapshot(), ColumnFamily::Ledger)
        .expect("scan ledger")
        .len()
}

fn agent_request(
    fixture: &Fixture,
    run_id: &str,
    raw_response_json: String,
) -> AgentForecastArtifactRequest {
    AgentForecastArtifactRequest {
        run_id: run_id.to_string(),
        created_at: "2026-07-05T22:30:00Z".to_string(),
        source_snapshot_refs: source_refs(fixture),
        prompt_template_id: "poly.deepseek.forecast.v1".to_string(),
        prompt_template_version: "2026-07-05".to_string(),
        rendered_prompt: format!(
            "Return strict local-forecast JSON for source {}. No order signing, order submission, bankroll management, or trading.",
            fixture.candidate_id
        ),
        provider: provider_metadata(),
        raw_response_json,
        markdown_prediction: "# Forecast\n\nProbability: 0.94\n\nLocal forecast artifact only.\n"
            .to_string(),
    }
}

fn source_refs(fixture: &Fixture) -> Vec<AgentSourceSnapshotRef> {
    vec![AgentSourceSnapshotRef {
        cx_id: fixture.candidate_id.to_string(),
        role: "candidate_market_snapshot".to_string(),
        snapshot: fixture.vault.snapshot(),
    }]
}

fn response_json(probability: f64, confidence: f64, rationale: &str) -> String {
    json!({
        "probability": probability,
        "confidence": confidence,
        "rationale": rationale,
        "constraints": ["Local forecast artifact only.", "No trading action is allowed."],
        "no_trade_policy_assertion": true
    })
    .to_string()
}

fn happy_rationale() -> &'static str {
    "The local source snapshot and association artifact support a high-confidence forecast."
}

fn admission_decision(probability: f64, confidence: f64) -> AdmissionDecision {
    evaluate_admission(
        &AdmissionParams::default(),
        &AdmissionInputs {
            p_win: probability,
            confidence,
            sufficiency_ok: true,
            evidence_count: 3,
            source_derived_evidence_count: 3,
            stale_evidence_count: 0,
            circular_evidence_count: 0,
            super_intel_pass: true,
            guard_calibrated: true,
            grounding_anchor_count: AdmissionParams::default().min_grounding_anchors,
            guard_pass: true,
            liquidity_ok: true,
            market_integrity: known_healthy_market_integrity(),
            oracle_risk: known_healthy_oracle_risk(),
            wash_trade: known_healthy_wash_trade(),
            kill_switch_active: false,
            daily_error_score: 0.0,
        },
    )
}

pub struct Fixture {
    vault: AsterVault,
    snapshot: MarketSnapshot,
    candidate_id: CxId,
}

struct AssociationArtifact {
    path: PathBuf,
    blake3: String,
}
