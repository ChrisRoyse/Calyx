use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use calyx_anneal::{
    AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry, AsterAnnealLedgerStore,
    CALYX_LEDGER_ENTRY_TOO_LARGE, ChangeId, MetricComparison, MetricSnapshot, TripwireMetric,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::FixedClock;
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use fsv_support::{hex, reset_dir, vault_id, write_json, write_manifest};
use serde_json::{Value, json};

const FSV_TS: u64 = 1_785_500_398;

#[test]
#[ignore = "requires CALYX_ISSUE398_FSV_ROOT in a manual verification run"]
fn issue398_anneal_ledger_writer_fsv() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE398_FSV_ROOT").expect("set CALYX_ISSUE398_FSV_ROOT"));
    fs::create_dir_all(&root).expect("create FSV root");

    let vault_dir = root.join("vault");
    reset_dir(&vault_dir);
    let vault = open_vault(&vault_dir);

    let before_rows = read_ledger_rows(&vault);
    assert!(before_rows.is_empty());

    let mut ledger = open_anneal_ledger(&vault);
    let promote = event(ChangeId(398_001), AnnealLedgerAction::Promote, "promote");
    let promote_ref = ledger.write(promote).expect("write promote");
    let revert = event(ChangeId(398_001), AnnealLedgerAction::Revert, "revert");
    let revert_ref = ledger.write(revert).expect("write revert");
    vault.flush().expect("flush after happy path");
    let after_rows = read_ledger_rows(&vault);
    assert_eq!(after_rows.len(), 2);
    assert_eq!(after_rows[1]["payload_json"]["action"], "revert");
    assert_eq!(after_rows[1]["payload_json"]["change_id"], 398_001);
    assert_eq!(
        after_rows[1]["payload_json"]["prior_ptr_hash"],
        hex(&[0x11; 32])
    );

    let empty_before = read_ledger_rows(&vault);
    let mut empty_description = event(ChangeId(398_002), AnnealLedgerAction::Park, "");
    empty_description.description.clear();
    let empty_ref = ledger
        .write(empty_description)
        .expect("write empty description");
    vault.flush().expect("flush empty description");
    let empty_after = read_ledger_rows(&vault);
    assert_eq!(empty_after.len(), empty_before.len() + 1);
    assert_eq!(empty_after[2]["payload_json"]["description"], "");

    let oversized_before = read_ledger_rows(&vault);
    let mut oversized = event(
        ChangeId(398_003),
        AnnealLedgerAction::Recalibrate,
        "oversized",
    );
    oversized.description = "oversized ".repeat(4096);
    let oversized_code = match ledger.write(oversized) {
        Ok(_) => "ok".to_string(),
        Err(error) => error.code.to_string(),
    };
    let oversized_after = read_ledger_rows(&vault);
    assert_eq!(oversized_code, CALYX_LEDGER_ENTRY_TOO_LARGE);
    assert_eq!(oversized_after.len(), oversized_before.len());

    let mismatch_before = read_ledger_rows(&vault);
    let mut mismatch = event(
        ChangeId(398_004),
        AnnealLedgerAction::MistakeUpdate,
        "wrong prev",
    );
    mismatch.prev_hash = Some([0xff; 32]);
    let mismatch_code = match ledger.write(mismatch) {
        Ok(_) => "ok".to_string(),
        Err(error) => error.code.to_string(),
    };
    let mismatch_after = read_ledger_rows(&vault);
    assert_eq!(mismatch_code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert_eq!(mismatch_after.len(), mismatch_before.len());

    let empty_vault_dir = root.join("empty-vault");
    reset_dir(&empty_vault_dir);
    let empty_vault = open_vault(&empty_vault_dir);
    let empty_cf_before = read_ledger_rows(&empty_vault);
    let empty_ledger = open_anneal_ledger(&empty_vault);
    let empty_read = empty_ledger.read_recent(10).expect("read empty");
    let empty_cf_after = read_ledger_rows(&empty_vault);
    assert!(empty_read.is_empty());
    assert!(empty_cf_before.is_empty());
    assert!(empty_cf_after.is_empty());

    let final_rows = read_ledger_rows(&vault);
    let readback_path = root.join("anneal-ledger-readback.json");
    write_json(
        &readback_path,
        &json!({
            "surface": "anneal.ledger_writer",
            "source_of_truth": "Aster vault ledger CF rows keyed by big-endian seq",
            "vault": vault_dir.display().to_string(),
            "trigger": "AnnealLedger::write Promote then Revert for synthetic change_id 398001",
            "expected": {
                "before_row_count": 0,
                "after_row_count": 2,
                "revert_change_id": 398001,
                "revert_prior_ptr_hash": hex(&[0x11; 32]),
                "kind": "anneal"
            },
            "actual_before": before_rows,
            "actual_after_happy_path": after_rows,
            "final_rows": final_rows,
            "ledger_refs": {
                "promote": promote_ref,
                "revert": revert_ref,
                "empty_description": empty_ref
            },
            "edges": [
                {
                    "case": "empty_description",
                    "before_row_count": empty_before.len(),
                    "after_row_count": empty_after.len(),
                    "description_after": empty_after[2]["payload_json"]["description"]
                },
                {
                    "case": "oversized_payload",
                    "expected": CALYX_LEDGER_ENTRY_TOO_LARGE,
                    "before_row_count": oversized_before.len(),
                    "result_code": oversized_code,
                    "after_row_count": oversized_after.len()
                },
                {
                    "case": "mismatched_prev_hash",
                    "expected": "CALYX_LEDGER_CHAIN_BROKEN",
                    "before_row_count": mismatch_before.len(),
                    "result_code": mismatch_code,
                    "after_row_count": mismatch_after.len()
                },
                {
                    "case": "read_empty_cf",
                    "before_rows": empty_cf_before,
                    "read_recent_result": empty_read,
                    "after_rows": empty_cf_after
                }
            ]
        }),
    );

    write_manifest(&root, &[readback_path]);
}

fn open_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        b"issue398-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn open_anneal_ledger(
    vault: &AsterVault,
) -> AnnealLedger<AsterAnnealLedgerStore<'_, calyx_core::SystemClock>, FixedClock> {
    let store = AsterAnnealLedgerStore::new(vault);
    let appender = LedgerAppender::open(store, FixedClock::new(FSV_TS)).unwrap();
    AnnealLedger::new(appender, ActorId::Service("calyx-anneal-fsv".to_string())).unwrap()
}

