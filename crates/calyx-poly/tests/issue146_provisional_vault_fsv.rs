use std::path::Path;

use calyx_aster::cf::{ColumnFamily, anchor_key, base_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CxId, VaultId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::{MarketSnapshot, Resolution};
use calyx_poly::pipeline::{ground_market, ingest_snapshot};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{
    collect_files, hex, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_wash_trade, named_fsv_root, reset_dir, write_blake3sums, write_json,
};

const VAULT_SALT: &[u8] = b"poly-issue146-provisional-vault";
const MIN_ANCHORS: usize = 50;

#[test]
fn issue146_provisional_vault_refusal_fsv() {
    let (root, keep_root) =
        named_fsv_root("POLY_ISSUE146_FSV_ROOT", "poly-issue146-provisional-vault");
    reset_dir(&root);

    let happy = happy_exact_anchor_floor_admits(&root);
    let uncalibrated = edge_uncalibrated_guard_refuses(&root);
    let below_floor = edge_below_anchor_floor_refuses(&root);
    let empty = edge_empty_history_refuses(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 146,
            "source_of_truth": "real AsterVault Base, Anchors, and Ledger CF rows on disk",
            "happy_path": happy,
            "edge_cases": {
                "uncalibrated_guard": uncalibrated,
                "below_anchor_floor": below_floor,
                "empty_history": empty
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);

    if keep_root {
        println!("poly_issue146_fsv_root={}", root.display());
    }
}

fn happy_exact_anchor_floor_admits(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("happy"), MIN_ANCHORS);
    let before = source_state(&fixture, None);
    let decision = decision_from_vault(&fixture, true, true);
    assert!(decision.admitted, "reason: {}", decision.reason);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_ADMITTED");
    let ledger_ref = append_decision_ledger(&fixture, &decision);
    fixture.vault.flush().expect("flush happy vault");
    let after = source_state(&fixture, Some(ledger_ref.seq));
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["admitted"],
        json!(true)
    );
    assert_eq!(
        after["decision_ledger"]["entry"]["payload"]["code"],
        json!("CALYX_POLY_ADMISSION_ADMITTED")
    );
    let evidence = json!({
        "trigger": "50 grounded histories, calibrated guard, guard pass",
        "expected": {
            "min_grounding_anchors": MIN_ANCHORS,
            "admitted": true,
            "decision_ledger_kind": "admission"
        },
        "decision": decision,
        "ledger_ref": {"seq": ledger_ref.seq, "hash": hex(&ledger_ref.hash)},
        "before": before,
        "after": after
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_uncalibrated_guard_refuses(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("edge-uncalibrated"), MIN_ANCHORS);
    let before = source_state(&fixture, None);
    let decision = decision_from_vault(&fixture, false, true);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_UNCALIBRATED_GUARD");
    let after = source_state(&fixture, None);
    assert_eq!(before, after);
    let evidence = json!({
        "trigger": "50 grounded histories but no current guard calibration",
        "decision": decision,
        "before": before,
        "after": after
    });
    write_json(&root.join("edge-uncalibrated-readback.json"), &evidence);
    evidence
}

fn edge_below_anchor_floor_refuses(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("edge-below-floor"), MIN_ANCHORS - 1);
    let before = source_state(&fixture, None);
    let decision = decision_from_vault(&fixture, true, true);
    assert!(!decision.admitted);
    assert_eq!(
        decision.code,
        "CALYX_POLY_ADMISSION_INSUFFICIENT_GROUNDING_ANCHORS"
    );
    let after = source_state(&fixture, None);
    assert_eq!(before, after);
    let evidence = json!({
        "trigger": "49 grounded histories with calibrated guard and guard pass",
        "decision": decision,
        "before": before,
        "after": after
    });
    write_json(&root.join("edge-below-floor-readback.json"), &evidence);
    evidence
}

fn edge_empty_history_refuses(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("edge-empty-history"), 0);
    let before = source_state(&fixture, None);
    let decision = decision_from_vault(&fixture, true, true);
    assert!(!decision.admitted);
    assert_eq!(
        decision.code,
        "CALYX_POLY_ADMISSION_INSUFFICIENT_GROUNDING_ANCHORS"
    );
    let after = source_state(&fixture, None);
    assert_eq!(before, after);
    let evidence = json!({
        "trigger": "0 grounded histories with calibrated guard and guard pass",
        "decision": decision,
        "before": before,
        "after": after
    });
    write_json(&root.join("edge-empty-history-readback.json"), &evidence);
    evidence
}

