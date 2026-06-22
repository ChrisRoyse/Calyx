use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    AdmissionRecord, AnnealLedger, AsterAnnealLedgerStore, CALYX_ASSAY_INVALID_METRIC, ChangeId,
    GateOutcome, LensAdmittedEntry, ProposalTerminalState, ProposeLens, ProposeLensRequest,
    RejectReason, record_admitted, record_proposal_outcome,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::FixedClock;
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use serde_json::{Value, json};

#[path = "fsv_support/mod.rs"]
mod fsv_support;
#[path = "support/propose_lens.rs"]
#[allow(dead_code)]
mod support;
use fsv_support::{reset_dir, vault_id, write_json, write_manifest, write_physical_size_list};
use support::*;

const FSV_TS: u64 = 1_785_500_422;

#[test]
#[ignore = "requires CALYX_ISSUE422_FSV_ROOT in a manual verification run"]
fn lens_proposal_integration() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE422_FSV_ROOT").expect("set CALYX_ISSUE422_FSV_ROOT"));
    fs::create_dir_all(&root).expect("create FSV root");

    let vault_dir = root.join("vault");
    reset_dir(&vault_dir);
    let vault = open_vault(&vault_dir);
    let before_rows = read_ledger_rows(&vault);
    assert!(before_rows.is_empty());

    let mut ledger = open_anneal_ledger(&vault);
    let admitted = admitted_proposal();
    assert!(admitted.admitted);
    assert_eq!(admitted.terminal_state, ProposalTerminalState::Admitted);
    assert_eq!(admitted.sufficiency_before, 0.20);
    assert_eq!(admitted.sufficiency_after, Some(0.80));

    let admitted_ref = record_proposal_outcome(&admitted, &mut ledger, FSV_TS, 0.80)
        .expect("record admitted")
        .expect("admitted ledger ref");
    vault.flush().expect("flush admitted record");
    let after_admitted_rows = read_ledger_rows(&vault);
    assert_eq!(after_admitted_rows.len(), 1);
    assert_eq!(
        after_admitted_rows[0]["payload_json"]["action"],
        "LensAdmitted"
    );

    let rejected_before_rows = read_ledger_rows(&vault);
    let rejected = rejected_proposal();
    assert_eq!(rejected.terminal_state, ProposalTerminalState::GateRejected);
    assert!(matches!(
        rejected.gate_outcome,
        Some(GateOutcome::Rejected {
            reason: RejectReason::InsufficientBits { .. }
        })
    ));
    let rejected_ref = record_proposal_outcome(&rejected, &mut ledger, FSV_TS + 1, 0.80)
        .expect("record rejected")
        .expect("rejected ledger ref");
    vault.flush().expect("flush rejected record");
    let after_rejected_rows = read_ledger_rows(&vault);
    assert_eq!(after_rejected_rows.len(), 2);
    assert_eq!(
        after_rejected_rows[1]["payload_json"]["action"],
        "LensRejected"
    );

    let history = calyx_anneal::proposal_history_with_refs(&ledger, 5).expect("history");
    assert_eq!(history.len(), 2);
    assert!(matches!(
        history[0].record,
        AdmissionRecord::LensAdmitted(LensAdmittedEntry {
            sufficiency_before: 0.20,
            sufficiency_after: 0.80,
            ..
        })
    ));
    assert!(matches!(
        history[1].record,
        AdmissionRecord::LensRejected(_)
    ));

    let zero_before = read_ledger_rows(&vault);
    let zero_history = calyx_anneal::proposal_history_with_refs(&ledger, 0).expect("zero history");
    let zero_after = read_ledger_rows(&vault);
    assert!(zero_history.is_empty());
    assert_eq!(zero_before, zero_after);

    let invalid_before = read_ledger_rows(&vault);
    let invalid_error = record_admitted(&invalid_admitted_record(), &mut ledger)
        .unwrap_err()
        .code
        .to_string();
    let invalid_after = read_ledger_rows(&vault);
    assert_eq!(invalid_error, CALYX_ASSAY_INVALID_METRIC);
    assert_eq!(invalid_before, invalid_after);

    let final_rows = read_ledger_rows(&vault);
    let readback_path = root.join("lens-proposal-readback.json");
    write_json(
        &readback_path,
        &json!({
            "surface": "anneal.lens_proposal.admission_record",
            "source_of_truth": "Aster vault ledger CF rows for LensAdmitted/LensRejected",
            "vault": vault_dir.display().to_string(),
            "trigger": "ProposeLens::propose_lens admitted synthetic candidate and gate-rejected below-threshold candidate, then AdmissionRecord wrote Ledger entries",
            "expected": {
                "before_row_count": 0,
                "after_row_count": 2,
                "admitted_sufficiency_before": 0.20,
                "admitted_sufficiency_after": 0.80,
                "rejected_reason": "InsufficientBits",
            },
            "actual_before": before_rows,
            "actual_after_admitted": after_admitted_rows,
            "actual_before_rejected": rejected_before_rows,
            "actual_after_rejected": after_rejected_rows,
            "final_rows": final_rows,
            "proposal_history": history,
            "ledger_refs": {
                "admitted": admitted_ref,
                "rejected": rejected_ref,
            },
            "edges": [
                {
                    "case": "history_zero",
                    "before_rows": zero_before,
                    "history": zero_history,
                    "after_rows": zero_after,
                },
                {
                    "case": "invalid_admitted_sufficiency",
                    "expected": CALYX_ASSAY_INVALID_METRIC,
                    "result_code": invalid_error,
                    "before_rows": invalid_before,
                    "after_rows": invalid_after,
                },
                {
                    "case": "mixed_admitted_rejected_order",
                    "expected_actions": ["LensAdmitted", "LensRejected"],
                    "actual_actions": final_rows
                        .iter()
                        .map(|row| row["payload_json"]["action"].clone())
                        .collect::<Vec<_>>(),
                },
            ],
        }),
    );

    let physical_path = root.join("physical-files.txt");
    write_physical_size_list(&physical_path, &vault_dir);
    write_manifest(&root, &[readback_path, physical_path]);
}