fn read_ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan ledger CF")
        .into_iter()
        .map(|(key, bytes)| {
            let entry = decode_ledger(&bytes).expect("decode ledger entry");
            assert_eq!(entry.kind, EntryKind::Anneal);
            assert_eq!(key, ledger_key(entry.seq));
            json!({
                "seq": entry.seq,
                "key_hex": hex(&key),
                "kind": entry.kind.as_str(),
                "prev_hash": hex(&entry.prev_hash),
                "entry_hash": hex(&entry.entry_hash),
                "payload_hex": hex(&entry.payload),
                "payload_json": serde_json::from_slice::<Value>(&entry.payload).unwrap(),
            })
        })
        .collect()
}

fn event(change_id: ChangeId, action: AnnealLedgerAction, label: &str) -> AnnealLedgerEntry {
    AnnealLedgerEntry {
        action,
        change_id,
        artifact_id: format!("artifact-{}", change_id.0),
        prior_ptr_hash: [0x11; 32],
        candidate_ptr_hash: [0x22; 32],
        metrics: MetricSnapshot {
            evaluated_at: FSV_TS,
            query_count: 4,
            metrics: vec![MetricComparison {
                metric: TripwireMetric::RecallAtK,
                candidate_value: 0.94,
                incumbent_value: 0.91,
            }],
        },
        ts: FSV_TS,
        description: format!("synthetic {label} for issue 398"),
        fault: None,
        proposal: None,
        details: None,
        prev_hash: None,
    }
}
