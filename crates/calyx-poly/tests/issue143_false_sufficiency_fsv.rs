use std::path::Path;

use calyx_aster::cf::{ColumnFamily, base_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::MarketSnapshot;
use calyx_poly::pipeline::ingest_snapshot;
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{
    collect_files, hex, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_wash_trade, named_fsv_root, reset_dir, write_blake3sums, write_json,
};

const VAULT_SALT: &[u8] = b"poly-issue143-false-sufficiency";
const ACTOR: &str = "calyx-poly-issue143";

#[test]
fn issue143_false_sufficiency_guard_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE143_FSV_ROOT", "poly-issue143-sufficiency");
    reset_dir(&root);

    let happy = happy_path(&root);
    let empty = edge_case(
        &root,
        "edge-empty-evidence",
        |inputs| {
            inputs.evidence_count = 0;
            inputs.source_derived_evidence_count = 0;
        },
        "CALYX_POLY_ADMISSION_MISSING_EVIDENCE",
    );
    let stale = edge_case(
        &root,
        "edge-stale-evidence",
        |inputs| inputs.stale_evidence_count = 1,
        "CALYX_POLY_ADMISSION_STALE_EVIDENCE",
    );
    let circular = edge_case(
        &root,
        "edge-circular-evidence",
        |inputs| inputs.circular_evidence_count = 1,
        "CALYX_POLY_ADMISSION_CIRCULAR_EVIDENCE",
    );
    let low_support = edge_case(
        &root,
        "edge-low-source-support",
        |inputs| inputs.source_derived_evidence_count = 0,
        "CALYX_POLY_ADMISSION_LOW_SOURCE_SUPPORT",
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 143,
            "source_of_truth": {
                "vault_rows": "durable AsterVault Base and Ledger CF rows",
                "evidence_files": root.join("evidence").display().to_string(),
                "admitted_artifacts": root.join("admitted").display().to_string()
            },
            "happy_path": happy,
            "edge_cases": {
                "empty_evidence": empty,
                "stale_evidence": stale,
                "circular_evidence": circular,
                "low_source_support": low_support
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);

    if keep_root {
        println!("poly_issue143_fsv_root={}", root.display());
    }
}

fn happy_path(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("happy"));
    let artifact = root.join("admitted").join("happy-admitted-forecast.json");
    let before = source_state(&fixture, &artifact, None);
    let inputs = good_inputs();
    let decision = evaluate_admission(&AdmissionParams::default(), &inputs);
    assert!(decision.admitted, "reason: {}", decision.reason);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_ADMITTED");
    write_admitted_artifact(&artifact, &decision, &inputs, &fixture);
    let ledger_ref = append_decision_ledger(&fixture, "happy", &decision, &inputs);
    fixture.vault.flush().expect("flush happy");
    let after = source_state(&fixture, &artifact, Some(ledger_ref.seq));

    assert_eq!(after["artifact"]["exists"], json!(true));
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["admitted"],
        json!(true)
    );
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["evidence_state"]["evidence_count"],
        json!(2)
    );
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["evidence_refs"][0]["file"],
        json!("market-source.json")
    );

    let evidence = json!({
        "trigger": "two source-derived evidence files and enough grounded anchors",
        "decision": decision,
        "ledger_ref": {"seq": ledger_ref.seq, "hash": hex(&ledger_ref.hash)},
        "before": before,
        "after": after
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_case<F>(root: &Path, name: &str, mutate: F, expected_code: &str) -> Value
where
    F: FnOnce(&mut AdmissionInputs),
{
    let fixture = setup_fixture(&root.join(name));
    let artifact = root
        .join("admitted")
        .join(format!("{name}-admitted-forecast.json"));
    let before = source_state(&fixture, &artifact, None);
    let mut inputs = good_inputs();
    mutate(&mut inputs);
    let decision = evaluate_admission(&AdmissionParams::default(), &inputs);
    assert!(!decision.admitted);
    assert_eq!(decision.code, expected_code);
    let ledger_ref = append_decision_ledger(&fixture, name, &decision, &inputs);
    fixture.vault.flush().expect("flush edge");
    let after = source_state(&fixture, &artifact, Some(ledger_ref.seq));

    assert_eq!(after["artifact"]["exists"], json!(false));
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["admitted"],
        json!(false)
    );
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["code"],
        json!(expected_code)
    );

    let evidence = json!({
        "trigger": name,
        "expected_code": expected_code,
        "actual_code": decision.code,
        "ledger_ref": {"seq": ledger_ref.seq, "hash": hex(&ledger_ref.hash)},
        "before": before,
        "after": after
    });
    write_json(&root.join(format!("{name}-readback.json")), &evidence);
    evidence
}

