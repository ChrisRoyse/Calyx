use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    AdmissionRecord, AnnealLedger, AsterAnnealLedgerStore, CandidateLens, ChangeId, GateOutcome,
    ProposalTerminalState, ProposeLens, ProposeLensRequest, RegistryHotAdder, RejectReason,
    ShadowRevertReason, record_proposal_outcome,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{FixedClock, Input, Modality};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use calyx_registry::Registry;
use serde_json::{Value, json};

#[path = "support/propose_lens.rs"]
#[allow(dead_code)]
mod support;
use support::*;

mod fsv_support;
use fsv_support::{
    ManifestPathStyle, reset_dir, vault_id, write_json, write_physical_size_list,
    write_tree_manifest,
};

const FSV_TS: u64 = 1_785_500_791;

#[test]
fn issue791_conversion_flywheel_fsv() {
    let root = fsv_root();
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let artifact_dir = root.join("factory-artifacts");
    let vault = open_vault(&vault_dir);
    let before_rows = ledger_rows(&vault);
    assert!(before_rows.is_empty());

    let mut ledger = open_ledger(&vault);
    let (admitted, panel_after, artifact_readback, probe_readback) =
        admitted_protein_flywheel(&artifact_dir);
    assert_eq!(admitted.terminal_state, ProposalTerminalState::Admitted);
    let admitted_ref =
        record_proposal_outcome(&admitted, &mut ledger, FSV_TS, 1.20).expect("record admitted");
    vault.flush().expect("flush admitted row");
    let after_admitted = ledger_rows(&vault);
    assert_eq!(after_admitted.len(), 1);

    let duplicate = duplicate_rejection();
    let duplicate_ref = record_proposal_outcome(&duplicate, &mut ledger, FSV_TS + 1, 1.20)
        .expect("record duplicate rejection");
    vault.flush().expect("flush duplicate row");
    let after_duplicate = ledger_rows(&vault);
    assert_eq!(after_duplicate.len(), 2);

    let zero_before = ledger_rows(&vault);
    let zero = zero_deficit();
    assert_eq!(zero.terminal_state, ProposalTerminalState::NoDeficit);
    let zero_ref =
        record_proposal_outcome(&zero, &mut ledger, FSV_TS + 2, 0.0).expect("record zero edge");
    vault.flush().expect("flush zero edge");
    let zero_after = ledger_rows(&vault);
    assert_eq!(zero_ref, None);
    assert_eq!(zero_before, zero_after);

    let budget_before = ledger_rows(&vault);
    let budget = budget_deferred();
    assert!(matches!(
        budget.terminal_state,
        ProposalTerminalState::SubstrateReverted {
            reason: ShadowRevertReason::BudgetExhausted
        }
    ));
    let budget_ref = record_proposal_outcome(&budget, &mut ledger, FSV_TS + 3, 1.20)
        .expect("record budget edge");
    vault.flush().expect("flush budget edge");
    let budget_after = ledger_rows(&vault);
    assert!(budget_ref.is_some());
    assert_eq!(budget_after.len(), budget_before.len() + 1);

    let history = calyx_anneal::proposal_history_with_refs(&ledger, 5).expect("history");
    assert_eq!(history.len(), 3);
    assert!(matches!(
        history[0].record,
        AdmissionRecord::LensAdmitted(_)
    ));
    assert!(matches!(
        history[1].record,
        AdmissionRecord::LensRejected(_)
    ));
    assert!(matches!(
        &history[2].record,
        AdmissionRecord::LensRejected(entry)
            if matches!(
                &entry.reason,
                RejectReason::SubstrateReverted { shadow_reason }
                    if *shadow_reason == ShadowRevertReason::BudgetExhausted
            )
    ));

    let summary = json!({
        "source_of_truth": "Aster ledger CF rows under vault/cf/ledger plus commissioned artifact bytes under factory-artifacts",
        "vault": vault_dir.display().to_string(),
        "trigger": "ProposeLens over protein deficit with RegistryHotAdder conversion target",
        "before_rows": before_rows,
        "after_admitted_rows": after_admitted,
        "after_duplicate_rows": after_duplicate,
        "ledger_refs": {
            "admitted": admitted_ref,
            "duplicate": duplicate_ref,
            "budget_deferred": budget_ref,
        },
        "proposal_history": history,
        "panel_after": panel_after,
        "artifact_readback": artifact_readback,
        "probe_readback": probe_readback,
        "edges": {
            "zero_deficit": {
                "terminal_state": zero.terminal_state,
                "ledger_ref": zero_ref,
                "before_rows": zero_before,
                "after_rows": zero_after,
            },
            "duplicate_gate": {
                "terminal_state": duplicate.terminal_state,
                "gate_outcome": duplicate.gate_outcome,
            },
            "budget_deferred": {
                "terminal_state": budget.terminal_state,
                "change_id": budget.change_id,
                "ledger_ref": budget_ref,
                "before_rows": budget_before,
                "after_rows": budget_after,
            }
        }
    });
    write_json(&root.join("summary.json"), &summary);
    write_physical_size_list(&root.join("physical-files.txt"), &root);
    write_tree_manifest(&root, ManifestPathStyle::Display);
    println!("ISSUE791_FSV_ROOT={}", root.display());
}

