use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::{CounterpartyVolume, MarketSnapshot};
use calyx_poly::pipeline::ingest_snapshot;
use calyx_poly::wash::{
    WASH_TRADE_COUNTERPARTY_CONCENTRATION, WASH_TRADE_DISTINCT_VOLUME_EXCEEDS_RAW,
    WASH_TRADE_INVALID_EVIDENCE, WASH_TRADE_LOW_COUNTERPARTY_DIVERSITY,
    WASH_TRADE_LOW_DISTINCT_VOLUME, WASH_TRADE_MISSING_RAW_VOLUME, WASH_TRADE_OK, WashTradeParams,
    WashTradeScreen, screen_wash_trading,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{
    collect_files, hex, known_healthy_market_integrity, known_healthy_oracle_risk, named_fsv_root,
    reset_dir, write_blake3sums, write_json,
};

const VAULT_SALT: &[u8] = b"poly-issue140-wash-trade";
const DOMAIN: &str = "crypto";

#[test]
fn issue140_wash_trade_distinct_counterparty_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE140_FSV_ROOT", "poly-issue140-wash");
    reset_dir(&root);

    let happy = happy_path_admits_and_persists_decision(&root);
    let low_diversity = edge_refuses(
        &root,
        "edge-low-counterparty-diversity",
        snapshot(
            "edge-low-counterparty-diversity",
            Some(100_000.0),
            &[50_000.0, 50_000.0],
        ),
        WASH_TRADE_LOW_COUNTERPARTY_DIVERSITY,
        "two counterparties cannot prove organic on-chain volume",
    );
    let low_distinct_volume = edge_refuses(
        &root,
        "edge-low-distinct-volume",
        snapshot(
            "edge-low-distinct-volume",
            Some(100_000.0),
            &[20_000.0, 5_000.0, 5_000.0, 5_000.0, 5_000.0],
        ),
        WASH_TRADE_LOW_DISTINCT_VOLUME,
        "distinct-counterparty volume ratio is below the 50% floor",
    );
    let concentrated = edge_refuses(
        &root,
        "edge-counterparty-concentration",
        snapshot(
            "edge-counterparty-concentration",
            Some(100_000.0),
            &[70_000.0, 10_000.0, 10_000.0, 5_000.0, 5_000.0],
        ),
        WASH_TRADE_COUNTERPARTY_CONCENTRATION,
        "one counterparty dominates distinct on-chain volume",
    );
    let exceeds_raw = edge_refuses(
        &root,
        "edge-distinct-volume-exceeds-raw",
        snapshot(
            "edge-distinct-volume-exceeds-raw",
            Some(80_000.0),
            &[20_000.0; 5],
        ),
        WASH_TRADE_DISTINCT_VOLUME_EXCEEDS_RAW,
        "distinct-counterparty volume cannot exceed the raw source volume",
    );
    let missing_raw = edge_refuses(
        &root,
        "edge-missing-raw-volume",
        snapshot("edge-missing-raw-volume", None, &[20_000.0; 5]),
        WASH_TRADE_MISSING_RAW_VOLUME,
        "raw 24h volume is absent",
    );
    let invalid = edge_refuses(
        &root,
        "edge-invalid-counterparty-row",
        invalid_counterparty_snapshot(),
        WASH_TRADE_INVALID_EVIDENCE,
        "counterparty evidence contains a zero-volume row",
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 140,
        "source_of_truth": [
            "source snapshot JSON files read back from disk",
            "real AsterVault Base CF scalar rows on disk",
            "real AsterVault Ledger CF rows on disk"
        ],
        "params": WashTradeParams::default(),
        "happy_path": happy,
        "edge_cases": {
            "low_counterparty_diversity": low_diversity,
            "low_distinct_volume": low_distinct_volume,
            "counterparty_concentration": concentrated,
            "distinct_volume_exceeds_raw": exceeds_raw,
            "missing_raw_volume": missing_raw,
            "invalid_counterparty_row": invalid
        },
        "physical_files": files
    });
    write_json(&root.join("summary.json"), &summary);
    write_blake3sums(&root);

    println!(
        "issue140_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue140_fsv_root={}", root.display());
    }
}

