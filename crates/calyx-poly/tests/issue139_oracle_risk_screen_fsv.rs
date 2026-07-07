use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::{MarketSnapshot, OracleRiskEvidence};
use calyx_poly::oracle::{
    ORACLE_RISK_ACTIVE_DISPUTE, ORACLE_RISK_ELEVATED_DISPUTE, ORACLE_RISK_LIVENESS_WINDOW,
    ORACLE_RISK_MISSING_UMA_EVIDENCE, ORACLE_RISK_NEAR_CERTAIN_PRICE, ORACLE_RISK_OK,
    OracleRiskParams, OracleRiskScreen, screen_oracle_risk,
};
use calyx_poly::pipeline::ingest_snapshot;
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{
    collect_files, hex, known_healthy_market_integrity, known_healthy_wash_trade, named_fsv_root,
    reset_dir, write_blake3sums, write_json,
};

const VAULT_SALT: &[u8] = b"poly-issue139-oracle-risk";
const DOMAIN: &str = "crypto";
const RAW_P_WIN: f64 = 0.94;

#[test]
fn issue139_oracle_risk_screen_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE139_FSV_ROOT", "poly-issue139-oracle");
    reset_dir(&root);

    let happy = happy_path_admits_with_adjusted_probability(&root);
    let active_dispute = edge_refuses(
        &root,
        "edge-active-dispute",
        snapshot("edge-active-dispute", 0.80, evidence(0.02, true, 0.0)),
        true,
        ORACLE_RISK_ACTIVE_DISPUTE,
        "UMA active dispute must block local forecast admission",
    );
    let liveness = edge_refuses(
        &root,
        "edge-liveness-window",
        snapshot("edge-liveness-window", 0.80, evidence(0.02, false, 3_600.0)),
        true,
        ORACLE_RISK_LIVENESS_WINDOW,
        "optimistic-oracle challenge window is still open",
    );
    let elevated = edge_refuses(
        &root,
        "edge-elevated-dispute-risk",
        snapshot(
            "edge-elevated-dispute-risk",
            0.80,
            evidence(0.35, false, 0.0),
        ),
        true,
        ORACLE_RISK_ELEVATED_DISPUTE,
        "dispute risk score exceeds the configured threshold",
    );
    let near_certain = edge_refuses(
        &root,
        "edge-near-certain-price",
        snapshot("edge-near-certain-price", 0.995, evidence(0.02, false, 0.0)),
        true,
        ORACLE_RISK_NEAR_CERTAIN_PRICE,
        "near-99c price is blocked before oracle finality",
    );
    let missing_uma = edge_refuses(
        &root,
        "edge-missing-uma-evidence",
        snapshot(
            "edge-missing-uma-evidence",
            0.80,
            OracleRiskEvidence::default(),
        ),
        true,
        ORACLE_RISK_MISSING_UMA_EVIDENCE,
        "missing UMA evidence fails closed",
    );
    let not_adjusted = edge_refuses(
        &root,
        "edge-probability-not-adjusted",
        snapshot(
            "edge-probability-not-adjusted",
            0.80,
            evidence(0.02, false, 0.0),
        ),
        false,
        "CALYX_POLY_ADMISSION_ORACLE_RISK_PROBABILITY_NOT_ADJUSTED",
        "caller used raw p_win instead of the oracle-adjusted p_win",
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 139,
        "source_of_truth": [
            "source snapshot JSON files read back from disk",
            "real AsterVault Base CF oracle scalar rows on disk",
            "real AsterVault Ledger CF rows on disk"
        ],
        "params": OracleRiskParams::default(),
        "happy_path": happy,
        "edge_cases": {
            "active_dispute": active_dispute,
            "liveness_window": liveness,
            "elevated_dispute_risk": elevated,
            "near_certain_price": near_certain,
            "missing_uma_evidence": missing_uma,
            "probability_not_adjusted": not_adjusted
        },
        "physical_files": files
    });
    write_json(&root.join("summary.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue139_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue139_fsv_root={}", root.display());
    }
}