fn setup_fixture(root: &Path) -> Fixture {
    reset_dir(root);
    let vault = AsterVault::new_durable(
        root.join("vault"),
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open issue143 vault");
    let panel = default_panel(1, vec!["global".to_string()]);
    let candidate_id = ingest_snapshot(&vault, &panel, &snapshot(), vault_id(), VAULT_SALT)
        .expect("ingest candidate");
    let evidence_dir = root.join("evidence");
    std::fs::create_dir_all(&evidence_dir).expect("create evidence dir");
    let refs = [
        evidence_dir.join("market-source.json"),
        evidence_dir.join("outcome-source.json"),
    ];
    write_json(
        &refs[0],
        &json!({
            "source": "public-read-only-market-snapshot",
            "candidate_cx_id": candidate_id.to_string(),
            "price": 0.64,
            "best_ask": 0.80
        }),
    );
    write_json(
        &refs[1],
        &json!({
            "source": "resolved-outcome-anchor-history",
            "anchor_count": AdmissionParams::default().min_grounding_anchors,
            "available_at_prediction_time": true
        }),
    );
    vault.flush().expect("flush setup");
    Fixture {
        vault,
        candidate_id,
        evidence_refs: refs.iter().map(|path| path.display().to_string()).collect(),
    }
}

fn good_inputs() -> AdmissionInputs {
    AdmissionInputs {
        p_win: 0.94,
        confidence: 0.74,
        sufficiency_ok: true,
        evidence_count: 2,
        source_derived_evidence_count: 2,
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
    }
}

fn write_admitted_artifact(
    path: &Path,
    decision: &AdmissionDecision,
    inputs: &AdmissionInputs,
    fixture: &Fixture,
) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create admitted artifact dir");
    }
    write_json(
        path,
        &json!({
            "admitted": decision.admitted,
            "code": decision.code,
            "candidate_cx_id": fixture.candidate_id.to_string(),
            "evidence_refs": fixture.evidence_refs,
            "evidence_state": evidence_state(inputs)
        }),
    );
}

fn append_decision_ledger(
    fixture: &Fixture,
    scenario: &str,
    decision: &AdmissionDecision,
    inputs: &AdmissionInputs,
) -> calyx_core::LedgerRef {
    let payload = json!({
        "scenario": scenario,
        "admitted": decision.admitted,
        "code": decision.code,
        "reason": decision.reason,
        "evidence_refs": ledger_evidence_refs(&fixture.evidence_refs),
        "evidence_state": evidence_state(inputs)
    });
    fixture
        .vault
        .append_ledger_entry(
            EntryKind::Admission,
            SubjectId::Cx(fixture.candidate_id),
            serde_json::to_vec(&payload).expect("encode decision"),
            ActorId::Service(ACTOR.to_string()),
        )
        .expect("append decision ledger")
}

fn source_state(fixture: &Fixture, artifact: &Path, decision_seq: Option<u64>) -> Value {
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
        "evidence_files": fixture.evidence_refs.iter().map(|path| file_state(Path::new(path))).collect::<Vec<_>>(),
        "artifact": file_state(artifact),
        "ledger_count": ledger_rows.len(),
        "decision_ledger": decision_seq
            .map(|seq| ledger_state(&fixture.vault, snapshot, seq, fixture.candidate_id))
            .unwrap_or_else(|| json!({"present": false}))
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

fn evidence_state(inputs: &AdmissionInputs) -> Value {
    json!({
        "evidence_count": inputs.evidence_count,
        "source_derived_evidence_count": inputs.source_derived_evidence_count,
        "stale_evidence_count": inputs.stale_evidence_count,
        "circular_evidence_count": inputs.circular_evidence_count,
        "grounding_anchor_count": inputs.grounding_anchor_count
    })
}

fn ledger_evidence_refs(paths: &[String]) -> Vec<Value> {
    paths
        .iter()
        .map(|path| {
            let file = Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .expect("evidence path has UTF-8 file name");
            json!({
                "file": file,
                "path_hash": blake3::hash(path.as_bytes()).to_hex().to_string()
            })
        })
        .collect()
}

fn file_state(path: &Path) -> Value {
    let bytes = std::fs::read(path).ok();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len),
        "blake3": bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes()))
    })
}

fn snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "issue143-token".to_string(),
        condition_id: "issue143-condition".to_string(),
        outcome_index: 0,
        slug: "issue143-known-truth-market".to_string(),
        question: Some("Issue 143 false sufficiency known truth?".to_string()),
        event_id: Some("issue143-event".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue143".to_string(), "false-sufficiency".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_786_000_000,
        price: Some(0.64),
        mid: Some(0.64),
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
    evidence_refs: Vec<String>,
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
