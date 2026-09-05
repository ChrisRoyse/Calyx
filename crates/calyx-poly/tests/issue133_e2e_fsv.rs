use calyx_aster::cf::{ColumnFamily, anchor_key, base_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{AnchorKind, CxId, VaultId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::admission::{AdmissionInputs, AdmissionParams, evaluate_admission};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::{MarketSnapshot, Resolution};
use calyx_poly::pipeline::{ground_market, ingest_snapshot};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
mod issue133_support;
// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use issue133_support::{Prediction, ScenarioError, outcome_anchor, scalar};
use support::{
    collect_files, hex, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_wash_trade, named_fsv_root, reset_dir, write_blake3sums, write_json,
};
const VAULT_SALT: &[u8] = b"poly-issue133-e2e";
#[test]
fn issue133_e2e_known_truth_to_admission_decision_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE133_FSV_ROOT", "poly-issue133-e2e");
    reset_dir(&root);
    let happy = happy_path_known_truth_admits(&root);
    let no_assoc = edge_no_grounded_associations_fails_closed(&root);
    let guard_refusal = edge_guard_refusal_writes_no_decision_ledger(&root);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 133,
            "source_of_truth": "real AsterVault Base, Anchors, and Ledger CF rows plus durable vault files",
            "happy_path": happy,
            "edge_cases": {
                "no_grounded_associations": no_assoc,
                "guard_refusal": guard_refusal
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);
    if keep_root {
        println!("poly_issue133_fsv_root={}", root.display());
    }
}
fn happy_path_known_truth_admits(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("happy"), true, 0.80);
    let before = source_state(&fixture, None);
    let prediction = associate_and_predict(&fixture).expect("known pattern predicts");
    assert!((0.93..=0.95).contains(&prediction.p_model));
    let decision = admission_decision(&fixture, prediction.p_model, true);
    assert!(decision.admitted, "reason: {}", decision.reason);
    let ledger_ref = append_decision_ledger(&fixture, prediction.p_model, &decision);
    fixture.vault.flush().expect("flush happy vault");
    let after = source_state(&fixture, Some(ledger_ref.seq));
    let decision_entry = &after["decision_ledger"]["entry"];
    assert_eq!(decision_entry["kind"], json!("admission"));
    assert_eq!(decision_entry["payload"]["admitted"], json!(true));
    assert_eq!(decision_entry["subject_is_candidate"], json!(true));
    let vault_dir = fixture.vault_dir.clone();
    let history_ids = fixture.history_ids.clone();
    let candidate_id = fixture.candidate_id;
    drop(fixture.vault);
    let reopened_vault = reopen_vault(&vault_dir);
    let reopened = Fixture {
        vault: reopened_vault,
        vault_dir,
        history_ids,
        candidate_id,
    };
    let reopened_state = source_state(&reopened, Some(ledger_ref.seq));
    assert_eq!(
        reopened_state["decision_ledger"]["entry"]["payload"]["p_model"],
        json!(prediction.p_model)
    );

    let evidence = json!({
        "trigger": "ingest known snapshots, ground history, associate candidate, predict p_model, admit forecast, append admission ledger row, flush, reopen",
        "expected": {
            "p_model_band": [0.93, 0.95],
            "admitted": true,
            "decision_ledger_kind": "admission"
        },
        "prediction": prediction.to_json(),
        "decision": decision,
        "ledger_ref": {
            "seq": ledger_ref.seq,
            "hash": hex(&ledger_ref.hash)
        },
        "before": before,
        "after": after,
        "reopened_after_close": reopened_state
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}
fn edge_no_grounded_associations_fails_closed(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("edge-no-grounded-associations"), false, 0.80);
    let before = source_state(&fixture, None);
    let error = associate_and_predict(&fixture)
        .err()
        .expect("ungrounded history must fail closed");
    assert_eq!(error.code, "CALYX_POLY_SCENARIO_NO_GROUNDED_ASSOCIATIONS");
    let after = source_state(&fixture, None);
    assert_eq!(before["ledger_count"], after["ledger_count"]);
    let evidence = json!({
        "trigger": "attempt association with ingested but ungrounded history",
        "error": error.to_json(),
        "before": before,
        "after": after
    });
    write_json(
        &root.join("edge-no-grounded-associations-readback.json"),
        &evidence,
    );
    evidence
}
fn edge_guard_refusal_writes_no_decision_ledger(root: &Path) -> Value {
    let fixture = setup_fixture(&root.join("edge-guard-refusal"), true, 0.80);
    let before = source_state(&fixture, None);
    let prediction = associate_and_predict(&fixture).expect("grounded pattern predicts");
    let decision = admission_decision(&fixture, prediction.p_model, false);
    assert!(!decision.admitted);
    assert!(decision.reason.contains("ward guard"));
    let after = source_state(&fixture, None);
    assert_eq!(before["ledger_count"], after["ledger_count"]);
    let evidence = json!({
        "trigger": "grounded pattern predicts but guard_pass=false",
        "prediction": prediction.to_json(),
        "decision": decision,
        "before": before,
        "after": after
    });
    write_json(&root.join("edge-guard-refusal-readback.json"), &evidence);
    evidence
}

fn setup_fixture(root: &Path, ground_history: bool, candidate_ask: f64) -> Fixture {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir);
    let vault_id = vault_id();
    let panel = default_panel(1, vec!["global".to_string()]);

    let win_a = snapshot("win-a", 0.40, 0.018, 0.72);
    let win_b = snapshot("win-b", 0.36, 0.020, 0.74);
    let error = snapshot("error-a", -0.24, 0.045, 0.42);
    let candidate = snapshot("candidate", 0.39, 0.019, candidate_ask);

    let win_a_id =
        ingest_snapshot(&vault, &panel, &win_a, vault_id, VAULT_SALT).expect("ingest win-a");
    let win_b_id =
        ingest_snapshot(&vault, &panel, &win_b, vault_id, VAULT_SALT).expect("ingest win-b");
    let error_id =
        ingest_snapshot(&vault, &panel, &error, vault_id, VAULT_SALT).expect("ingest error");
    let candidate_id = ingest_snapshot(&vault, &panel, &candidate, vault_id, VAULT_SALT)
        .expect("ingest candidate");

    if ground_history {
        ground_market(&vault, &[win_a_id], &resolution("win-a", true), 0).expect("ground win-a");
        ground_market(&vault, &[win_b_id], &resolution("win-b", true), 0).expect("ground win-b");
        ground_market(&vault, &[error_id], &resolution("error-a", false), 0).expect("ground error");
    }
    vault.flush().expect("flush setup");

    Fixture {
        vault,
        vault_dir,
        history_ids: vec![win_a_id, win_b_id, error_id],
        candidate_id,
    }
}