fn admitted_protein_flywheel(
    artifact_dir: &Path,
) -> (calyx_anneal::ProposalOutcome, Value, Value, Value) {
    let clock = FixedClock::new(FSV_TS);
    let mut controller = controller();
    let panel_before = serde_json::to_value(controller.panel()).unwrap();
    let mut substrate = TestSubstrate::promote(ChangeId(791_100));
    let assay =
        FixtureAssay::new([0.20, 0.86], 1.40).with_expected_modalities(vec![Modality::Protein]);
    let profiler = StaticProfiler::new(0.14);
    let nmi = StaticNmi::new(0.30);
    let mut registry = Registry::new();
    let anchor = anchor();
    let corpus = corpus();
    let outcome = {
        let mut hot_add = RegistryHotAdder::with_artifact_dir(&mut registry, artifact_dir);
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
            .expect("protein proposal")
    };
    let target = match outcome.candidate.as_ref().expect("candidate") {
        CandidateLens::Commission { spec } => spec.suggested_targets[0].clone(),
        other => panic!("expected commission target, got {other:?}"),
    };
    assert_eq!(target.hf_id, "facebook/esm2_t6_8M_UR50D");
    assert_eq!(target.modality, Modality::Protein);
    assert_eq!(target.axis, "protein_sequence");
    let lens_id = outcome.hot_add.as_ref().expect("hot add").lens_id;
    let measured = registry
        .measure(
            lens_id,
            &Input::new(Modality::Protein, b"MEEPQSDPSV".to_vec()),
        )
        .expect("measure commissioned protein lens");
    let artifact = artifact_readback(artifact_dir);
    let panel_after = json!({
        "before": panel_before,
        "after": controller.panel(),
        "new_slot": controller.panel().slots.last(),
        "target": target,
        "substrate_proposed": substrate.proposed,
        "substrate_rolled_back": substrate.rolled_back,
    });
    let probe = json!({
        "lens_id": lens_id,
        "input_modality": "protein",
        "vector": measured,
    });
    (outcome, panel_after, artifact, probe)
}

fn duplicate_rejection() -> calyx_anneal::ProposalOutcome {
    let outcome = proposal_with(0.20, 1.40, 0.14, 0.75, ChangeId(791_101));
    assert_eq!(outcome.terminal_state, ProposalTerminalState::GateRejected);
    assert!(matches!(
        outcome.gate_outcome,
        Some(GateOutcome::Rejected {
            reason: RejectReason::TooCorrelated { .. }
        })
    ));
    outcome
}

fn zero_deficit() -> calyx_anneal::ProposalOutcome {
    proposal_with(0.96, 1.00, 0.14, 0.30, ChangeId(791_102))
}

fn budget_deferred() -> calyx_anneal::ProposalOutcome {
    let clock = FixedClock::new(FSV_TS + 3);
    let mut controller = controller();
    let mut substrate =
        TestSubstrate::revert(ChangeId(791_103), ShadowRevertReason::BudgetExhausted);
    let assay = FixtureAssay::new([0.20], 1.40).with_expected_modalities(vec![Modality::Protein]);
    let profiler = StaticProfiler::new(0.14);
    let nmi = StaticNmi::new(0.30);
    let mut hot_add = TestHotAdder::succeed();
    let anchor = anchor();
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
        .expect("budget edge")
}

fn proposal_with(
    sufficiency: f64,
    entropy: f64,
    bits: f32,
    corr: f64,
    change_id: ChangeId,
) -> calyx_anneal::ProposalOutcome {
    let clock = FixedClock::new(FSV_TS + change_id.0 % 10);
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(change_id);
    let assay =
        FixtureAssay::new([sufficiency], entropy).with_expected_modalities(vec![Modality::Protein]);
    let profiler = StaticProfiler::new(bits);
    let nmi = StaticNmi::new(corr);
    let mut hot_add = TestHotAdder::succeed();
    let anchor = anchor();
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
        .expect("proposal edge")
}

fn artifact_readback(root: &Path) -> Value {
    let mut artifacts = Vec::new();
    collect_artifacts(root, &mut artifacts);
    artifacts.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    assert_eq!(artifacts.len(), 1);
    json!({ "artifacts": artifacts })
}

fn collect_artifacts(dir: &Path, out: &mut Vec<Value>) {
    for entry in fs::read_dir(dir).expect("read artifact dir") {
        let path = entry.expect("artifact entry").path();
        if path.is_dir() {
            collect_artifacts(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            let bytes = fs::read(&path).expect("read artifact");
            out.push(json!({
                "path": path.display().to_string(),
                "len": bytes.len(),
                "blake3": blake3::hash(&bytes).to_hex().to_string(),
                "json": serde_json::from_slice::<Value>(&bytes).unwrap(),
            }));
        }
    }
}

fn open_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        b"issue791-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn open_ledger(
    vault: &AsterVault,
) -> AnnealLedger<AsterAnnealLedgerStore<'_, calyx_core::SystemClock>, FixedClock> {
    let store = AsterAnnealLedgerStore::new(vault);
    let appender = LedgerAppender::open(store, FixedClock::new(FSV_TS)).unwrap();
    AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-fsv-issue791".to_string()),
    )
    .unwrap()
}

fn ledger_rows(vault: &AsterVault) -> Vec<Value> {
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
                "entry_hash": hex(&entry.entry_hash),
                "payload_json": serde_json::from_slice::<Value>(&entry.payload).unwrap(),
            })
        })
        .collect()
}

fn fsv_root() -> PathBuf {
    env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::temp_dir().join(format!("issue791-fsv-{}", std::process::id())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