fn happy_path_admits_with_adjusted_probability(root: &Path) -> Value {
    let fixture = setup_fixture(
        &root.join("happy"),
        snapshot("happy-oracle-risk-priced", 0.80, evidence(0.02, false, 0.0)),
    );
    let before = source_state(&fixture, None);
    let (screen, decision) = decision_from_sources(&fixture, true);
    assert_eq!(screen.code, ORACLE_RISK_OK);
    assert_eq!(screen.raw_p_win, RAW_P_WIN);
    assert_approx(Some(screen.p_win_adjusted), 0.92);
    assert!(decision.admitted, "reason: {}", decision.reason);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_ADMITTED");
    assert_screen_matches_vault_scalars(&screen, &before);
    let ledger_ref = append_decision_ledger(&fixture, "happy", &decision, &screen);
    fixture.vault.flush().expect("flush happy");
    let after = source_state(&fixture, Some(ledger_ref.seq));
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["oracle_risk"]["code"],
        json!(ORACLE_RISK_OK)
    );
    assert_eq!(
        after["admission_ledger_count"].as_u64().expect("after"),
        before["admission_ledger_count"].as_u64().expect("before") + 1
    );
    let evidence = json!({
        "trigger": "UMA evidence has low dispute risk and closed liveness; p_win is haircutted from 0.94 to 0.92",
        "oracle_risk": screen,
        "decision": decision,
        "ledger_ref": {"seq": ledger_ref.seq, "hash": hex(&ledger_ref.hash)},
        "before": before,
        "after": after
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_refuses(
    root: &Path,
    scenario: &str,
    snapshot: MarketSnapshot,
    use_adjusted_probability: bool,
    expected_code: &str,
    trigger: &str,
) -> Value {
    let fixture = setup_fixture(&root.join(scenario), snapshot);
    let before = source_state(&fixture, None);
    let (screen, decision) = decision_from_sources(&fixture, use_adjusted_probability);
    assert!(!decision.admitted);
    assert_eq!(decision.code, expected_code);
    if expected_code.starts_with("CALYX_POLY_ADMISSION_ORACLE_RISK_")
        && expected_code != "CALYX_POLY_ADMISSION_ORACLE_RISK_PROBABILITY_NOT_ADJUSTED"
    {
        assert_eq!(screen.code, expected_code);
    }
    fixture.vault.flush().expect("flush edge vault");
    let after = source_state(&fixture, None);
    assert_eq!(before, after, "refusal must not mutate vault state");
    let evidence = json!({
        "trigger": trigger,
        "oracle_risk": screen,
        "decision": decision,
        "before": before,
        "after": after
    });
    write_json(&root.join(format!("{scenario}-readback.json")), &evidence);
    evidence
}

fn setup_fixture(root: &Path, snapshot: MarketSnapshot) -> Fixture {
    reset_dir(root);
    let source_path = root.join("source-snapshot.json");
    write_json(
        &source_path,
        &serde_json::to_value(&snapshot).expect("encode source snapshot"),
    );
    let source_snapshot: MarketSnapshot =
        serde_json::from_slice(&std::fs::read(&source_path).expect("read source snapshot"))
            .expect("decode source snapshot");
    let vault = open_vault(&root.join("vault"));
    let panel = default_panel(1, vec!["global".to_string()]);
    let candidate_id = ingest_snapshot(&vault, &panel, &source_snapshot, vault_id(), VAULT_SALT)
        .expect("ingest source snapshot");
    vault.flush().expect("flush setup");
    Fixture {
        vault,
        candidate_id,
        source_path,
        source_snapshot,
    }
}

fn decision_from_sources(
    fixture: &Fixture,
    use_adjusted_probability: bool,
) -> (OracleRiskScreen, AdmissionDecision) {
    let snapshot = fixture.vault.snapshot();
    let _candidate = fixture
        .vault
        .get(fixture.candidate_id, snapshot)
        .expect("read candidate from vault");
    let screen = screen_oracle_risk(
        &fixture.source_snapshot,
        RAW_P_WIN,
        &OracleRiskParams::default(),
    );
    assert!(
        screen.valid_state(),
        "oracle-risk screen must be well-formed"
    );
    let p_win = if use_adjusted_probability {
        screen.p_win_adjusted
    } else {
        screen.raw_p_win
    };
    let inputs = AdmissionInputs {
        p_win,
        confidence: 0.74,
        sufficiency_ok: true,
        evidence_count: 2,
        source_derived_evidence_count: 2,
        stale_evidence_count: 0,
        circular_evidence_count: 0,
        super_intel_pass: true,
        guard_calibrated: true,
        grounding_anchor_count: 0,
        guard_pass: true,
        liquidity_ok: true,
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: screen.clone(),
        wash_trade: known_healthy_wash_trade(),
        kill_switch_active: false,
        daily_error_score: 0.0,
    };
    let params = AdmissionParams {
        min_grounding_anchors: 0,
        ..AdmissionParams::default()
    };
    (screen, evaluate_admission(&params, &inputs))
}

fn append_decision_ledger(
    fixture: &Fixture,
    scenario: &str,
    decision: &AdmissionDecision,
    screen: &OracleRiskScreen,
) -> calyx_core::LedgerRef {
    let payload = json!({
        "issue": 139,
        "scenario": scenario,
        "domain": DOMAIN,
        "admitted": decision.admitted,
        "code": decision.code,
        "reason": decision.reason,
        "oracle_risk": screen
    });
    fixture
        .vault
        .append_ledger_entry(
            EntryKind::Admission,
            SubjectId::Cx(fixture.candidate_id),
            serde_json::to_vec(&payload).expect("encode admission payload"),
            ActorId::Service("calyx-poly-issue139".to_string()),
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
    let candidate = fixture
        .vault
        .get(fixture.candidate_id, snapshot)
        .expect("get candidate");
    let ledger_rows = fixture
        .vault
        .scan_cf_at(snapshot, ColumnFamily::Ledger)
        .expect("scan ledger");
    json!({
        "snapshot": snapshot,
        "source_file": file_state(&fixture.source_path),
        "candidate_base": {
            "present": candidate_base.is_some(),
            "bytes": candidate_base.as_ref().map(Vec::len),
            "row_hash": candidate_base.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string()),
            "decoded_scalars": {
                "best_ask": candidate.scalars.get("best_ask"),
                "oracle_dispute_risk": candidate.scalars.get("oracle_dispute_risk"),
                "oracle_active_dispute": candidate.scalars.get("oracle_active_dispute"),
                "oracle_liveness_seconds_remaining": candidate.scalars.get("oracle_liveness_seconds_remaining")
            }
        },
        "ledger_count": ledger_rows.len(),
        "admission_ledger_count": admission_ledger_count(&fixture.vault),
        "decision_ledger": decision_seq
            .map(|seq| ledger_state(&fixture.vault, snapshot, seq, fixture.candidate_id))
            .unwrap_or_else(|| json!({"present": false}))
    })
}

fn admission_ledger_count(vault: &AsterVault) -> usize {
    vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Ledger)
        .expect("scan ledger")
        .into_iter()
        .filter(|(_, row)| {
            let entry = decode_ledger(row).expect("decode ledger row");
            if entry.kind != EntryKind::Admission {
                return false;
            }
            let payload: Value =
                serde_json::from_slice(&entry.payload).expect("decode admission payload");
            payload["issue"] == json!(139)
        })
        .count()
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

fn assert_screen_matches_vault_scalars(screen: &OracleRiskScreen, state: &Value) {
    let scalars = &state["candidate_base"]["decoded_scalars"];
    assert_approx(scalars["oracle_dispute_risk"].as_f64(), screen.dispute_risk);
    assert_approx(
        scalars["oracle_liveness_seconds_remaining"].as_f64(),
        screen.liveness_seconds_remaining,
    );
    assert_eq!(
        scalars["oracle_active_dispute"].as_f64().expect("active") as u32,
        u32::from(screen.active_dispute)
    );
}

fn assert_approx(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("scalar present");
    assert!(
        (actual - expected).abs() < 1.0e-9,
        "expected {expected}, got {actual}"
    );
}

fn snapshot(slug: &str, market_price: f64, oracle_risk: OracleRiskEvidence) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("{slug}-token"),
        condition_id: format!("{slug}-condition"),
        outcome_index: 0,
        slug: slug.to_string(),
        question: Some(format!("Oracle risk screen {slug}?")),
        event_id: Some(format!("{slug}-event")),
        category: Some(DOMAIN.to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue139".to_string(), "risk".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(market_price),
        mid: Some(market_price),
        best_bid: Some((market_price - 0.01).max(0.0)),
        best_ask: Some(market_price),
        spread: Some(0.01),
        tick_size: Some(0.01),
        volume_24h: Some(100_000.0),
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
        oracle_risk,
        book: Default::default(),
    }
}

fn evidence(
    dispute_risk: f64,
    active_dispute: bool,
    liveness_seconds_remaining: f64,
) -> OracleRiskEvidence {
    OracleRiskEvidence {
        oracle: "uma".to_string(),
        dispute_risk,
        active_dispute,
        liveness_seconds_remaining,
    }
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

struct Fixture {
    vault: AsterVault,
    candidate_id: CxId,
    source_path: PathBuf,
    source_snapshot: MarketSnapshot,
}

fn open_vault(dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open issue139 vault")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
