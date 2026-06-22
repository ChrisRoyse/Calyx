use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    AnchorGap, AsterOperatorProposalStorage, CALYX_ANNEAL_OPERATOR_NO_GAIN,
    CALYX_ASSAY_INVALID_METRIC, DeficitMap, OperatorTerminalState, ProposeOperator,
    ProposeOperatorRequest, decode_operator_proposal,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{FixedClock, Modality};
use serde_json::{Value, json};

#[allow(dead_code)]
#[path = "support/fsv_bad_change.rs"]
mod support;

const TEST_TS: u64 = 1_785_500_582;

#[test]
#[ignore = "requires CALYX_ISSUE582_FSV_ROOT in a manual verification run"]
fn issue582_operator_synth_fsv_manual() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE582_FSV_ROOT").expect("set CALYX_ISSUE582_FSV_ROOT"));
    support::reset_dir(&root);
    let (vault_dir, vault) = support::open_durable_vault(&root, "vault");
    let before = snapshot(&vault, &vault_dir);

    let online_promoted = run_promote(
        &vault,
        &vault_dir,
        TEST_TS,
        online_deficit(),
        0.20,
        None,
        None,
    );
    vault.flush().unwrap();
    let after_online = snapshot(&vault, &vault_dir);

    let kernel_promoted = run_promote(
        &vault,
        &vault_dir,
        TEST_TS + 1,
        kernel_deficit(),
        0.05,
        Some(0.40),
        Some(0.72),
    );
    vault.flush().unwrap();
    let after_kernel = snapshot(&vault, &vault_dir);

    let refit_before = snapshot(&vault, &vault_dir);
    let refit_closed = run_promote(
        &vault,
        &vault_dir,
        TEST_TS + 2,
        online_deficit(),
        0.80,
        None,
        None,
    );
    let refit_after = snapshot(&vault, &vault_dir);

    let no_gain_before = snapshot(&vault, &vault_dir);
    let no_gain_error = run_no_gain(&vault, &vault_dir);
    let no_gain_after = snapshot(&vault, &vault_dir);

    let rollback_before = snapshot(&vault, &vault_dir);
    let rollback = run_rollback(&vault, &vault_dir);
    vault.flush().unwrap();
    let rollback_after = snapshot(&vault, &vault_dir);

    let invalid_before = snapshot(&vault, &vault_dir);
    let invalid_error = run_invalid_metric(&vault, &vault_dir);
    let invalid_after = snapshot(&vault, &vault_dir);

    assert_eq!(before["operator_rows"].as_array().unwrap().len(), 0);
    assert_eq!(after_online["operator_rows"].as_array().unwrap().len(), 1);
    assert_eq!(after_kernel["operator_rows"].as_array().unwrap().len(), 2);
    assert_eq!(refit_before["operator_rows"], refit_after["operator_rows"]);
    assert_eq!(
        no_gain_before["operator_rows"],
        no_gain_after["operator_rows"]
    );
    assert_eq!(
        invalid_before["operator_rows"],
        invalid_after["operator_rows"]
    );
    assert_eq!(no_gain_error, CALYX_ANNEAL_OPERATOR_NO_GAIN);
    assert_eq!(invalid_error, CALYX_ASSAY_INVALID_METRIC);
    assert_eq!(rollback_after["operator_rows"].as_array().unwrap().len(), 3);
    assert!(has_ledger_action(&after_online, "operator_promoted"));
    assert!(has_ledger_action(&after_kernel, "operator_promoted"));
    assert!(has_ledger_action(&rollback_after, "operator_reverted"));
    assert!(matches!(
        rollback.terminal_state,
        OperatorTerminalState::RolledBack { .. }
    ));

    let artifact = json!({
        "issue": 582,
        "source_of_truth": {
            "operator_cf": "vault/cf/anneal_operators",
            "ledger_cf": "vault/cf/ledger",
            "rollback_cf": "vault/cf/anneal_rollback",
            "wal": "vault/wal"
        },
        "trigger": "synthetic PH47 deficit where refit_delta_j is lower than total_bits_deficit",
        "hand_expected": {
            "online_shadow_delta_j": 0.60,
            "kernel_shadow_delta_j": 0.32,
            "refit_closed_writes_rows": false,
            "no_gain_error": CALYX_ANNEAL_OPERATOR_NO_GAIN,
            "invalid_metric_error": CALYX_ASSAY_INVALID_METRIC
        },
        "before": before,
        "happy_online_promoted": online_promoted,
        "after_online": after_online,
        "happy_kernel_promoted": kernel_promoted,
        "after_kernel": after_kernel,
        "edges": {
            "refit_closed": {
                "outcome": refit_closed,
                "before": refit_before,
                "after": refit_after
            },
            "no_gain": {
                "error": no_gain_error,
                "before": no_gain_before,
                "after": no_gain_after
            },
            "rollback": {
                "outcome": rollback,
                "before": rollback_before,
                "after": rollback_after
            },
            "invalid_metric": {
                "error": invalid_error,
                "before": invalid_before,
                "after": invalid_after
            }
        }
    });
    support::write_json(
        &root.join("issue582-operator-synth-fsv-artifact.json"),
        &artifact,
    );
}

