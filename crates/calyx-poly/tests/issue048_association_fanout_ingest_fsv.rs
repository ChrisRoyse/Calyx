//! Issue #48 - association fan-out pipeline wired into ingest.
//!
//! Source of truth: durable AsterVault base/ledger column families plus the persisted fan-out
//! selection report read back from disk.

use std::fs;
use std::path::Path;

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{VaultId, VaultStore};
use calyx_ledger::{EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::constellation::build_constellation;
use calyx_poly::fanout_selection::{
    ExpensiveAssociationEstimator, FanoutCandidate, FanoutSelectionRequest, FanoutThresholds,
    read_fanout_selection_report,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::{Book, MarketSnapshot, OracleRiskEvidence};
use calyx_poly::pipeline::{
    ASSOCIATION_FANOUT_INGEST_SCHEMA_VERSION, ERR_ASSOCIATION_FANOUT_INVALID_REQUEST,
    ERR_ASSOCIATION_FANOUT_NO_SELECTED, ingest_snapshot_with_association_fanout,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const PANEL_VERSION: u32 = 1;
const VAULT_SALT: &[u8] = b"issue048-association-fanout-salt";

#[test]
fn issue048_association_fanout_ingest_fsv() {
    let (root, _keep) =
        named_fsv_root("POLY_ISSUE048_FSV_ROOT", "poly-issue048-association-fanout");
    reset_dir(&root);

    let happy = happy_path(&root);
    let panel_mismatch = edge_case(
        &root,
        "edge-panel-version-mismatch",
        |mut request| {
            request.panel_version = PANEL_VERSION + 1;
            request
        },
        ERR_ASSOCIATION_FANOUT_INVALID_REQUEST,
    );
    let duplicate = edge_case(
        &root,
        "edge-duplicate-pair",
        |mut request| {
            let mut duplicate = request.candidates[0].clone();
            std::mem::swap(&mut duplicate.left_key, &mut duplicate.right_key);
            duplicate.pair_id = "duplicate-reversed".to_string();
            request.candidates.push(duplicate);
            request
        },
        "CALYX_POLY_FANOUT_SELECTION_DUPLICATE_PAIR",
    );
    let empty = edge_case(
        &root,
        "edge-empty-candidates",
        |mut request| {
            request.candidates.clear();
            request
        },
        "CALYX_POLY_FANOUT_SELECTION_EMPTY",
    );
    let no_selected = edge_case(
        &root,
        "edge-no-expensive-confirm-candidates",
        |mut request| {
            request.candidates = vec![candidate(
                "below-only",
                "noise_a",
                "noise_b",
                0.01,
                0.02,
                0.03,
            )];
            request
        },
        ERR_ASSOCIATION_FANOUT_NO_SELECTED,
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 48,
        "proof_claim": "Poly ingest can run bounded association fan-out as part of the ingest path, persist the cheap-screen selection report, write the snapshot to the durable vault, and append a ledger row tying the ingested CxId to the fan-out report hash.",
        "minimum_sufficient_corpus": {
            "snapshots": 1,
            "cheap_screen_candidates": 4,
            "edge_requests": 4,
            "why_this_is_sufficient": "one snapshot is the smallest durable ingest source; four candidates are the smallest happy corpus that proves selected expensive-confirm pairs, fan-out cap drops, below-screen drops, report readback, vault base persistence, and ledger linkage in one run.",
            "why_smaller_is_insufficient": "without one snapshot there is no ingest CxId to ledger; fewer than four candidates cannot prove both selected and both drop classes while preserving deterministic cap order.",
            "why_larger_is_wasteful": "more snapshots or candidates would repeat the same preflight, report write/readback, vault put, and ledger append paths without adding another #48 behavior."
        },
        "source_of_truth": "durable AsterVault Base/Ledger column families plus persisted fan-out selection JSON report",
        "happy_path": happy,
        "edge_cases": {
            "panel_version_mismatch": panel_mismatch,
            "duplicate_pair": duplicate,
            "empty_candidates": empty,
            "no_expensive_confirm_candidates": no_selected
        },
        "physical_files": files
    });
    let summary_path = root.join("issue048_association_fanout_ingest_fsv_report.json");
    write_json(&summary_path, &summary);
    write_blake3sums(&root);
    println!(
        "ISSUE048_ASSOCIATION_FANOUT_INGEST_FSV={}",
        summary_path.display()
    );
}

fn happy_path(root: &Path) -> Value {
    let case_dir = root.join("happy");
    let fanout_dir = case_dir.join("fanout");
    let source_path = case_dir.join("cheap-screen-source.json");
    let request = known_request();
    write_json(
        &source_path,
        &serde_json::to_value(&request).expect("fanout request json"),
    );
    let request_readback: FanoutSelectionRequest =
        serde_json::from_slice(&fs::read(&source_path).expect("read source request"))
            .expect("decode source request");
    assert_eq!(request_readback, request);

    let store = open_vault(&case_dir.join("vault"));
    let panel = default_panel(PANEL_VERSION, vec!["global".to_string()]);
    let snapshot = snapshot("happy");
    let before = source_counts(&store);
    let run = ingest_snapshot_with_association_fanout(
        &store,
        &panel,
        &snapshot,
        vault_id(),
        VAULT_SALT,
        &fanout_dir,
        &request_readback,
    )
    .expect("ingest with association fanout");
    store.flush().expect("flush happy vault");
    let after = source_counts(&store);

    let stored = store
        .get(run.cx_id, store.snapshot())
        .expect("read durable source constellation");
    assert_eq!(stored.cx_id, run.cx_id);
    let report = read_fanout_selection_report(&run.fanout_report_path).expect("report readback");
    assert_eq!(report, run.fanout_report);
    assert_eq!(report.selected_count, 2);
    assert_eq!(report.dropped_count, 2);
    assert_eq!(
        report
            .selected
            .iter()
            .map(|decision| decision.pair_id.as_str())
            .collect::<Vec<_>>(),
        vec!["pair-edge", "pair-nmi"]
    );

    let ledger_row = store
        .read_cf_at(
            store.snapshot(),
            ColumnFamily::Ledger,
            &ledger_key(run.ledger_ref.seq),
        )
        .expect("read fanout ledger row")
        .expect("fanout ledger row exists");
    let ledger = decode_ledger(&ledger_row).expect("decode fanout ledger row");
    assert_eq!(ledger.kind, EntryKind::Measure);
    assert!(matches!(ledger.subject, SubjectId::Cx(cx) if cx == run.cx_id));
    assert_eq!(ledger.seq, 1);
    assert_eq!(run.ledger_ref.hash, ledger.entry_hash);
    let payload: Value = serde_json::from_slice(&ledger.payload).expect("fanout payload json");
    let report_bytes = fs::read(&run.fanout_report_path).expect("read report bytes");
    let report_blake3 = blake3::hash(&report_bytes).to_hex().to_string();
    assert_eq!(
        payload["schema_version"],
        json!(ASSOCIATION_FANOUT_INGEST_SCHEMA_VERSION)
    );
    assert_eq!(payload["cx_id"], json!(run.cx_id.to_string()));
    let payload_hash = payload["fanout_report_blake3_chunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(payload_hash, report_blake3);
    assert_eq!(payload["selected_count"], json!(2));
    assert_eq!(payload["dropped_count"], json!(2));
    assert_eq!(payload["selected_pairs"].as_array().unwrap().len(), 2);

    let evidence = json!({
        "trigger": "ingest one snapshot with four known cheap-screen candidates",
        "source_request_path": source_path.display().to_string(),
        "before": before,
        "after": after,
        "cx_id": run.cx_id.to_string(),
        "fanout_report_path": run.fanout_report_path.display().to_string(),
        "fanout_report_blake3": report_blake3,
        "ledger_ref": {"seq": run.ledger_ref.seq, "hash": hex(&run.ledger_ref.hash)},
        "stored_metadata_question": stored.metadata.get("question"),
        "report_readback": report,
        "ledger_payload": payload
    });
    write_json(&case_dir.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_case<F>(root: &Path, name: &str, mutate: F, expected_code: &str) -> Value
where
    F: FnOnce(FanoutSelectionRequest) -> FanoutSelectionRequest,
{
    let case_dir = root.join(name);
    let fanout_dir = case_dir.join("fanout");
    let source_path = case_dir.join("cheap-screen-source.json");
    let request = mutate(known_request());
    write_json(
        &source_path,
        &serde_json::to_value(&request).expect("edge request json"),
    );
    let request_readback: FanoutSelectionRequest =
        serde_json::from_slice(&fs::read(&source_path).expect("read edge source request"))
            .expect("decode edge source request");

    let store = open_vault(&case_dir.join("vault"));
    let panel = default_panel(PANEL_VERSION, vec!["global".to_string()]);
    let snapshot = snapshot(name);
    let expected_cx = build_constellation(&snapshot, &panel, vault_id(), VAULT_SALT)
        .expect("precompute edge cx")
        .cx_id;
    let before = source_counts(&store);
    let err = ingest_snapshot_with_association_fanout(
        &store,
        &panel,
        &snapshot,
        vault_id(),
        VAULT_SALT,
        &fanout_dir,
        &request_readback,
    )
    .expect_err("edge must fail closed");
    assert_eq!(err.code(), expected_code);
    let after = source_counts(&store);
    assert_eq!(before, after);
    assert!(store.get(expected_cx, store.snapshot()).is_err());
    let report_path = fanout_dir.join("fanout_selection_crypto_v1.json");
    assert!(!report_path.exists());
    let evidence = json!({
        "trigger": name,
        "expected_code": expected_code,
        "actual_code": err.code(),
        "message": err.message(),
        "source_request_path": source_path.display().to_string(),
        "expected_cx": expected_cx.to_string(),
        "before": before,
        "after": after,
        "fanout_report_present": report_path.exists()
    });
    write_json(&case_dir.join(format!("{name}-readback.json")), &evidence);
    evidence
}

fn known_request() -> FanoutSelectionRequest {
    FanoutSelectionRequest {
        domain: "crypto".to_string(),
        panel_version: PANEL_VERSION,
        thresholds: FanoutThresholds {
            min_normalized_mutual_info: 0.20,
            min_abs_spearman: 0.20,
            min_edge_weight: 0.15,
            max_expensive_candidates: 2,
        },
        expensive_estimators: vec![
            ExpensiveAssociationEstimator::TransferEntropy,
            ExpensiveAssociationEstimator::DistanceCorrelation,
            ExpensiveAssociationEstimator::PermutationConfirm,
        ],
        candidates: vec![
            candidate(
                "pair-edge",
                "holder_membership",
                "maker_share",
                0.03,
                0.04,
                0.45,
            ),
            candidate("pair-nmi", "question_bm25", "outcome", 0.50, 0.05, 0.02),
            candidate("pair-cap", "volume", "spread", 0.30, 0.05, 0.02),
            candidate("pair-below", "noise_a", "noise_b", 0.02, 0.04, 0.03),
        ],
    }
}

fn candidate(
    pair_id: &str,
    left_key: &str,
    right_key: &str,
    normalized_mutual_info: f64,
    abs_spearman: f64,
    edge_weight: f64,
) -> FanoutCandidate {
    FanoutCandidate {
        pair_id: pair_id.to_string(),
        left_key: left_key.to_string(),
        right_key: right_key.to_string(),
        normalized_mutual_info,
        abs_spearman,
        edge_weight,
        provenance: "issue048-known-cheap-screen-score".to_string(),
    }
}

fn snapshot(suffix: &str) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("tok-{suffix}"),
        condition_id: format!("cond-{suffix}"),
        outcome_index: 0,
        slug: format!("will-btc-100k-{suffix}"),
        question: Some("Will Bitcoin trade above 100000 dollars before June?".to_string()),
        event_id: Some(format!("event-{suffix}")),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["crypto".to_string(), "bitcoin".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_048,
        price: Some(0.62),
        mid: Some(0.62),
        best_bid: Some(0.61),
        best_ask: Some(0.63),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(125_000.0),
        liquidity: Some(40_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.03),
        ofi: Some(0.2),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: Some(1_785_500_048),
        sequence_position: Some(1),
        sequence_total: Some(3),
        oracle_risk: OracleRiskEvidence::default(),
        book: Book::default(),
    }
}

fn open_vault(dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable test vault")
}

fn source_counts(store: &AsterVault) -> Value {
    json!({
        "base_count": store.scan_cf_at(store.snapshot(), ColumnFamily::Base).expect("scan base").len(),
        "ledger_count": store.scan_cf_at(store.snapshot(), ColumnFamily::Ledger).expect("scan ledger").len()
    })
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