fn associate_and_predict(fixture: &Fixture) -> Result<Prediction, ScenarioError> {
    let snapshot = fixture.vault.snapshot();
    let candidate = fixture
        .vault
        .get(fixture.candidate_id, snapshot)
        .map_err(|err| ScenarioError::new("CALYX_POLY_SCENARIO_READ_FAILED", err.to_string()))?;
    let candidate_ofi = scalar(&candidate, "ofi")?;
    let candidate_spread = scalar(&candidate, "spread")?;

    let mut associated = 0_u32;
    let mut positive = 0_u32;
    let mut neighbors = Vec::new();
    for id in &fixture.history_ids {
        let stored = fixture.vault.get(*id, snapshot).map_err(|err| {
            ScenarioError::new("CALYX_POLY_SCENARIO_READ_FAILED", err.to_string())
        })?;
        let Some(outcome) = outcome_anchor(&stored) else {
            continue;
        };
        let ofi = scalar(&stored, "ofi")?;
        let spread = scalar(&stored, "spread")?;
        let similar =
            (ofi - candidate_ofi).abs() <= 0.10 && (spread - candidate_spread).abs() <= 0.02;
        if similar {
            associated += 1;
            positive += u32::from(outcome);
            neighbors.push(json!({
                "cx_id": id.to_string(),
                "ofi": ofi,
                "spread": spread,
                "outcome_win": outcome
            }));
        }
    }
    if associated == 0 {
        return Err(ScenarioError::new(
            "CALYX_POLY_SCENARIO_NO_GROUNDED_ASSOCIATIONS",
            "candidate has no grounded similar neighbors",
        ));
    }
    let win_rate = f64::from(positive) / f64::from(associated);
    Ok(Prediction {
        associated,
        positive,
        p_model: 0.50 + 0.44 * win_rate,
        neighbors,
    })
}

fn admission_decision(
    fixture: &Fixture,
    p_model: f64,
    guard_pass: bool,
) -> calyx_poly::admission::AdmissionDecision {
    let stored = fixture
        .vault
        .get(fixture.candidate_id, fixture.vault.snapshot())
        .expect("read candidate for admission");
    let inputs = AdmissionInputs {
        p_win: p_model,
        confidence: 0.74,
        sufficiency_ok: true,
        evidence_count: fixture.history_ids.len() as u32,
        source_derived_evidence_count: fixture.history_ids.len() as u32,
        stale_evidence_count: 0,
        circular_evidence_count: 0,
        super_intel_pass: true,
        guard_calibrated: true,
        grounding_anchor_count: fixture.history_ids.len() as u32,
        guard_pass,
        liquidity_ok: scalar(&stored, "liquidity").expect("candidate liquidity") >= 25_000.0,
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: known_healthy_oracle_risk(),
        wash_trade: known_healthy_wash_trade(),
        kill_switch_active: false,
        daily_error_score: 0.0,
    };
    let params = AdmissionParams {
        min_grounding_anchors: fixture.history_ids.len() as u32,
        ..AdmissionParams::default()
    };
    evaluate_admission(&params, &inputs)
}