fn admitted_proposal() -> calyx_anneal::ProposalOutcome {
    let clock = FixedClock::new(FSV_TS);
    let anchor = anchor();
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(ChangeId(422_001));
    let assay = FixtureAssay::new([0.20, 0.80], 1.00);
    let profiler = StaticProfiler::new(0.12);
    let nmi = StaticNmi::new(0.10);
    let mut hot_add = TestHotAdder::succeed();
    let corpus = corpus();
    ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .expect("admitted proposal")
}

fn rejected_proposal() -> calyx_anneal::ProposalOutcome {
    let clock = FixedClock::new(FSV_TS + 1);
    let anchor = anchor();
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(ChangeId(422_002));
    let assay = FixtureAssay::new([0.20], 1.00);
    let profiler = StaticProfiler::new(0.02);
    let nmi = StaticNmi::new(0.10);
    let mut hot_add = TestHotAdder::succeed();
    let corpus = corpus();
    ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .expect("rejected proposal")
}

fn invalid_admitted_record() -> LensAdmittedEntry {
    LensAdmittedEntry {
        candidate_desc: "invalid admitted edge".to_string(),
        bits_gain: 0.12,
        max_corr: 0.10,
        sufficiency_before: 0.80,
        sufficiency_after: 0.20,
        change_id: ChangeId(422_099),
        ts: FSV_TS + 99,
    }
}

fn open_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        b"issue422-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn open_anneal_ledger(
    vault: &AsterVault,
) -> AnnealLedger<AsterAnnealLedgerStore<'_, calyx_core::SystemClock>, FixedClock> {
    let store = AsterAnnealLedgerStore::new(vault);
    let appender = LedgerAppender::open(store, FixedClock::new(FSV_TS)).unwrap();
    AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-fsv-issue422".to_string()),
    )
    .unwrap()
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
