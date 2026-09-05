use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::{HolderShare, MakerShare, MakerShareEvidenceSource, MarketSnapshot};
use calyx_poly::pipeline::ingest_snapshot;
use calyx_poly::risk::{
    MARKET_INTEGRITY_HOLDER_CONCENTRATION, MARKET_INTEGRITY_INVALID_EVIDENCE,
    MARKET_INTEGRITY_LOW_HOLDER_DIVERSITY, MARKET_INTEGRITY_MAKER_CONCENTRATION,
    MARKET_INTEGRITY_MISSING_MAKER_EVIDENCE, MARKET_INTEGRITY_OK, MarketIntegrityParams,
    MarketIntegrityScreen, screen_market_integrity,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{
    collect_files, hex, known_healthy_oracle_risk, known_healthy_wash_trade, named_fsv_root,
    reset_dir, write_blake3sums, write_json,
};

const VAULT_SALT: &[u8] = b"poly-issue141-thin-market";
const DOMAIN: &str = "crypto";

#[test]
fn issue141_thin_manipulable_market_screen_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE141_FSV_ROOT", "poly-issue141-thin");
    reset_dir(&root);

    let happy = happy_path_admits_and_persists_decision(&root);
    let low_holder = edge_refuses(
        &root,
        "edge-low-holder-diversity",
        snapshot("edge-low-holder-diversity", &[100.0; 8], &[250.0; 4]),
        MARKET_INTEGRITY_LOW_HOLDER_DIVERSITY,
        "8 holder wallets is below the required 9-holder floor",
    );
    let holder_concentration = edge_refuses(
        &root,
        "edge-holder-concentration",
        snapshot(
            "edge-holder-concentration",
            &[70.0, 10.0, 10.0, 10.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            &[250.0; 4],
        ),
        MARKET_INTEGRITY_HOLDER_CONCENTRATION,
        "one holder dominates the holder HHI",
    );
    let maker_concentration = edge_refuses(
        &root,
        "edge-maker-concentration",
        snapshot(
            "edge-maker-concentration",
            &[100.0; 10],
            &[700.0, 100.0, 100.0, 100.0],
        ),
        MARKET_INTEGRITY_MAKER_CONCENTRATION,
        "one maker address dominates visible resting size",
    );
    let missing_maker = edge_refuses(
        &root,
        "edge-missing-maker",
        snapshot("edge-missing-maker", &[100.0; 10], &[]),
        MARKET_INTEGRITY_MISSING_MAKER_EVIDENCE,
        "maker-address evidence is absent",
    );
    let invalid_evidence = edge_refuses(
        &root,
        "edge-invalid-evidence",
        invalid_holder_snapshot(),
        MARKET_INTEGRITY_INVALID_EVIDENCE,
        "holder evidence contains a zero-size row",
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 141,
        "source_of_truth": [
            "source snapshot JSON files read back from disk",
            "real AsterVault Base CF scalar rows on disk",
            "real AsterVault Ledger CF rows on disk"
        ],
        "params": MarketIntegrityParams::default(),
        "happy_path": happy,
        "edge_cases": {
            "low_holder_diversity": low_holder,
            "holder_concentration": holder_concentration,
            "maker_concentration": maker_concentration,
            "missing_maker_evidence": missing_maker,
            "invalid_evidence": invalid_evidence
        },
        "physical_files": files
    });
    write_json(&root.join("summary.json"), &summary);
    write_blake3sums(&root);

    println!(
        "issue141_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue141_fsv_root={}", root.display());
    }
}