fn run_promote(
    vault: &AsterVault,
    vault_dir: &Path,
    ts: u64,
    deficit: DeficitMap,
    refit_delta_j: f64,
    kernel_recall_before: Option<f64>,
    kernel_recall_after: Option<f64>,
) -> calyx_anneal::OperatorProposalOutcome {
    let clock = FixedClock::new(ts);
    let storage = AsterOperatorProposalStorage::new(vault);
    let mut substrate = support::durable_substrate(&clock, vault, vault_dir);
    ProposeOperator::new(&clock)
        .propose_operator(ProposeOperatorRequest {
            deficit: &deficit,
            refit_delta_j,
            storage: &storage,
            gate: &mut substrate,
            kernel_recall_before,
            kernel_recall_after,
        })
        .unwrap()
}

fn run_rollback(vault: &AsterVault, vault_dir: &Path) -> calyx_anneal::OperatorProposalOutcome {
    let clock = FixedClock::new(TEST_TS + 3);
    let storage = AsterOperatorProposalStorage::new(vault);
    let mut substrate = support::durable_substrate_with_budget(
        &clock,
        vault,
        vault_dir,
        support::budget_config(0.0),
    );
    ProposeOperator::new(&clock)
        .propose_operator(ProposeOperatorRequest {
            deficit: &kernel_deficit(),
            refit_delta_j: 0.05,
            storage: &storage,
            gate: &mut substrate,
            kernel_recall_before: Some(0.50),
            kernel_recall_after: Some(0.75),
        })
        .unwrap()
}

fn run_no_gain(vault: &AsterVault, vault_dir: &Path) -> &'static str {
    let clock = FixedClock::new(TEST_TS + 4);
    let storage = AsterOperatorProposalStorage::new(vault);
    let mut substrate = support::durable_substrate(&clock, vault, vault_dir);
    ProposeOperator::new(&clock)
        .propose_operator(ProposeOperatorRequest {
            deficit: &kernel_deficit(),
            refit_delta_j: 0.05,
            storage: &storage,
            gate: &mut substrate,
            kernel_recall_before: Some(0.42),
            kernel_recall_after: Some(0.42),
        })
        .unwrap_err()
        .code
}

fn run_invalid_metric(vault: &AsterVault, vault_dir: &Path) -> &'static str {
    let clock = FixedClock::new(TEST_TS + 5);
    let storage = AsterOperatorProposalStorage::new(vault);
    let mut substrate = support::durable_substrate(&clock, vault, vault_dir);
    ProposeOperator::new(&clock)
        .propose_operator(ProposeOperatorRequest {
            deficit: &invalid_deficit(),
            refit_delta_j: 0.0,
            storage: &storage,
            gate: &mut substrate,
            kernel_recall_before: None,
            kernel_recall_after: None,
        })
        .unwrap_err()
        .code
}

fn snapshot(vault: &AsterVault, vault_dir: &Path) -> Value {
    json!({
        "operator_rows": operator_rows(vault),
        "ledger_rows": support::read_ledger_rows(vault),
        "rollback_rows": support::read_rollback_rows(vault),
        "operator_cf_files": list_files(&vault_dir.join("cf").join("anneal_operators")),
        "ledger_cf_files": list_files(&vault_dir.join("cf").join("ledger")),
        "wal_files": list_files(&vault_dir.join("wal")),
    })
}

fn operator_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealOperators)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            json!({
                "key_hex": hex(&key),
                "value_hex": hex(&value),
                "record": decode_operator_proposal(&value).unwrap(),
            })
        })
        .collect()
}

fn has_ledger_action(snapshot: &Value, action: &str) -> bool {
    snapshot["ledger_rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| {
            row["payload_json"]["action"]
                .as_str()
                .is_some_and(|observed| observed == action)
        })
}

fn online_deficit() -> DeficitMap {
    deficit(
        "oracle_prediction_quality",
        1.0,
        0.20,
        vec![Modality::Structured],
    )
}

fn kernel_deficit() -> DeficitMap {
    deficit("kernel_recall_window", 1.0, 0.35, Vec::new())
}

fn invalid_deficit() -> DeficitMap {
    deficit("oracle_prediction_quality", f64::NAN, 0.0, Vec::new())
}

fn deficit(
    anchor: &str,
    entropy_h: f64,
    mutual_info_i: f64,
    modalities: Vec<Modality>,
) -> DeficitMap {
    let gap = (entropy_h - mutual_info_i).max(0.0);
    DeficitMap {
        computed_at: TEST_TS,
        top_gaps: vec![AnchorGap {
            anchor_class: anchor.to_string(),
            entropy_h,
            mutual_info_i,
            gap,
        }],
        underrepresented_modalities: modalities,
        total_bits_deficit: gap,
    }
}

fn list_files(dir: &Path) -> Vec<String> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut out = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path().display().to_string())
        .collect::<Vec<_>>();
    out.sort();
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