fn setup_fixture(root: &Path, grounded_histories: usize) -> Fixture {
    reset_dir(root);
    let vault = open_vault(&root.join("vault"));
    let panel = default_panel(1, vec!["global".to_string()]);
    let vault_id = vault_id();
    let mut history_ids = Vec::with_capacity(grounded_histories);
    for idx in 0..grounded_histories {
        let cx_id = ingest_snapshot(
            &vault,
            &panel,
            &snapshot(&format!("history-{idx}"), 0.61, 0.62),
            vault_id,
            VAULT_SALT,
        )
        .expect("ingest history");
        ground_market(&vault, &[cx_id], &resolution(idx), 0).expect("ground history");
        history_ids.push(cx_id);
    }
    let candidate_id = ingest_snapshot(
        &vault,
        &panel,
        &snapshot("candidate", 0.61, 0.80),
        vault_id,
        VAULT_SALT,
    )
    .expect("ingest candidate");
    vault.flush().expect("flush setup");
    Fixture {
        vault,
        history_ids,
        candidate_id,
    }
}

fn decision_from_vault(
    fixture: &Fixture,
    guard_calibrated: bool,
    guard_pass: bool,
) -> AdmissionDecision {
    let snapshot = fixture.vault.snapshot();
    let _candidate = fixture
        .vault
        .get(fixture.candidate_id, snapshot)
        .expect("read candidate");
    let inputs = AdmissionInputs {
        p_win: 0.94,
        confidence: 0.74,
        sufficiency_ok: true,
        evidence_count: 1,
        source_derived_evidence_count: 1,
        stale_evidence_count: 0,
        circular_evidence_count: 0,
        super_intel_pass: true,
        guard_calibrated,
        grounding_anchor_count: grounding_anchor_count(&fixture.vault, &fixture.history_ids),
        guard_pass,
        liquidity_ok: true,
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: known_healthy_oracle_risk(),
        wash_trade: known_healthy_wash_trade(),
        kill_switch_active: false,
        daily_error_score: 0.0,
    };
    evaluate_admission(&AdmissionParams::default(), &inputs)
}

fn append_decision_ledger(
    fixture: &Fixture,
    decision: &AdmissionDecision,
) -> calyx_core::LedgerRef {
    let payload = json!({
        "scenario": "issue146_provisional_vault_refusal",
        "admitted": decision.admitted,
        "code": decision.code,
        "reason": decision.reason,
        "grounding_anchor_count": fixture.history_ids.len(),
    });
    fixture
        .vault
        .append_ledger_entry(
            EntryKind::Admission,
            SubjectId::Cx(fixture.candidate_id),
            serde_json::to_vec(&payload).expect("encode decision payload"),
            ActorId::Service("calyx-poly-issue146".to_string()),
        )
        .expect("append admission decision ledger")
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
    json!({
        "snapshot": snapshot,
        "candidate_base": {
            "present": candidate_base.is_some(),
            "bytes": candidate_base.as_ref().map(Vec::len),
            "row_hash": candidate_base.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string())
        },
        "history_count": fixture.history_ids.len(),
        "grounding_anchor_count": grounding_anchor_count(&fixture.vault, &fixture.history_ids),
        "ledger_count": ledger_rows.len(),
        "decision_ledger": decision_seq
            .map(|seq| ledger_state(&fixture.vault, snapshot, seq, fixture.candidate_id))
            .unwrap_or_else(|| json!({"present": false}))
    })
}

fn grounding_anchor_count(vault: &AsterVault, history_ids: &[CxId]) -> u32 {
    let snapshot = vault.snapshot();
    history_ids
        .iter()
        .filter(|id| {
            vault
                .read_cf_at(
                    snapshot,
                    ColumnFamily::Anchors,
                    &anchor_key(**id, &AnchorKind::TestPass),
                )
                .expect("read anchor")
                .is_some()
        })
        .count() as u32
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

fn snapshot(slug: &str, price: f64, ask: f64) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("{slug}-token"),
        condition_id: format!("{slug}-condition"),
        outcome_index: 0,
        slug: slug.to_string(),
        question: Some(format!("Provisional vault market {slug}?")),
        event_id: Some(format!("{slug}-event")),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue146".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(price),
        mid: Some(price),
        best_bid: Some(price - 0.01),
        best_ask: Some(ask),
        spread: Some(ask - (price - 0.01)),
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

fn resolution(idx: usize) -> Resolution {
    Resolution {
        condition_id: format!("history-{idx}-condition"),
        winning_outcome_index: 0,
        winning_label: "YES".to_string(),
        resolved_ts: 1_785_600_000,
        source: "uma".to_string(),
        disputed: false,
    }
}

struct Fixture {
    vault: AsterVault,
    history_ids: Vec<CxId>,
    candidate_id: CxId,
}

fn open_vault(dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open issue146 vault")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
