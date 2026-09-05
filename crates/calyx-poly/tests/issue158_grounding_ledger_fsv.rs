use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, anchor_key, base_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{AnchorKind, CxId, VaultId, VaultStore};
use calyx_ledger::{EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::constellation::resolution_anchor;
use calyx_poly::error::PolyError;
use calyx_poly::lenses::default_panel;
use calyx_poly::model::{MarketSnapshot, Resolution};
use calyx_poly::pipeline::{ground_market, ingest_snapshot};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_SALT: &[u8] = b"poly-issue158-grounding-ledger";

#[test]
fn issue158_grounding_writes_anchor_and_ledger_fsv() {
    let (root, keep_root) =
        named_fsv_root("POLY_ISSUE158_FSV_ROOT", "poly-issue158-grounding-ledger");
    reset_dir(&root);

    let happy = happy_path_grounding_has_anchor_and_ledger(&root);
    let unknown = edge_unknown_cx_fails_closed(&root);
    let duplicate = edge_duplicate_grounding_is_idempotent(&root);
    let conflict = edge_conflicting_grounding_fails_closed(&root);
    let legacy = edge_legacy_unstamped_anchor_fails_closed(&root);
    let commit_failure = edge_ledger_commit_failure_mutates_nothing(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 158,
            "source_of_truth": "real AsterVault Base, Anchors, and Ledger CF rows on disk",
            "happy_path": happy,
            "edge_cases": {
                "unknown_cx": unknown,
                "duplicate_grounding": duplicate,
                "conflicting_grounding": conflict,
                "legacy_unstamped_anchor": legacy,
                "ledger_commit_failure": commit_failure
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);

    if keep_root {
        println!("poly_issue158_fsv_root={}", root.display());
    }
}

fn happy_path_grounding_has_anchor_and_ledger(root: &Path) -> Value {
    let fixture = ingest_fixture(&root.join("happy"), VaultOptions::default());
    let before = source_state(&fixture.vault, fixture.cx_id, None);
    let refs = ground_market(&fixture.vault, &[fixture.cx_id], &resolution(true), 0)
        .expect("grounding should write anchor and ledger");
    assert_eq!(refs.len(), 1);
    fixture.vault.flush().expect("flush happy vault");
    let after = source_state(&fixture.vault, fixture.cx_id, Some(refs[0].seq));
    assert_eq!(after["anchor"]["decoded"]["value"], json!("Bool(true)"));
    assert_eq!(
        after["label_anchor"]["decoded"]["value"],
        json!("Enum(\"YES\")")
    );
    assert_eq!(after["ledger"]["entry"]["kind"], json!("grounding"));
    assert_eq!(after["ledger"]["entry"]["subject_is_cx"], json!(true));
    assert_eq!(
        after["ledger"]["entry"]["payload"]["event"],
        json!("poly.outcome_grounding")
    );
    assert_eq!(
        after["ledger"]["entry"]["payload"]["anchors"]
            .as_array()
            .expect("anchors array")
            .len(),
        2
    );
    drop(fixture.vault);

    let reopened = reopen_vault(&fixture.vault_dir, VaultOptions::default());
    let reopened_state = source_state(&reopened, fixture.cx_id, Some(refs[0].seq));
    assert_eq!(
        reopened_state["ledger"]["entry"]["kind"],
        json!("grounding")
    );

    let evidence = json!({
        "trigger": "ingest one Poly snapshot, ground known winning resolution, flush, reopen",
        "ledger_ref": {"seq": refs[0].seq, "hash": hex(&refs[0].hash)},
        "before": before,
        "after": after,
        "reopened_after_close": reopened_state
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_unknown_cx_fails_closed(root: &Path) -> Value {
    let dir = root.join("edge-unknown-cx").join("vault");
    let vault = open_vault(&dir, VaultOptions::default());
    let unknown = CxId::from_bytes([0x58; 16]);
    let before = source_state(&vault, unknown, None);
    let err = ground_market(&vault, &[unknown], &resolution(true), 0)
        .expect_err("unknown cx must fail closed");
    let error = error_json(err);
    assert_eq!(error["code"], json!("CALYX_STALE_DERIVED"));
    let after = source_state(&vault, unknown, None);
    assert_eq!(before, after);
    let evidence = json!({
        "trigger": "ground unknown CxId",
        "error": error,
        "before": before,
        "after": after
    });
    write_json(&root.join("edge-unknown-cx-readback.json"), &evidence);
    evidence
}

fn edge_duplicate_grounding_is_idempotent(root: &Path) -> Value {
    let fixture = ingest_fixture(&root.join("edge-duplicate"), VaultOptions::default());
    let first = ground_market(&fixture.vault, &[fixture.cx_id], &resolution(true), 0)
        .expect("initial grounding");
    fixture.vault.flush().expect("flush initial grounding");
    let before = source_state(&fixture.vault, fixture.cx_id, Some(first[0].seq));
    let second = ground_market(&fixture.vault, &[fixture.cx_id], &resolution(true), 0)
        .expect("duplicate grounding should be idempotent");
    let after = source_state(&fixture.vault, fixture.cx_id, Some(first[0].seq));
    assert_eq!(second[0].seq, first[0].seq);
    assert_eq!(before["ledger_count"], after["ledger_count"]);
    assert_eq!(before["anchor"], after["anchor"]);
    assert_eq!(before["label_anchor"], after["label_anchor"]);
    let evidence = json!({
        "trigger": "repeat identical grounding after the first ledger-stamped grounding",
        "first_ledger_ref": {"seq": first[0].seq, "hash": hex(&first[0].hash)},
        "second_ledger_ref": {"seq": second[0].seq, "hash": hex(&second[0].hash)},
        "before": before,
        "after": after
    });
    write_json(&root.join("edge-duplicate-readback.json"), &evidence);
    evidence
}

fn edge_conflicting_grounding_fails_closed(root: &Path) -> Value {
    let fixture = ingest_fixture(&root.join("edge-conflict"), VaultOptions::default());
    let first = ground_market(&fixture.vault, &[fixture.cx_id], &resolution(true), 0)
        .expect("initial grounding");
    fixture.vault.flush().expect("flush initial grounding");
    let before = source_state(&fixture.vault, fixture.cx_id, Some(first[0].seq));
    let err = ground_market(&fixture.vault, &[fixture.cx_id], &resolution(false), 0)
        .expect_err("conflicting grounding must fail closed");
    let error = error_json(err);
    assert_eq!(error["code"], json!("CALYX_ASTER_CORRUPT_SHARD"));
    let after = source_state(&fixture.vault, fixture.cx_id, Some(first[0].seq));
    assert_eq!(before["ledger_count"], after["ledger_count"]);
    assert_eq!(after["anchor"]["decoded"]["value"], json!("Bool(true)"));
    assert_eq!(
        after["label_anchor"]["decoded"]["value"],
        json!("Enum(\"YES\")")
    );
    let evidence = json!({
        "trigger": "ground same CxId with opposite outcome after initial grounding",
        "error": error,
        "before": before,
        "after": after
    });
    write_json(&root.join("edge-conflict-readback.json"), &evidence);
    evidence
}

fn edge_legacy_unstamped_anchor_fails_closed(root: &Path) -> Value {
    let fixture = ingest_fixture(&root.join("edge-legacy-unstamped"), VaultOptions::default());
    fixture
        .vault
        .anchor(fixture.cx_id, resolution_anchor(&resolution(true), 0))
        .expect("write legacy generic anchor without grounding ledger");
    fixture.vault.flush().expect("flush legacy anchor");
    let before = source_state(&fixture.vault, fixture.cx_id, None);
    assert_eq!(before["anchor"]["decoded"]["value"], json!("Bool(true)"));
    let err = ground_market(&fixture.vault, &[fixture.cx_id], &resolution(true), 0)
        .expect_err("legacy generic anchor must not pass as ledger-stamped grounding");
    let error = error_json(err);
    assert_eq!(error["code"], json!("CALYX_ASTER_CORRUPT_SHARD"));
    let after = source_state(&fixture.vault, fixture.cx_id, None);
    assert_eq!(before, after);
    let evidence = json!({
        "trigger": "pre-existing generic anchor without a grounding ledger row",
        "error": error,
        "before": before,
        "after": after
    });
    write_json(&root.join("edge-legacy-unstamped-readback.json"), &evidence);
    evidence
}

fn edge_ledger_commit_failure_mutates_nothing(root: &Path) -> Value {
    let fixture = ingest_fixture(&root.join("edge-commit-failure"), VaultOptions::default());
    drop(fixture.vault);
    let low_cap_vault = reopen_vault(
        &fixture.vault_dir,
        VaultOptions {
            memtable_byte_cap: 32,
            ..VaultOptions::default()
        },
    );
    let before = source_state(&low_cap_vault, fixture.cx_id, None);
    let err = ground_market(&low_cap_vault, &[fixture.cx_id], &resolution(true), 0)
        .expect_err("low memtable cap should reject the atomic grounding batch");
    let error = error_json(err);
    assert_eq!(error["code"], json!("CALYX_BACKPRESSURE"));
    let after = source_state(&low_cap_vault, fixture.cx_id, None);
    assert_eq!(before, after);
    let evidence = json!({
        "trigger": "reopen with memtable_byte_cap=32 and attempt ledger-stamped grounding",
        "error": error,
        "before": before,
        "after": after
    });
    write_json(&root.join("edge-commit-failure-readback.json"), &evidence);
    evidence
}

fn ingest_fixture(root: &Path, options: VaultOptions) -> Fixture {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir, options);
    let panel = default_panel(1, vec!["global".to_string()]);
    let cx_id = ingest_snapshot(&vault, &panel, &snapshot(), vault_id(), VAULT_SALT)
        .expect("ingest fixture snapshot");
    vault.flush().expect("flush fixture ingest");
    Fixture {
        vault,
        vault_dir,
        cx_id,
    }
}

fn source_state(vault: &AsterVault, cx_id: CxId, ledger_seq: Option<u64>) -> Value {
    let snapshot = vault.snapshot();
    let base = vault
        .read_cf_at(snapshot, ColumnFamily::Base, &base_key(cx_id))
        .expect("read base");
    let anchor = anchor_state(vault, snapshot, cx_id, &AnchorKind::TestPass);
    let label_anchor = anchor_state(
        vault,
        snapshot,
        cx_id,
        &AnchorKind::Label("outcome".to_string()),
    );
    let ledger_rows = vault
        .scan_cf_at(snapshot, ColumnFamily::Ledger)
        .expect("scan ledger");
    json!({
        "snapshot": snapshot,
        "base": {
            "present": base.is_some(),
            "bytes": base.as_ref().map(Vec::len),
            "row_hash": base.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string()),
            "decoded": base.as_ref().map(|bytes| {
                let cx = encode::decode_constellation_base(bytes).expect("decode base");
                json!({
                    "cx_id": cx.cx_id.to_string(),
                    "ungrounded": cx.flags.ungrounded,
                    "anchor_count": cx.anchors.len(),
                    "provenance_seq": cx.provenance.seq,
                    "provenance_hash": hex(&cx.provenance.hash)
                })
            })
        },
        "anchor": anchor,
        "label_anchor": label_anchor,
        "ledger_count": ledger_rows.len(),
        "ledger": ledger_seq
            .map(|seq| ledger_state(vault, snapshot, seq, cx_id))
            .unwrap_or_else(|| json!({"present": false}))
    })
}

fn anchor_state(vault: &AsterVault, snapshot: u64, cx_id: CxId, kind: &AnchorKind) -> Value {
    let row = vault
        .read_cf_at(snapshot, ColumnFamily::Anchors, &anchor_key(cx_id, kind))
        .expect("read anchor");
    let decoded = row
        .as_ref()
        .map(|bytes| encode::decode_anchor(bytes).expect("decode anchor"));
    json!({
        "present": row.is_some(),
        "bytes": row.as_ref().map(Vec::len),
        "row_hash": row.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string()),
        "decoded": decoded.map(|anchor| json!({
            "kind": format!("{:?}", anchor.kind),
            "value": format!("{:?}", anchor.value),
            "source": anchor.source,
            "observed_at": anchor.observed_at,
            "confidence": anchor.confidence
        }))
    })
}

fn ledger_state(vault: &AsterVault, snapshot: u64, seq: u64, cx_id: CxId) -> Value {
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
            "is_grounding_kind": entry.kind == EntryKind::Grounding,
            "subject_is_cx": matches!(&entry.subject, SubjectId::Cx(id) if *id == cx_id),
            "entry_hash": hex(&entry.entry_hash),
            "payload": payload
        }
    })
}

fn error_json(err: PolyError) -> Value {
    match err {
        PolyError::Grounding { code, message } => json!({"code": code, "message": message}),
        PolyError::Calyx { code, message } => json!({"code": code, "message": message}),
        other => panic!("unexpected error variant: {other}"),
    }
}

fn snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "issue158-token".to_string(),
        condition_id: "issue158-condition".to_string(),
        outcome_index: 0,
        slug: "issue158-ledger-grounding".to_string(),
        question: Some("Issue 158 ledger grounding market?".to_string()),
        event_id: Some("issue158-event".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue158".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
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
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

fn resolution(won: bool) -> Resolution {
    Resolution {
        condition_id: "issue158-condition".to_string(),
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
    cx_id: CxId,
}

fn open_vault(dir: &Path, options: VaultOptions) -> AsterVault {
    AsterVault::new_durable(dir, vault_id(), VAULT_SALT.to_vec(), options)
        .expect("open issue158 vault")
}

fn reopen_vault(dir: &Path, options: VaultOptions) -> AsterVault {
    AsterVault::open(dir, vault_id(), VAULT_SALT.to_vec(), options).expect("reopen issue158 vault")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