fn happy_path_admits_and_persists_decision(root: &Path) -> Value {
    let fixture = setup_fixture(
        &root.join("happy"),
        snapshot(
            "happy-distinct-counterparties",
            Some(100_000.0),
            &[20_000.0; 5],
        ),
    );
    let before = source_state(&fixture, None);
    let (screen, decision) = decision_from_sources(&fixture);
    assert_eq!(screen.code, WASH_TRADE_OK);
    assert!(decision.admitted, "reason: {}", decision.reason);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_ADMITTED");
    assert_screen_matches_vault_scalars(&screen, &before);
    let ledger_ref = append_decision_ledger(&fixture, "happy", &decision, &screen);
    fixture.vault.flush().expect("flush happy");
    let after = source_state(&fixture, Some(ledger_ref.seq));
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["wash_trade"]["code"],
        json!(WASH_TRADE_OK)
    );
    assert_eq!(
        after["admission_ledger_count"]
            .as_u64()
            .expect("after count"),
        before["admission_ledger_count"]
            .as_u64()
            .expect("before count")
            + 1
    );
    let evidence = json!({
        "trigger": "five distinct counterparties cover all raw volume",
        "wash_trade": screen,
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
        "wash_trade": screen,
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

fn decision_from_sources(fixture: &Fixture) -> (WashTradeScreen, AdmissionDecision) {
    let snapshot = fixture.vault.snapshot();
    let _candidate = fixture
        .vault
        .get(fixture.candidate_id, snapshot)
        .expect("read candidate from vault");
    let screen = screen_wash_trading(&fixture.source_snapshot, &WashTradeParams::default());
    assert!(
        screen.valid_state(),
        "wash-trade screen must be well-formed"
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
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: known_healthy_oracle_risk(),
        wash_trade: screen.clone(),
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
    screen: &WashTradeScreen,
) -> calyx_core::LedgerRef {
    let payload = json!({
        "issue": 140,
        "scenario": scenario,
        "domain": DOMAIN,
        "admitted": decision.admitted,
        "code": decision.code,
        "reason": decision.reason,
        "wash_trade": screen
    });
    fixture
        .vault
        .append_ledger_entry(
            EntryKind::Admission,
            SubjectId::Cx(fixture.candidate_id),
            serde_json::to_vec(&payload).expect("encode admission payload"),
            ActorId::Service("calyx-poly-issue140".to_string()),
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
                "volume_24h": candidate.scalars.get("volume_24h"),
                "distinct_counterparty_count": candidate.scalars.get("distinct_counterparty_count"),
                "distinct_counterparty_volume": candidate.scalars.get("distinct_counterparty_volume"),
                "distinct_counterparty_volume_ratio": candidate.scalars.get("distinct_counterparty_volume_ratio"),
                "top_counterparty_share": candidate.scalars.get("top_counterparty_share")
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
            payload["issue"] == json!(140)
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

fn assert_screen_matches_vault_scalars(screen: &WashTradeScreen, state: &Value) {
    let scalars = &state["candidate_base"]["decoded_scalars"];
    assert_eq!(
        scalars["distinct_counterparty_count"]
            .as_f64()
            .expect("counterparty_count") as u32,
        screen.distinct_counterparty_count
    );
    assert_approx(
        scalars["distinct_counterparty_volume"].as_f64(),
        screen.distinct_counterparty_volume,
    );
    assert_approx(
        scalars["distinct_counterparty_volume_ratio"].as_f64(),
        screen.distinct_counterparty_volume_ratio,
    );
    assert_approx(
        scalars["top_counterparty_share"].as_f64(),
        screen.top_counterparty_share,
    );
}

fn assert_approx(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("scalar present");
    assert!(
        (actual - expected).abs() < 1.0e-9,
        "expected {expected}, got {actual}"
    );
}

fn snapshot(slug: &str, raw_volume: Option<f64>, cp_volumes: &[f64]) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("{slug}-token"),
        condition_id: format!("{slug}-condition"),
        outcome_index: 0,
        slug: slug.to_string(),
        question: Some(format!("Wash trade screen {slug}?")),
        event_id: Some(format!("{slug}-event")),
        category: Some(DOMAIN.to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue140".to_string(), "risk".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(0.61),
        mid: Some(0.61),
        best_bid: Some(0.79),
        best_ask: Some(0.80),
        spread: Some(0.01),
        tick_size: Some(0.01),
        volume_24h: raw_volume,
        liquidity: Some(40_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.02),
        ofi: Some(0.2),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: cp_volumes
            .iter()
            .enumerate()
            .map(|(idx, volume)| CounterpartyVolume {
                counterparty: format!("0xcp{idx:02}"),
                volume: *volume,
            })
            .collect(),
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

fn invalid_counterparty_snapshot() -> MarketSnapshot {
    let mut snapshot = snapshot(
        "edge-invalid-counterparty-row",
        Some(100_000.0),
        &[20_000.0; 5],
    );
    snapshot.counterparty_volumes[0].volume = 0.0;
    snapshot
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
    .expect("open issue140 vault")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
