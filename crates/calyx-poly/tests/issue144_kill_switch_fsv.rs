use std::fs;
use std::path::{Path, PathBuf};

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

const VAULT_SALT: &[u8] = b"poly-issue144-kill-switch";
const FORECAST_DAY: &str = "2026-07-03";

#[test]
fn issue144_kill_switch_halts_new_admissions_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE144_FSV_ROOT", "poly-issue144-kill-switch");
    reset_dir(&root);

    let happy = happy_clear_switch_admits(&root);
    let active = edge_active_kill_switch_refuses(&root);
    let error_limit = edge_daily_error_score_at_limit_refuses(&root);
    let error_over = edge_daily_error_score_over_limit_refuses(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 144,
            "source_of_truth": "physical kill-switch JSON file plus real AsterVault Ledger CF rows on disk",
            "happy_path": happy,
            "edge_cases": {
                "active_kill_switch": active,
                "daily_error_score_at_limit": error_limit,
                "daily_error_score_over_limit": error_over
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);

    if keep_root {
        println!("poly_issue144_fsv_root={}", root.display());
    }
}

fn happy_clear_switch_admits(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("happy"), false, &[-2.0]);
    let before = source_state(&fixture, None);
    let decision = decision_from_sources(&fixture);
    assert!(decision.admitted, "reason: {}", decision.reason);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_ADMITTED");
    let ledger_ref = append_decision_ledger(&fixture, &decision);
    fixture.vault.flush().expect("flush happy");
    let after = source_state(&fixture, Some(ledger_ref.seq));
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["admitted"],
        json!(true)
    );
    let evidence = json!({
        "trigger": "kill flag false and daily error below limit",
        "decision": decision,
        "ledger_ref": {"seq": ledger_ref.seq, "hash": hex(&ledger_ref.hash)},
        "before": before,
        "after": after
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_active_kill_switch_refuses(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("edge-active-switch"), true, &[]);
    let before = source_state(&fixture, None);
    let decision = decision_from_sources(&fixture);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_KILL_SWITCH_ACTIVE");
    let after = source_state(&fixture, None);
    assert_eq!(before, after);
    edge_evidence(
        root,
        "edge-active-switch-readback.json",
        "kill flag true",
        decision,
        before,
        after,
    )
}

fn edge_daily_error_score_at_limit_refuses(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("edge-error-at-limit"), false, &[-10.0]);
    let before = source_state(&fixture, None);
    let decision = decision_from_sources(&fixture);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_DAILY_ERROR_LIMIT");
    let after = source_state(&fixture, None);
    assert_eq!(before, after);
    edge_evidence(
        root,
        "edge-error-at-limit-readback.json",
        "daily error equals max_daily_error_score",
        decision,
        before,
        after,
    )
}

fn edge_daily_error_score_over_limit_refuses(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("edge-error-over-limit"), false, &[-12.5]);
    let before = source_state(&fixture, None);
    let decision = decision_from_sources(&fixture);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_DAILY_ERROR_LIMIT");
    let after = source_state(&fixture, None);
    assert_eq!(before, after);
    edge_evidence(
        root,
        "edge-error-over-limit-readback.json",
        "daily error exceeds max_daily_error_score",
        decision,
        before,
        after,
    )
}

fn edge_evidence(
    root: &Path,
    file_name: &str,
    trigger: &str,
    decision: AdmissionDecision,
    before: Value,
    after: Value,
) -> Value {
    let evidence = json!({
        "trigger": trigger,
        "decision": decision,
        "before": before,
        "after": after
    });
    write_json(&root.join(file_name), &evidence);
    evidence
}

fn setup_fixture(root: &Path, kill_active: bool, error_rows: &[f64]) -> Fixture {
    reset_dir(root);
    let kill_file = root.join("kill-switch.json");
    write_json(
        &kill_file,
        &json!({"active": kill_active, "updated_by": "issue144-fsv"}),
    );
    let vault = open_vault(&root.join("vault"));
    let panel = default_panel(1, vec!["global".to_string()]);
    let candidate_id = ingest_snapshot(
        &vault,
        &panel,
        &snapshot("candidate"),
        vault_id(),
        VAULT_SALT,
    )
    .expect("ingest candidate");
    for error_score in error_rows {
        append_error_row(&vault, candidate_id, *error_score);
    }
    vault.flush().expect("flush setup");
    Fixture {
        vault,
        candidate_id,
        kill_file,
    }
}