fn append_decision_ledger(
    fixture: &Fixture,
    p_model: f64,
    decision: &calyx_poly::admission::AdmissionDecision,
) -> calyx_core::LedgerRef {
    let payload = json!({
        "scenario": "issue133_known_truth",
        "p_model": p_model,
        "admitted": decision.admitted,
        "code": decision.code,
        "reason": decision.reason,
    });
    fixture
        .vault
        .append_ledger_entry(
            EntryKind::Admission,
            SubjectId::Cx(fixture.candidate_id),
            serde_json::to_vec(&payload).expect("encode admission payload"),
            ActorId::Service("calyx-poly-issue133".to_string()),
        )
        .expect("append admission decision ledger entry")
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
    let groundings: Vec<_> = fixture
        .history_ids
        .iter()
        .map(|id| anchor_state(&fixture.vault, snapshot, *id))
        .collect();
    json!({
        "snapshot": snapshot,
        "candidate_base": {
            "present": candidate_base.is_some(),
            "bytes": candidate_base.as_ref().map(Vec::len),
            "row_hash": candidate_base.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string()),
            "decoded": {
                "cx_id": candidate.cx_id.to_string(),
                "ungrounded": candidate.flags.ungrounded,
                "scalars": {
                    "ofi": candidate.scalars.get("ofi"),
                    "spread": candidate.scalars.get("spread"),
                    "best_ask": candidate.scalars.get("best_ask"),
                    "liquidity": candidate.scalars.get("liquidity")
                }
            }
        },
        "history_groundings": groundings,
        "ledger_count": ledger_rows.len(),
        "decision_ledger": decision_seq
            .map(|seq| ledger_state(&fixture.vault, snapshot, seq, fixture.candidate_id))
            .unwrap_or_else(|| json!({"present": false}))
    })
}

fn anchor_state(vault: &AsterVault, snapshot: u64, id: CxId) -> Value {
    let key = anchor_key(id, &AnchorKind::TestPass);
    let row = vault
        .read_cf_at(snapshot, ColumnFamily::Anchors, &key)
        .expect("read anchor row");
    let decoded = row
        .as_ref()
        .map(|bytes| encode::decode_anchor(bytes).expect("decode anchor"));
    json!({
        "cx_id": id.to_string(),
        "present": row.is_some(),
        "row_hash": row.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string()),
        "decoded": decoded.map(|anchor| json!({
            "kind": format!("{:?}", anchor.kind),
            "value": format!("{:?}", anchor.value),
            "confidence": anchor.confidence,
            "source": anchor.source
        }))
    })
}

fn ledger_state(vault: &AsterVault, snapshot: u64, seq: u64, candidate_id: CxId) -> Value {
    let row = vault
        .read_cf_at(snapshot, ColumnFamily::Ledger, &ledger_key(seq))
        .expect("read decision ledger row")
        .expect("decision ledger row exists");
    let entry = decode_ledger(&row).expect("decode decision ledger row");
    let payload: Value = serde_json::from_slice(&entry.payload).expect("decode admission payload");
    let subject_is_candidate = matches!(&entry.subject, SubjectId::Cx(id) if *id == candidate_id);
    json!({
        "present": true,
        "bytes": row.len(),
        "row_hash": blake3::hash(&row).to_hex().to_string(),
        "entry": {
            "seq": entry.seq,
            "kind": entry.kind.as_str(),
            "subject_is_candidate": subject_is_candidate,
            "entry_hash": hex(&entry.entry_hash),
            "payload": payload
        }
    })
}

fn snapshot(slug: &str, ofi: f64, spread: f64, best_ask: f64) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("token-{slug}"),
        condition_id: format!("condition-{slug}"),
        outcome_index: 0,
        slug: format!("issue133-{slug}"),
        question: Some(format!("Issue 133 {slug} market?")),
        event_id: Some("issue133-event".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue133".to_string(), "known-pattern".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(best_ask - 0.01),
        mid: Some(best_ask - 0.005),
        best_bid: Some(best_ask - spread),
        best_ask: Some(best_ask),
        spread: Some(spread),
        tick_size: Some(0.01),
        volume_24h: Some(150_000.0),
        liquidity: Some(40_000.0),
        one_hour_change: Some(0.02),
        one_day_change: Some(0.08),
        ofi: Some(ofi),
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

fn resolution(slug: &str, won: bool) -> Resolution {
    Resolution {
        condition_id: format!("condition-{slug}"),
        winning_outcome_index: if won { 0 } else { 1 },
        winning_label: if won { "YES" } else { "NO" }.to_string(),
        resolved_ts: 1_785_600_000,
        source: "uma".to_string(),
        disputed: false,
    }
}

struct Fixture {
    vault: AsterVault,
    vault_dir: PathBuf,
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
    .expect("open durable issue133 vault")
}

fn reopen_vault(dir: &Path) -> AsterVault {
    AsterVault::open(
        dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("reopen durable issue133 vault")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