fn happy_path_admits_and_persists_decision(root: &Path) -> Value {
    let fixture = setup_fixture(
        &root.join("happy"),
        snapshot("happy-diverse-market", &[100.0; 10], &[250.0; 4]),
    );
    let before = source_state(&fixture, None);
    let (screen, decision) = decision_from_sources(&fixture);
    assert_eq!(screen.code, MARKET_INTEGRITY_OK);
    assert!(decision.admitted, "reason: {}", decision.reason);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_ADMITTED");
    assert_screen_matches_vault_scalars(&screen, &before);
    let ledger_ref = append_decision_ledger(&fixture, "happy", &decision, &screen);
    fixture.vault.flush().expect("flush happy");
    let after = source_state(&fixture, Some(ledger_ref.seq));
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["market_integrity"]["code"],
        json!(MARKET_INTEGRITY_OK)
    );
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["admitted"],
        json!(true)
    );
    assert_eq!(
        after["ledger_count"].as_u64().expect("after ledger count"),
        before["ledger_count"]
            .as_u64()
            .expect("before ledger count")
            + 1
    );
    let evidence = json!({
        "trigger": "diverse 10-holder and 4-maker market",
        "market_integrity": screen,
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
    expected_code: &str,
    trigger: &str,
) -> Value {
    let fixture = setup_fixture(&root.join(scenario), snapshot);
    let before = source_state(&fixture, None);
    let (screen, decision) = decision_from_sources(&fixture);
    assert_eq!(screen.code, expected_code);
    assert!(!screen.ok);
    assert!(!decision.admitted);
    assert_eq!(decision.code, expected_code);
    fixture.vault.flush().expect("flush edge vault");
    let after = source_state(&fixture, None);
    assert_eq!(
        before, after,
        "refused admission must not mutate vault state"
    );
    let evidence = json!({
        "trigger": trigger,
        "market_integrity": screen,
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

fn decision_from_sources(fixture: &Fixture) -> (MarketIntegrityScreen, AdmissionDecision) {
    let snapshot = fixture.vault.snapshot();
    let _candidate = fixture
        .vault
        .get(fixture.candidate_id, snapshot)
        .expect("read candidate from vault");
    let screen =
        screen_market_integrity(&fixture.source_snapshot, &MarketIntegrityParams::default());
    assert!(
        screen.valid_state(),
        "market integrity screen must be well-formed"
    );
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
        grounding_anchor_count: 0,
        guard_pass: true,
        liquidity_ok: true,
        market_integrity: screen.clone(),
        oracle_risk: known_healthy_oracle_risk(),
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
    screen: &MarketIntegrityScreen,
) -> calyx_core::LedgerRef {
    let payload = json!({
        "issue": 141,
        "scenario": scenario,
        "domain": DOMAIN,
        "admitted": decision.admitted,
        "code": decision.code,
        "reason": decision.reason,
        "market_integrity": screen
    });
    fixture
        .vault
        .append_ledger_entry(
            EntryKind::Admission,
            SubjectId::Cx(fixture.candidate_id),
            serde_json::to_vec(&payload).expect("encode admission payload"),
            ActorId::Service("calyx-poly-issue141".to_string()),
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
            "row_hash": candidate_base
                .as_ref()
                .map(|bytes| blake3::hash(bytes).to_hex().to_string()),
            "decoded_scalars": {
                "best_ask": candidate.scalars.get("best_ask"),
                "holder_count": candidate.scalars.get("holder_count"),
                "holder_herfindahl": candidate.scalars.get("holder_herfindahl"),
                "top_holder_share": candidate.scalars.get("top_holder_share"),
                "maker_count": candidate.scalars.get("maker_count"),
                "maker_herfindahl": candidate.scalars.get("maker_herfindahl"),
                "top_maker_share": candidate.scalars.get("top_maker_share")
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
            payload["issue"] == json!(141)
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

fn assert_screen_matches_vault_scalars(screen: &MarketIntegrityScreen, state: &Value) {
    let scalars = &state["candidate_base"]["decoded_scalars"];
    assert_approx(
        scalars["holder_herfindahl"].as_f64(),
        screen.holder_herfindahl,
    );
    assert_approx(
        scalars["top_holder_share"].as_f64(),
        screen.top_holder_share,
    );
    assert_approx(
        scalars["maker_herfindahl"].as_f64(),
        screen.maker_herfindahl,
    );
    assert_approx(scalars["top_maker_share"].as_f64(), screen.top_maker_share);
    assert_eq!(
        scalars["holder_count"].as_f64().expect("holder_count") as u32,
        screen.holder_count
    );
    assert_eq!(
        scalars["maker_count"].as_f64().expect("maker_count") as u32,
        screen.maker_count
    );
}

fn assert_approx(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("scalar present");
    assert!(
        (actual - expected).abs() < 1.0e-9,
        "expected {expected}, got {actual}"
    );
}

fn snapshot(slug: &str, holder_amounts: &[f64], maker_sizes: &[f64]) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("{slug}-token"),
        condition_id: format!("{slug}-condition"),
        outcome_index: 0,
        slug: slug.to_string(),
        question: Some(format!("Thin market screen {slug}?")),
        event_id: Some(format!("{slug}-event")),
        category: Some(DOMAIN.to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue141".to_string(), "risk".to_string()],
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
        holders: holder_amounts
            .iter()
            .enumerate()
            .map(|(idx, amount)| HolderShare {
                wallet: format!("0xholder{idx:02}"),
                amount: *amount,
                outcome_index: 0,
            })
            .collect(),
        makers: maker_sizes
            .iter()
            .enumerate()
            .map(|(idx, size)| MakerShare {
                maker: format!("0xmaker{idx:02}"),
                size: *size,
                evidence_source: MakerShareEvidenceSource::RestingClobOrderBook,
            })
            .collect(),
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

fn invalid_holder_snapshot() -> MarketSnapshot {
    let mut snapshot = snapshot("edge-invalid-evidence", &[100.0; 10], &[250.0; 4]);
    snapshot.holders[0].amount = 0.0;
    snapshot
}

fn file_state(path: &Path) -> Value {
    let bytes = std::fs::read(path).ok();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len),
        "blake3": bytes
            .as_ref()
            .map(|bytes| hex(blake3::hash(bytes).as_bytes()))
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
    .expect("open issue141 vault")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