fn decision_from_sources(fixture: &Fixture) -> AdmissionDecision {
    let _candidate = fixture
        .vault
        .get(fixture.candidate_id, fixture.vault.snapshot())
        .expect("read candidate");
    let inputs = AdmissionInputs {
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
        kill_switch_active: read_kill_switch(&fixture.kill_file),
        daily_error_score: daily_error_score_from_ledger(&fixture.vault),
    };
    evaluate_admission(&AdmissionParams::default(), &inputs)
}

fn append_error_row(vault: &AsterVault, subject: CxId, forecast_error_score: f64) {
    let payload = json!({
        "scenario": "issue144_error_score",
        "forecast_day": FORECAST_DAY,
        "forecast_error_score": forecast_error_score
    });
    vault
        .append_ledger_entry(
            EntryKind::Admission,
            SubjectId::Cx(subject),
            serde_json::to_vec(&payload).expect("encode error_score"),
            ActorId::Service("calyx-poly-issue144".to_string()),
        )
        .expect("append error_score row");
}

fn append_decision_ledger(
    fixture: &Fixture,
    decision: &AdmissionDecision,
) -> calyx_core::LedgerRef {
    let payload = json!({
        "scenario": "issue144_decision",
        "forecast_day": FORECAST_DAY,
        "admitted": decision.admitted,
        "code": decision.code,
        "reason": decision.reason,
        "daily_error_score": daily_error_score_from_ledger(&fixture.vault),
        "kill_switch_active": read_kill_switch(&fixture.kill_file)
    });
    fixture
        .vault
        .append_ledger_entry(
            EntryKind::Admission,
            SubjectId::Cx(fixture.candidate_id),
            serde_json::to_vec(&payload).expect("encode decision"),
            ActorId::Service("calyx-poly-issue144".to_string()),
        )
        .expect("append decision ledger")
}

fn source_state(fixture: &Fixture, decision_seq: Option<u64>) -> Value {
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
    let kill_bytes = fs::read(&fixture.kill_file).expect("read kill file");
    json!({
        "snapshot": snapshot,
        "kill_switch": {
            "active": read_kill_switch(&fixture.kill_file),
            "bytes": kill_bytes.len(),
            "row_hash": blake3::hash(&kill_bytes).to_hex().to_string()
        },
        "daily_error_score": daily_error_score_from_ledger(&fixture.vault),
        "candidate_base": {
            "present": candidate_base.is_some(),
            "bytes": candidate_base.as_ref().map(Vec::len),
            "row_hash": candidate_base.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string())
        },
        "ledger_count": ledger_rows.len(),
        "decision_ledger": decision_seq
            .map(|seq| ledger_state(&fixture.vault, snapshot, seq, fixture.candidate_id))
            .unwrap_or_else(|| json!({"present": false}))
    })
}

fn read_kill_switch(path: &Path) -> bool {
    let value: Value = serde_json::from_slice(&fs::read(path).expect("read kill-switch file"))
        .expect("decode kill-switch JSON");
    value["active"].as_bool().expect("kill-switch active bool")
}

fn daily_error_score_from_ledger(vault: &AsterVault) -> f64 {
    let mut error = 0.0;
    for (_, row) in vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Ledger)
        .expect("scan ledger")
    {
        let entry = decode_ledger(&row).expect("decode ledger row");
        if entry.kind != EntryKind::Admission {
            continue;
        }
        let payload: Value =
            serde_json::from_slice(&entry.payload).expect("decode admission payload");
        if payload["scenario"] != json!("issue144_error_score")
            || payload["forecast_day"] != json!(FORECAST_DAY)
        {
            continue;
        }
        let error_score = payload["forecast_error_score"]
            .as_f64()
            .expect("error_score is f64");
        if error_score < 0.0 {
            error += -error_score;
        }
    }
    error
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

fn snapshot(slug: &str) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("{slug}-token"),
        condition_id: format!("{slug}-condition"),
        outcome_index: 0,
        slug: slug.to_string(),
        question: Some(format!("Kill switch market {slug}?")),
        event_id: Some(format!("{slug}-event")),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue144".to_string()],
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
    kill_file: PathBuf,
}

fn open_vault(dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open issue144 vault")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
